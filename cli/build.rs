use sha2::{Digest, Sha256};
use std::{
    env,
    fmt::Write,
    fs,
    io::{self, Write as IoWrite},
    path::{Path, PathBuf},
    process::Command,
};

const TARGETS: &[TargetSpec] = &[
    TargetSpec {
        const_name: "OPENCODE_EMBEDDED_ASSETS",
        relative_root: "assets/generated/config/opencode",
    },
    TargetSpec {
        const_name: "CLAUDE_EMBEDDED_ASSETS",
        relative_root: "assets/generated/config/claude",
    },
    TargetSpec {
        const_name: "HOOK_EMBEDDED_ASSETS",
        relative_root: "assets/hooks",
    },
];

struct TargetSpec {
    const_name: &'static str,
    relative_root: &'static str,
}

fn main() {
    if let Err(error) = generate_embedded_asset_manifest() {
        panic!("failed to generate setup embedded asset manifest: {error}");
    }

    emit_git_commit();
}

fn emit_git_commit() {
    println!("cargo:rerun-if-env-changed=SCE_GIT_COMMIT");

    if let Ok(commit) = env::var("SCE_GIT_COMMIT") {
        let commit = commit.trim();
        if !commit.is_empty() {
            println!("cargo:rustc-env=SCE_GIT_COMMIT={commit}");
            return;
        }
    }

    let manifest_dir = match env::var("CARGO_MANIFEST_DIR") {
        Ok(value) => PathBuf::from(value),
        Err(_) => return,
    };

    let repository_root = match manifest_dir.parent() {
        Some(path) => path.to_path_buf(),
        None => return,
    };

    let git_dir = repository_root.join(".git");
    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    println!(
        "cargo:rerun-if-changed={}",
        git_dir.join("packed-refs").display()
    );

    let output = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(&repository_root)
        .output();

    let Ok(output) = output else {
        return;
    };

    if !output.status.success() {
        return;
    }

    let Ok(commit) = String::from_utf8(output.stdout) else {
        return;
    };

    let commit = commit.trim();
    if !commit.is_empty() {
        println!("cargo:rustc-env=SCE_GIT_COMMIT={commit}");
    }
}

fn generate_embedded_asset_manifest() -> io::Result<()> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|e| invalid_data(&e))?);
    let out_dir = PathBuf::from(env::var("OUT_DIR").map_err(|e| invalid_data(&e))?);
    let destination_path = out_dir.join("setup_embedded_assets.rs");

    let mut output = String::new();

    for target in TARGETS {
        let source_root = manifest_dir.join(target.relative_root);
        println!("cargo:rerun-if-changed={}", source_root.display());

        let mut files = Vec::new();
        collect_files(&source_root, &source_root, &mut files)?;
        files.sort_unstable_by(|a, b| a.relative_path.cmp(&b.relative_path));

        writeln!(
            output,
            "pub static {}: &[EmbeddedAsset] = &[",
            target.const_name
        )
        .expect("writing to String buffer should never fail");

        for file in &files {
            println!("cargo:rerun-if-changed={}", file.absolute_path.display());
            let bytes = fs::read(&file.absolute_path)?;
            let sha256 = compute_sha256(&bytes);
            writeln!(
                output,
                "    EmbeddedAsset {{ relative_path: \"{}\", bytes: {}, sha256: {} }},",
                escape_for_rust_string(&file.relative_path),
                format_byte_literal("&[", &bytes),
                format_byte_literal("[", &sha256),
            )
            .expect("writing to String buffer should never fail");
        }

        output.push_str("];\n\n");
    }

    let mut output_file = fs::File::create(destination_path)?;
    output_file.write_all(output.as_bytes())
}

#[derive(Debug)]
struct SourceFile {
    absolute_path: PathBuf,
    relative_path: String,
}

fn collect_files(
    base_root: &Path,
    current_dir: &Path,
    output: &mut Vec<SourceFile>,
) -> io::Result<()> {
    for entry in fs::read_dir(current_dir)? {
        let entry = entry?;
        let path = entry.path();

        if entry.file_type()?.is_dir() {
            collect_files(base_root, &path, output)?;
            continue;
        }

        let relative_path = path
            .strip_prefix(base_root)
            .map_err(|_| invalid_data(&"failed to strip source root from file path"))?;

        let relative_path = normalize_relative_path(relative_path)?;

        output.push(SourceFile {
            absolute_path: path,
            relative_path,
        });
    }

    Ok(())
}

fn normalize_relative_path(path: &Path) -> io::Result<String> {
    let normalized = path
        .to_str()
        .ok_or_else(|| invalid_data(&"non-UTF-8 config paths are not supported"))?
        .replace('\\', "/");

    if normalized.is_empty() {
        return Err(invalid_data(&"relative path cannot be empty"));
    }

    if normalized.starts_with('/') {
        return Err(invalid_data(&"relative path must not start with '/'"));
    }

    Ok(normalized)
}

fn escape_for_rust_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn compute_sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    digest.into()
}

fn format_byte_literal(prefix: &str, bytes: &[u8]) -> String {
    format!(
        "{prefix}{}]",
        bytes
            .iter()
            .map(|byte| format!("0x{byte:02x}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn invalid_data<E: ToString>(error: &E) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
