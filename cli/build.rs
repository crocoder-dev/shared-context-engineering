use sha2::{Digest, Sha256};
use std::{
    env,
    fmt::Write,
    fs,
    io::{self, Write as IoWrite},
    path::{Path, PathBuf},
    process::Command,
};

const PKL_OUTPUT_DIR: &str = "pkl-generated";
const STATIC_OUTPUT_DIR: &str = "static";
const MIGRATIONS_ROOT: &str = "migrations";
const PACKAGE_FALLBACK_DIR: &str = "package-fallback";
const PACKAGE_FALLBACK_INVENTORY: &str = "SHA256SUMS";

const TARGETS: &[TargetSpec] = &[
    TargetSpec {
        const_name: "OPENCODE_EMBEDDED_ASSETS",
        generated_root: "config/.opencode",
    },
    TargetSpec {
        const_name: "CLAUDE_EMBEDDED_ASSETS",
        generated_root: "config/.claude",
    },
    TargetSpec {
        const_name: "PI_EMBEDDED_ASSETS",
        generated_root: "config/.pi",
    },
    TargetSpec {
        const_name: "HOOK_EMBEDDED_ASSETS",
        generated_root: "static/hooks",
    },
];

struct TargetSpec {
    const_name: &'static str,
    generated_root: &'static str,
}

fn main() {
    if let Err(error) = prepare_build_artifacts() {
        panic!("failed to prepare embedded CLI build artifacts: {error}");
    }

    emit_git_commit();
    emit_repo_version();
}

fn prepare_build_artifacts() -> io::Result<()> {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|e| invalid_data(&e))?);
    let repository_root = manifest_dir
        .parent()
        .ok_or_else(|| invalid_data(&"CLI manifest directory must have a repository parent"))?;
    let out_dir = PathBuf::from(env::var("OUT_DIR").map_err(|e| invalid_data(&e))?);

    if repository_sources_available(repository_root) {
        generate_pkl_outputs(repository_root, &out_dir)?;
        stage_static_inputs(repository_root, &manifest_dir, &out_dir)?;
    } else {
        stage_packaged_fallback(&manifest_dir, &out_dir)?;
    }
    validate_staged_artifacts(&out_dir)?;
    generate_embedded_asset_manifest(&out_dir)?;
    generate_migration_manifest(&out_dir)
}

fn repository_sources_available(repository_root: &Path) -> bool {
    repository_root.join("config/pkl/generate.pkl").is_file()
        && repository_root.join("config/lib").is_dir()
}

fn generate_pkl_outputs(repository_root: &Path, out_dir: &Path) -> io::Result<()> {
    let pkl_root = repository_root.join("config/pkl");
    let config_lib_root = repository_root.join("config/lib");
    emit_rerun_tree(&pkl_root)?;
    emit_rerun_tree(&config_lib_root)?;

    let output_root = out_dir.join(PKL_OUTPUT_DIR);
    remove_path_if_exists(&output_root)?;
    fs::create_dir_all(&output_root)?;

    let generator = pkl_root.join("generate.pkl");
    let status = Command::new("pkl")
        .arg("eval")
        .arg("-m")
        .arg(&output_root)
        .arg(&generator)
        .current_dir(repository_root)
        .status()
        .map_err(|error| {
            invalid_data(&format!(
                "could not run Pkl generator '{}': {error}. Run Cargo from the Nix dev shell",
                generator.display()
            ))
        })?;

    if !status.success() {
        return Err(invalid_data(&format!(
            "Pkl generator '{}' exited with {status}",
            generator.display()
        )));
    }

    for target in TARGETS.iter().take(3) {
        let expected_root = output_root.join(target.generated_root);
        if !expected_root.is_dir() {
            return Err(invalid_data(&format!(
                "Pkl generator did not emit required target directory '{}'",
                expected_root.display()
            )));
        }
    }

    Ok(())
}

fn stage_packaged_fallback(manifest_dir: &Path, out_dir: &Path) -> io::Result<()> {
    let fallback_root = manifest_dir.join(PACKAGE_FALLBACK_DIR);
    println!("cargo:rerun-if-changed={}", fallback_root.display());
    validate_fallback_inventory(&fallback_root)?;

    for directory in [PKL_OUTPUT_DIR, STATIC_OUTPUT_DIR] {
        let source = fallback_root.join(directory);
        let destination = out_dir.join(directory);
        remove_path_if_exists(&destination)?;
        copy_tree(&source, &destination)?;
    }

    Ok(())
}

fn validate_fallback_inventory(fallback_root: &Path) -> io::Result<()> {
    let inventory_path = fallback_root.join(PACKAGE_FALLBACK_INVENTORY);
    let expected = fs::read_to_string(&inventory_path).map_err(|error| {
        invalid_data(&format!(
            "packaged fallback inventory '{}' is unavailable: {error}. The crate package must be prepared with scripts/prepare-cli-generated-assets.sh",
            inventory_path.display()
        ))
    })?;

    let mut files = Vec::new();
    for directory in [PKL_OUTPUT_DIR, STATIC_OUTPUT_DIR] {
        let root = fallback_root.join(directory);
        if !root.is_dir() {
            return Err(invalid_data(&format!(
                "packaged fallback is missing required directory '{}'. Recreate the crate package with scripts/prepare-cli-generated-assets.sh",
                root.display()
            )));
        }
        collect_files(fallback_root, &root, &mut files)?;
    }
    files.sort_unstable_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let mut actual = String::new();
    for file in files {
        let digest = compute_sha256(&fs::read(&file.absolute_path)?);
        writeln!(actual, "{}  {}", format_hex(&digest), file.relative_path)
            .expect("writing to String buffer should never fail");
    }

    if actual != expected {
        return Err(invalid_data(&format!(
            "packaged fallback inventory '{}' does not match its payload. Recreate the crate package with scripts/prepare-cli-generated-assets.sh",
            inventory_path.display()
        )));
    }

    Ok(())
}

fn validate_staged_artifacts(out_dir: &Path) -> io::Result<()> {
    for target in TARGETS.iter().take(3) {
        let expected_root = out_dir.join(PKL_OUTPUT_DIR).join(target.generated_root);
        if !expected_root.is_dir() {
            return Err(invalid_data(&format!(
                "embedded artifact preparation did not provide required target directory '{}'",
                expected_root.display()
            )));
        }
    }

    for relative_path in [
        "static/hooks",
        "static/migrations",
        "static/schema/agent-trace.schema.json",
        "pkl-generated/config/schema/sce-config.schema.json",
    ] {
        if !out_dir.join(relative_path).exists() {
            return Err(invalid_data(&format!(
                "embedded artifact preparation did not provide required path '{}'",
                out_dir.join(relative_path).display()
            )));
        }
    }

    Ok(())
}

fn stage_static_inputs(
    repository_root: &Path,
    manifest_dir: &Path,
    out_dir: &Path,
) -> io::Result<()> {
    let static_root = out_dir.join(STATIC_OUTPUT_DIR);
    remove_path_if_exists(&static_root)?;

    copy_tree(
        &manifest_dir.join("assets/hooks"),
        &static_root.join("hooks"),
    )?;
    copy_tree(
        &manifest_dir.join(MIGRATIONS_ROOT),
        &static_root.join(MIGRATIONS_ROOT),
    )?;
    copy_file(
        &repository_root.join("config/schema/agent-trace.schema.json"),
        &static_root.join("schema/agent-trace.schema.json"),
    )
}

fn copy_tree(source_root: &Path, destination_root: &Path) -> io::Result<()> {
    println!("cargo:rerun-if-changed={}", source_root.display());

    let mut files = Vec::new();
    collect_files(source_root, source_root, &mut files)?;
    files.sort_unstable_by(|left, right| left.relative_path.cmp(&right.relative_path));

    for file in files {
        println!("cargo:rerun-if-changed={}", file.absolute_path.display());
        copy_file(
            &file.absolute_path,
            &destination_root.join(&file.relative_path),
        )?;
    }

    Ok(())
}

fn copy_file(source: &Path, destination: &Path) -> io::Result<()> {
    println!("cargo:rerun-if-changed={}", source.display());
    let parent = destination
        .parent()
        .ok_or_else(|| invalid_data(&"staged file must have a parent directory"))?;
    fs::create_dir_all(parent)?;
    fs::copy(source, destination)?;
    Ok(())
}

fn emit_rerun_tree(root: &Path) -> io::Result<()> {
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_unstable_by(|left, right| left.relative_path.cmp(&right.relative_path));
    for file in files {
        println!("cargo:rerun-if-changed={}", file.absolute_path.display());
    }
    Ok(())
}

fn generate_embedded_asset_manifest(out_dir: &Path) -> io::Result<()> {
    let destination_path = out_dir.join("setup_embedded_assets.rs");
    let mut output = String::new();

    for target in TARGETS {
        let source_root = if target.generated_root.starts_with("static/") {
            out_dir.join(target.generated_root)
        } else {
            out_dir.join(PKL_OUTPUT_DIR).join(target.generated_root)
        };
        let include_root = source_root
            .strip_prefix(out_dir)
            .map_err(|_| invalid_data(&"embedded asset root must be inside OUT_DIR"))?;
        let include_root = normalize_relative_path(include_root)?;

        let mut files = Vec::new();
        collect_files(&source_root, &source_root, &mut files)?;
        files.sort_unstable_by(|left, right| left.relative_path.cmp(&right.relative_path));

        writeln!(
            output,
            "pub static {}: &[EmbeddedAsset] = &[",
            target.const_name
        )
        .expect("writing to String buffer should never fail");

        for file in &files {
            let bytes = fs::read(&file.absolute_path)?;
            let sha256 = compute_sha256(&bytes);
            let include_path = format!("{include_root}/{}", file.relative_path);
            writeln!(
                output,
                "    EmbeddedAsset {{ relative_path: \"{}\", bytes: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{}\")), sha256: {} }},",
                escape_for_rust_string(&file.relative_path),
                escape_for_rust_string(&include_path),
                format_byte_literal("[", &sha256),
            )
            .expect("writing to String buffer should never fail");
        }

        output.push_str("];\n\n");
    }

    let mut output_file = fs::File::create(destination_path)?;
    output_file.write_all(output.as_bytes())
}

fn generate_migration_manifest(out_dir: &Path) -> io::Result<()> {
    let migrations_root = out_dir.join(STATIC_OUTPUT_DIR).join(MIGRATIONS_ROOT);
    let destination_path = out_dir.join("generated_migrations.rs");
    let mut databases = collect_migration_databases(&migrations_root)?;
    databases.sort_unstable_by(|left, right| left.directory_name.cmp(&right.directory_name));

    let mut output = String::new();
    output.push_str("// @generated by build.rs; do not edit by hand.\n\n");

    for database in &databases {
        output.push_str("#[rustfmt::skip]\n");
        output.push_str("pub static ");
        output.push_str(&database.const_name);
        output.push_str(": &[(&str, &str)] = &[\n");

        for migration in &database.migrations {
            writeln!(
                output,
                "    (\"{}\", include_str!(concat!(env!(\"OUT_DIR\"), \"/static/migrations/{}/{}.sql\"))),",
                escape_for_rust_string(&migration.id),
                escape_for_rust_string(&database.directory_name),
                escape_for_rust_string(&migration.id),
            )
            .expect("writing to String buffer should never fail");
        }

        output.push_str("];\n\n");
    }

    write_if_changed(&destination_path, output.as_bytes())
}

fn emit_git_commit() {
    println!("cargo:rerun-if-env-changed=SCE_GIT_COMMIT");

    if let Ok(commit) = env::var("SCE_GIT_COMMIT") {
        let commit = commit.trim();
        if !commit.is_empty() {
            println!("cargo:rustc-env=SCE_GIT_COMMIT={commit}");
        }
    }
}

fn emit_repo_version() {
    let manifest_dir = match env::var("CARGO_MANIFEST_DIR") {
        Ok(value) => PathBuf::from(value),
        Err(_) => return,
    };
    let Some(repository_root) = manifest_dir.parent() else {
        return;
    };

    let version_path = repository_root.join(".version");
    println!("cargo:rerun-if-changed={}", version_path.display());

    let Ok(bytes) = fs::read(&version_path) else {
        return;
    };
    let version = String::from_utf8_lossy(&bytes);
    let version = version.trim();
    if !version.is_empty() {
        println!("cargo:rustc-env=SCE_VERSION={version}");
    }
}

#[derive(Debug)]
struct MigrationDatabase {
    directory_name: String,
    const_name: String,
    migrations: Vec<MigrationFile>,
}

#[derive(Debug)]
struct MigrationFile {
    id: String,
    sort_prefix: u64,
}

fn collect_migration_databases(migrations_root: &Path) -> io::Result<Vec<MigrationDatabase>> {
    let mut databases = Vec::new();

    for entry in fs::read_dir(migrations_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let directory_path = entry.path();
        let directory_name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| invalid_data(&"non-UTF-8 migration directory names are not supported"))?
            .to_owned();
        let mut migrations = collect_migration_files(&directory_path)?;
        migrations.sort_unstable_by(|left, right| {
            left.sort_prefix
                .cmp(&right.sort_prefix)
                .then_with(|| left.id.cmp(&right.id))
        });

        databases.push(MigrationDatabase {
            const_name: migration_const_name(&directory_name)?,
            directory_name,
            migrations,
        });
    }

    Ok(databases)
}

fn collect_migration_files(directory_path: &Path) -> io::Result<Vec<MigrationFile>> {
    let mut migrations = Vec::new();

    for entry in fs::read_dir(directory_path)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir()
            || path.extension().and_then(|value| value.to_str()) != Some("sql")
        {
            continue;
        }

        let id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| invalid_data(&"non-UTF-8 migration filenames are not supported"))?
            .to_owned();
        migrations.push(MigrationFile {
            sort_prefix: migration_sort_prefix(&id)?,
            id,
        });
    }

    Ok(migrations)
}

fn migration_sort_prefix(id: &str) -> io::Result<u64> {
    let prefix = id
        .split_once('_')
        .map(|(prefix, _)| prefix)
        .ok_or_else(|| invalid_data(&format!("migration filename '{id}' must contain '_'")))?;
    if prefix.is_empty() || !prefix.chars().all(|character| character.is_ascii_digit()) {
        return Err(invalid_data(&format!(
            "migration filename '{id}' must start with a numeric prefix"
        )));
    }
    prefix.parse::<u64>().map_err(|error| invalid_data(&error))
}

fn migration_const_name(directory_name: &str) -> io::Result<String> {
    let mut const_name = String::new();
    for character in directory_name.chars() {
        if character.is_ascii_alphanumeric() {
            const_name.push(character.to_ascii_uppercase());
        } else if character == '-' || character == '_' {
            const_name.push('_');
        } else {
            return Err(invalid_data(&format!(
                "migration directory '{directory_name}' contains unsupported character '{character}'"
            )));
        }
    }
    if const_name.is_empty() {
        return Err(invalid_data(&"migration directory name cannot be empty"));
    }
    const_name.push_str("_MIGRATIONS");
    Ok(const_name)
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
    if normalized.is_empty() || normalized.starts_with('/') {
        return Err(invalid_data(&"path must be a non-empty relative path"));
    }
    Ok(normalized)
}

fn remove_path_if_exists(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else if path.exists() {
        fs::remove_file(path)
    } else {
        Ok(())
    }
}

fn escape_for_rust_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn compute_sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn format_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").expect("writing to String buffer should never fail");
        output
    })
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

fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    fs::write(path, bytes)
}

fn invalid_data<E: ToString>(error: &E) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
