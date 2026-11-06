use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use murmur3::murmur3_32;

const CONTENT_HASH_PREFIX: &str = "murmur3:";
const CONTENT_HASH_SEED: u32 = 0;
const GIT_BLOB_OBJECT_TYPE: &str = "blob";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommittedFile {
    Blob(Vec<u8>),
    NotLineAddressable,
    Unavailable,
}

pub trait CommittedFileReader {
    fn read_committed_file(&self, path: &str) -> CommittedFile;
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoCommittedFiles;

#[cfg(test)]
impl CommittedFileReader for NoCommittedFiles {
    fn read_committed_file(&self, _path: &str) -> CommittedFile {
        CommittedFile::Unavailable
    }
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

    fn git_stdout(&self, args: &[&str]) -> Option<Vec<u8>> {
        let output = Command::new("git")
            .arg("--literal-pathspecs")
            .args(args)
            .current_dir(self.repository_root)
            .output()
            .ok()?;
        output.status.success().then_some(output.stdout)
    }
}

impl CommittedFileReader for GitCommittedFiles<'_> {
    fn read_committed_file(&self, path: &str) -> CommittedFile {
        let Some(listing) =
            self.git_stdout(&["ls-tree", "-z", "--full-tree", self.revision, "--", path])
        else {
            return CommittedFile::Unavailable;
        };
        let Some((object_type, object_id)) = parse_ls_tree_entry(&listing, path) else {
            return CommittedFile::Unavailable;
        };
        if object_type != GIT_BLOB_OBJECT_TYPE {
            return CommittedFile::NotLineAddressable;
        }
        self.git_stdout(&["cat-file", "blob", object_id])
            .map_or(CommittedFile::Unavailable, CommittedFile::Blob)
    }
}

fn parse_ls_tree_entry<'o>(listing: &'o [u8], path: &str) -> Option<(&'o str, &'o str)> {
    listing
        .split(|byte| *byte == 0)
        .filter_map(|entry| std::str::from_utf8(entry).ok())
        .find_map(|entry| {
            let (header, entry_path) = entry.split_once('\t')?;
            if entry_path != path {
                return None;
            }
            let mut fields = header.split(' ');
            let _mode = fields.next()?;
            Some((fields.next()?, fields.next()?))
        })
}

pub(super) struct CommittedFileCache<'r> {
    reader: &'r dyn CommittedFileReader,
    files: HashMap<String, CommittedFile>,
}

impl<'r> CommittedFileCache<'r> {
    pub(super) fn new(reader: &'r dyn CommittedFileReader) -> Self {
        Self {
            reader,
            files: HashMap::new(),
        }
    }

    pub(super) fn get(&mut self, path: &str) -> &CommittedFile {
        self.files
            .entry(path.to_owned())
            .or_insert_with(|| self.reader.read_committed_file(path))
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
