use std::collections::HashMap;
use std::fmt;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use murmur3::murmur3_32;

const CONTENT_HASH_PREFIX: &str = "murmur3:";
const CONTENT_HASH_SEED: u32 = 0;
const GIT_PROGRAM: &str = "git";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommittedFile {
    Blob(Vec<u8>),
    NotLineAddressable,
    Missing,
    #[cfg(test)]
    Unverified,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommittedFileReadError {
    TreeLookupFailed {
        path: String,
        revision: String,
        detail: String,
    },
    MalformedTreeEntry {
        path: String,
        revision: String,
        detail: String,
    },
    BlobReadFailed {
        path: String,
        revision: String,
        object_id: String,
        detail: String,
    },
}

impl fmt::Display for CommittedFileReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TreeLookupFailed {
                path,
                revision,
                detail,
            } => write!(
                f,
                "Failed to look up '{path}' in revision '{revision}' with 'git ls-tree': {detail}"
            ),
            Self::MalformedTreeEntry {
                path,
                revision,
                detail,
            } => write!(
                f,
                "Unexpected 'git ls-tree' entry for '{path}' in revision '{revision}': {detail}"
            ),
            Self::BlobReadFailed {
                path,
                revision,
                object_id,
                detail,
            } => write!(
                f,
                "Failed to read blob '{object_id}' for '{path}' in revision '{revision}' with 'git cat-file': {detail}"
            ),
        }
    }
}

impl std::error::Error for CommittedFileReadError {}

pub type CommittedFileRead = Result<CommittedFile, CommittedFileReadError>;

pub trait CommittedFileReader {
    fn read_committed_file(&self, path: &str) -> CommittedFileRead;
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UnverifiedCommittedFiles;

#[cfg(test)]
impl CommittedFileReader for UnverifiedCommittedFiles {
    fn read_committed_file(&self, _path: &str) -> CommittedFileRead {
        Ok(CommittedFile::Unverified)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GitEntryKind {
    RegularFile,
    Symlink,
    Gitlink,
    Directory,
}

impl GitEntryKind {
    pub(crate) fn classify(mode: &str, object_type: &str) -> Option<Self> {
        match (mode, object_type) {
            ("100644" | "100755", "blob") => Some(Self::RegularFile),
            ("120000", "blob") => Some(Self::Symlink),
            ("160000", "commit") => Some(Self::Gitlink),
            ("040000", "tree") => Some(Self::Directory),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GitTreeEntry {
    pub(crate) kind: GitEntryKind,
    pub(crate) object_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GitCommittedFiles<'a> {
    repository_root: &'a Path,
    revision: &'a str,
}

impl<'a> GitCommittedFiles<'a> {
    pub fn new(repository_root: &'a Path, revision: &'a str) -> Self {
        Self {
            repository_root,
            revision,
        }
    }

    fn git_stdout(&self, args: &[&str]) -> Result<Vec<u8>, String> {
        let output = Command::new(GIT_PROGRAM)
            .arg("--literal-pathspecs")
            .args(args)
            .current_dir(self.repository_root)
            .output()
            .map_err(|error| format!("could not run git: {error}"))?;
        if output.status.success() {
            return Ok(output.stdout);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if stderr.is_empty() {
            format!("git exited with {}", output.status)
        } else {
            stderr
        })
    }
}

impl CommittedFileReader for GitCommittedFiles<'_> {
    fn read_committed_file(&self, path: &str) -> CommittedFileRead {
        let listing = self
            .git_stdout(&["ls-tree", "-z", "--full-tree", self.revision, "--", path])
            .map_err(|detail| CommittedFileReadError::TreeLookupFailed {
                path: path.to_owned(),
                revision: self.revision.to_owned(),
                detail,
            })?;
        let entry = parse_ls_tree_entry(&listing, path).map_err(|detail| {
            CommittedFileReadError::MalformedTreeEntry {
                path: path.to_owned(),
                revision: self.revision.to_owned(),
                detail,
            }
        })?;
        let Some(entry) = entry else {
            return Ok(CommittedFile::Missing);
        };
        if entry.kind != GitEntryKind::RegularFile {
            return Ok(CommittedFile::NotLineAddressable);
        }
        self.git_stdout(&["cat-file", "blob", &entry.object_id])
            .map(CommittedFile::Blob)
            .map_err(|detail| CommittedFileReadError::BlobReadFailed {
                path: path.to_owned(),
                revision: self.revision.to_owned(),
                object_id: entry.object_id,
                detail,
            })
    }
}

pub(crate) fn parse_ls_tree_entry(
    listing: &[u8],
    path: &str,
) -> Result<Option<GitTreeEntry>, String> {
    let mut found = None;
    for record in listing
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| {
                format!(
                    "record without a path: {:?}",
                    String::from_utf8_lossy(record)
                )
            })?;
        let (header, entry_path) = (&record[..tab], &record[tab + 1..]);
        let header = std::str::from_utf8(header)
            .map_err(|_| format!("non-UTF-8 header: {:?}", String::from_utf8_lossy(header)))?;
        let fields: Vec<&str> = header.split(' ').collect();
        let [mode, object_type, object_id] = fields.as_slice() else {
            return Err(format!("expected '<mode> <type> <object>', got {header:?}"));
        };
        if object_id.is_empty() || !object_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("invalid object id {object_id:?}"));
        }
        let kind = GitEntryKind::classify(mode, object_type)
            .ok_or_else(|| format!("unsupported mode '{mode}' for object type '{object_type}'"))?;
        if entry_path != path.as_bytes() {
            continue;
        }
        if found.is_some() {
            return Err(String::from("more than one entry matches the path"));
        }
        found = Some(GitTreeEntry {
            kind,
            object_id: (*object_id).to_owned(),
        });
    }
    Ok(found)
}

pub(super) struct CommittedFileCache<'r> {
    reader: &'r dyn CommittedFileReader,
    files: HashMap<String, CommittedFileRead>,
}

impl<'r> CommittedFileCache<'r> {
    pub(super) fn new(reader: &'r dyn CommittedFileReader) -> Self {
        Self {
            reader,
            files: HashMap::new(),
        }
    }

    pub(super) fn get(&mut self, path: &str) -> Result<&CommittedFile, CommittedFileReadError> {
        self.files
            .entry(path.to_owned())
            .or_insert_with(|| self.reader.read_committed_file(path))
            .as_ref()
            .map_err(Clone::clone)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommittedLines<'a> {
    bytes: &'a [u8],
    bounds: Vec<usize>,
}

impl<'a> CommittedLines<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        let mut bounds = vec![0];
        bounds.extend(
            bytes
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .map(|(index, _)| index + 1),
        );
        if bounds.last() != Some(&bytes.len()) {
            bounds.push(bytes.len());
        }
        Self { bytes, bounds }
    }

    pub(crate) fn line_count(&self) -> u64 {
        (self.bounds.len() - 1) as u64
    }

    pub(crate) fn span(&self, start_line: u64, end_line: u64) -> Option<&'a [u8]> {
        if start_line == 0 || end_line < start_line || end_line > self.line_count() {
            return None;
        }
        let start = self.bounds[usize::try_from(start_line - 1).ok()?];
        let end = self.bounds[usize::try_from(end_line).ok()?];
        Some(&self.bytes[start..end])
    }

    pub(crate) fn line_matches(&self, line_number: u64, content: &str) -> bool {
        let Some(line) = self.span(line_number, line_number) else {
            return false;
        };
        let without_lf = line.strip_suffix(b"\n").unwrap_or(line);
        without_lf == content.as_bytes()
            || without_lf.strip_suffix(b"\r") == Some(content.as_bytes())
    }
}

pub(crate) fn content_hash(bytes: &[u8]) -> String {
    let hash = murmur3_32(&mut Cursor::new(bytes), CONTENT_HASH_SEED)
        .expect("murmur3 hashing from an in-memory cursor should not fail");
    format!("{CONTENT_HASH_PREFIX}{hash:08x}")
}

#[cfg(test)]
mod tests;
