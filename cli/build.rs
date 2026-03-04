use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

const TARGETS: &[TargetSpec] = &[
    TargetSpec {
        const_name: "OPENCODE_EMBEDDED_ASSETS",
        relative_root: "config/.opencode",
        include_prefix: "/../config/.opencode/",
    },
    TargetSpec {
        const_name: "CLAUDE_EMBEDDED_ASSETS",
        relative_root: "config/.claude",
        include_prefix: "/../config/.claude/",
    },
    TargetSpec {
        const_name: "HOOK_EMBEDDED_ASSETS",
        relative_root: "cli/assets/hooks",
        include_prefix: "/assets/hooks/",
    },
];

struct TargetSpec {
    const_name: &'static str,
    relative_root: &'static str,
    include_prefix: &'static str,
}

fn main() {
    if let Err(error) = generate_embedded_asset_manifest() {
        panic!("failed to generate setup embedded asset manifest: {error}");
    }
}

fn generate_embedded_asset_manifest() -> io::Result<()> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(invalid_data)?);
    let repository_root = manifest_dir
        .parent()
        .ok_or_else(|| invalid_data("CARGO_MANIFEST_DIR does not have a parent"))?
        .to_path_buf();
    let out_dir = PathBuf::from(env::var("OUT_DIR").map_err(invalid_data)?);
    let destination_path = out_dir.join("setup_embedded_assets.rs");

    let mut output = String::new();

    for target in TARGETS {
        let source_root = repository_root.join(target.relative_root);
        println!("cargo:rerun-if-changed={}", source_root.display());

        let mut files = Vec::new();
        collect_files(&source_root, &source_root, &mut files)?;
        files.sort_unstable_by(|a, b| a.relative_path.cmp(&b.relative_path));

        output.push_str(&format!(
            "pub static {}: &[EmbeddedAsset] = &[\n",
            target.const_name
        ));

        for file in &files {
            println!("cargo:rerun-if-changed={}", file.absolute_path.display());
            output.push_str(&format!(
                "    EmbeddedAsset {{ relative_path: \"{}\", bytes: include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"{}{}\")) }},\n",
                escape_for_rust_string(&file.relative_path),
                target.include_prefix,
                escape_for_rust_string(&file.relative_path),
            ));
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
            .map_err(|_| invalid_data("failed to strip source root from file path"))?;

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
        .ok_or_else(|| invalid_data("non-UTF-8 config paths are not supported"))?
        .replace('\\', "/");

    if normalized.is_empty() {
        return Err(invalid_data("relative path cannot be empty"));
    }

    if normalized.starts_with('/') {
        return Err(invalid_data("relative path must not start with '/'"));
    }

    Ok(normalized)
}

fn escape_for_rust_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn invalid_data<E: ToString>(error: E) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
