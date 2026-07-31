use sha2::{Digest, Sha256};
use std::{
    env,
    ffi::OsString,
    fmt::Write,
    fs,
    io::{self, Write as IoWrite},
    path::{Path, PathBuf},
};

const PKL_OUTPUT_DIR: &str = "pkl-generated";
const STATIC_OUTPUT_DIR: &str = "static";
const MIGRATIONS_ROOT: &str = "migrations";
const GENERATED_INPUT_ENV: &str = "SCE_CLI_GENERATED_INPUT_DIR";
const PACKAGE_FALLBACK_ENV: &str = "SCE_CLI_PACKAGE_FALLBACK";
const GENERATED_INPUT_INVENTORY: &str = "SHA256SUMS";
const CANONICAL_INPUT_INVENTORY: &str = "INPUTS.SHA256SUMS";
const PACKAGE_FALLBACK_DIR: &str = "package-fallback";
const PACKAGE_FALLBACK_INVENTORY: &str = "SHA256SUMS";
const OPTIONAL_WORKFLOW_MANIFEST: &str = "config/optional-workflows.json";
const OPTIONAL_WORKFLOW_MANIFEST_SCHEMA_VERSION: u64 = 1;
const CANONICAL_GENERATOR_INPUTS: &[&str] = &[
    "config/pkl",
    "config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts",
    "config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts",
    "config/lib/pi-plugin/sce-pi-extension.ts",
];

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
        println!("cargo:rerun-if-env-changed={GENERATED_INPUT_ENV}");
        println!("cargo:rerun-if-env-changed={PACKAGE_FALLBACK_ENV}");
        // Sandboxed source builds (Flatpak) compile a full repository checkout
        // without Pkl, so they stage the packaging fallback and opt into it
        // explicitly. Every other repository build must supply the handoff.
        if package_fallback_requested(env::var_os(PACKAGE_FALLBACK_ENV))? {
            stage_packaged_fallback(&manifest_dir, &out_dir)?;
        } else {
            let generated_input_root = generated_input_root(env::var_os(GENERATED_INPUT_ENV))?;
            stage_generated_input(repository_root, &generated_input_root, &out_dir)?;
            stage_static_inputs(repository_root, &manifest_dir, &out_dir)?;
        }
    } else {
        stage_packaged_fallback(&manifest_dir, &out_dir)?;
    }
    validate_staged_artifacts(&out_dir)?;
    generate_embedded_asset_manifest(&out_dir)?;
    generate_optional_workflow_catalog(&out_dir)?;
    generate_migration_manifest(&out_dir)
}

fn repository_sources_available(repository_root: &Path) -> bool {
    repository_root.join("config/pkl/generate.pkl").is_file()
        && repository_root.join("config/lib").is_dir()
}

fn package_fallback_requested(value: Option<OsString>) -> io::Result<bool> {
    let Some(value) = value else {
        return Ok(false);
    };
    let value = value
        .to_str()
        .ok_or_else(|| invalid_data(&format!("{PACKAGE_FALLBACK_ENV} must be valid UTF-8")))?;
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" => Ok(false),
        "1" | "true" => Ok(true),
        other => Err(invalid_data(&format!(
            "{PACKAGE_FALLBACK_ENV} must be '1', 'true', '0', or 'false'; got '{other}'"
        ))),
    }
}

fn generated_input_root(value: Option<OsString>) -> io::Result<PathBuf> {
    let value = value.ok_or_else(|| {
        invalid_data(&format!(
            "repository builds require a pre-generated Pkl payload. Set {GENERATED_INPUT_ENV} to a generated-input directory containing {PKL_OUTPUT_DIR}/, {GENERATED_INPUT_INVENTORY}, and {CANONICAL_INPUT_INVENTORY}, or set {PACKAGE_FALLBACK_ENV}=1 for Pkl-free sandboxed source builds that stage {PACKAGE_FALLBACK_DIR}/"
        ))
    })?;
    if value.is_empty() {
        return Err(invalid_data(&format!(
            "{GENERATED_INPUT_ENV} is empty; set it to a generated-input directory"
        )));
    }
    Ok(PathBuf::from(value))
}

fn stage_generated_input(
    repository_root: &Path,
    generated_input_root: &Path,
    out_dir: &Path,
) -> io::Result<()> {
    for relative_path in CANONICAL_GENERATOR_INPUTS {
        let path = repository_root.join(relative_path);
        if path.is_dir() {
            emit_rerun_tree(&path)?;
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    let payload_inventory = generated_input_root.join(GENERATED_INPUT_INVENTORY);
    let canonical_inventory = generated_input_root.join(CANONICAL_INPUT_INVENTORY);
    println!("cargo:rerun-if-changed={}", payload_inventory.display());
    println!("cargo:rerun-if-changed={}", canonical_inventory.display());

    validate_inventory(
        generated_input_root,
        &[PKL_OUTPUT_DIR],
        &payload_inventory,
        "generated Pkl payload",
    )?;
    validate_inventory(
        repository_root,
        CANONICAL_GENERATOR_INPUTS,
        &canonical_inventory,
        "canonical Pkl inputs",
    )
    .map_err(|error| {
        invalid_data(&format!(
            "generated Pkl payload at '{}' is stale: {error}. Regenerate it from the current config/pkl and config/lib inputs",
            generated_input_root.display()
        ))
    })?;

    let output_root = out_dir.join(PKL_OUTPUT_DIR);
    remove_path_if_exists(&output_root)?;
    copy_tree(&generated_input_root.join(PKL_OUTPUT_DIR), &output_root)?;

    Ok(())
}

fn validate_inventory(
    root: &Path,
    directories: &[&str],
    inventory_path: &Path,
    description: &str,
) -> io::Result<()> {
    let expected = fs::read_to_string(inventory_path).map_err(|error| {
        invalid_data(&format!(
            "{description} inventory '{}' is unavailable: {error}",
            inventory_path.display()
        ))
    })?;
    let actual = inventory_for_paths(root, directories).map_err(|error| {
        invalid_data(&format!(
            "could not inspect {description} under '{}': {error}",
            root.display()
        ))
    })?;
    if actual != expected {
        return Err(invalid_data(&format!(
            "{description} inventory '{}' does not match the current files",
            inventory_path.display()
        )));
    }
    Ok(())
}

fn inventory_for_paths(root: &Path, paths: &[&str]) -> io::Result<String> {
    let mut files = Vec::new();
    for relative_path in paths {
        let path = root.join(relative_path);
        if path.is_dir() {
            collect_files(root, &path, &mut files)?;
        } else if path.is_file() {
            files.push(SourceFile {
                relative_path: normalize_relative_path(Path::new(relative_path))?,
                absolute_path: path,
            });
        } else {
            return Err(invalid_data(&format!(
                "required path '{}' is missing",
                path.display()
            )));
        }
    }
    files.sort_unstable_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let mut inventory = String::new();
    for file in files {
        let digest = compute_sha256(&fs::read(&file.absolute_path)?);
        writeln!(inventory, "{}  {}", format_hex(&digest), file.relative_path)
            .expect("writing to String buffer should never fail");
    }
    Ok(inventory)
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
        "pkl-generated/config/optional-workflows.json",
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

/// Projects the generated optional-workflow manifest into a Rust catalog so the
/// CLI carries optional workflow identity without re-declaring it. Pkl stays the
/// single source: adding an optional workflow to the catalog is enough.
fn generate_optional_workflow_catalog(out_dir: &Path) -> io::Result<()> {
    let manifest_path = out_dir
        .join(PKL_OUTPUT_DIR)
        .join(OPTIONAL_WORKFLOW_MANIFEST);
    let destination_path = out_dir.join("optional_workflows.rs");

    let bytes = fs::read(&manifest_path).map_err(|error| {
        invalid_data(&format!(
            "optional-workflow manifest '{}' is unavailable: {error}",
            manifest_path.display()
        ))
    })?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        invalid_data(&format!(
            "optional-workflow manifest '{}' is not valid JSON: {error}",
            manifest_path.display()
        ))
    })?;

    if manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(OPTIONAL_WORKFLOW_MANIFEST_SCHEMA_VERSION)
    {
        return Err(invalid_data(&format!(
            "optional-workflow manifest '{}' must declare schemaVersion {OPTIONAL_WORKFLOW_MANIFEST_SCHEMA_VERSION}",
            manifest_path.display()
        )));
    }

    let workflows = manifest
        .get("workflows")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            invalid_data(&format!(
                "optional-workflow manifest '{}' must contain a 'workflows' array",
                manifest_path.display()
            ))
        })?;

    let mut output = String::from("// @generated by build.rs; do not edit by hand.\n\n");
    output.push_str("pub static OPTIONAL_WORKFLOWS: &[OptionalWorkflow] = &[\n");
    for workflow in workflows {
        writeln!(
            output,
            "    OptionalWorkflow {{ id: \"{}\", title: \"{}\", description: \"{}\", command_slug: \"{}\", skill_slug: \"{}\" }},",
            manifest_string_field(workflow, "id", &manifest_path)?,
            manifest_string_field(workflow, "title", &manifest_path)?,
            manifest_string_field(workflow, "description", &manifest_path)?,
            manifest_string_field(workflow, "commandSlug", &manifest_path)?,
            manifest_string_field(workflow, "skillSlug", &manifest_path)?,
        )
        .expect("writing to String buffer should never fail");
    }
    output.push_str("];\n");

    write_if_changed(&destination_path, output.as_bytes())
}

fn manifest_string_field(
    workflow: &serde_json::Value,
    field: &str,
    manifest_path: &Path,
) -> io::Result<String> {
    workflow
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(escape_for_rust_string)
        .ok_or_else(|| {
            invalid_data(&format!(
                "optional-workflow manifest '{}' has an entry without a non-empty '{field}' string",
                manifest_path.display()
            ))
        })
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

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::{
        generated_input_root, inventory_for_paths, package_fallback_requested,
        stage_generated_input, CANONICAL_GENERATOR_INPUTS, CANONICAL_INPUT_INVENTORY,
        GENERATED_INPUT_INVENTORY, PKL_OUTPUT_DIR,
    };

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "sce-build-script-{name}-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create test temporary directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove test temporary directory");
        }
    }

    fn write_file(root: &Path, relative_path: &str, contents: &str) {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("fixture file parent"))
            .expect("create fixture directory");
        fs::write(path, contents).expect("write fixture file");
    }

    fn create_fixture() -> (TempDir, PathBuf, PathBuf) {
        let temp = TempDir::new("handoff");
        let repository_root = temp.path().join("repository");
        let handoff_root = temp.path().join("handoff");

        write_file(&repository_root, "config/pkl/generate.pkl", "generator");
        write_file(
            &repository_root,
            "config/lib/agent-trace-plugin/opencode-sce-agent-trace-plugin.ts",
            "agent trace",
        );
        write_file(
            &repository_root,
            "config/lib/bash-policy-plugin/opencode-bash-policy-plugin.ts",
            "bash policy",
        );
        write_file(
            &repository_root,
            "config/lib/pi-plugin/sce-pi-extension.ts",
            "pi extension",
        );
        write_file(
            &handoff_root,
            "pkl-generated/config/.opencode/opencode.json",
            "opencode",
        );
        write_file(
            &handoff_root,
            "pkl-generated/config/.claude/settings.json",
            "claude",
        );
        write_file(
            &handoff_root,
            "pkl-generated/config/.pi/extensions/sce/index.ts",
            "pi",
        );

        let payload_inventory =
            inventory_for_paths(&handoff_root, &[PKL_OUTPUT_DIR]).expect("payload inventory");
        fs::write(
            handoff_root.join(GENERATED_INPUT_INVENTORY),
            payload_inventory,
        )
        .expect("write payload inventory");
        let canonical_inventory = inventory_for_paths(&repository_root, CANONICAL_GENERATOR_INPUTS)
            .expect("canonical inventory");
        fs::write(
            handoff_root.join(CANONICAL_INPUT_INVENTORY),
            canonical_inventory,
        )
        .expect("write canonical inventory");

        (temp, repository_root, handoff_root)
    }

    #[test]
    fn generated_input_requires_handoff_environment_value() {
        let error = generated_input_root(None).expect_err("missing handoff must fail");
        assert!(error
            .to_string()
            .contains("repository builds require a pre-generated Pkl payload"));
        assert!(error.to_string().contains("SCE_CLI_PACKAGE_FALLBACK=1"));
        assert!(generated_input_root(Some(OsString::from(""))).is_err());
    }

    #[test]
    fn package_fallback_opt_in_is_explicit() {
        assert!(!package_fallback_requested(None).expect("absent value is not an opt-in"));
        for unset in ["", "0", "false", "FALSE"] {
            assert!(!package_fallback_requested(Some(OsString::from(unset)))
                .expect("negative value is not an opt-in"));
        }
        for set in ["1", "true", "True"] {
            assert!(package_fallback_requested(Some(OsString::from(set)))
                .expect("positive value is an opt-in"));
        }

        let error = package_fallback_requested(Some(OsString::from("yes")))
            .expect_err("unrecognized value must fail");
        assert!(error
            .to_string()
            .contains("SCE_CLI_PACKAGE_FALLBACK must be"));
    }

    #[test]
    fn missing_generated_input_directory_is_rejected() {
        let temp = TempDir::new("missing-handoff");
        let repository_root = temp.path().join("repository");
        write_file(&repository_root, "config/pkl/generate.pkl", "generator");
        for relative_path in CANONICAL_GENERATOR_INPUTS.iter().skip(1) {
            write_file(&repository_root, relative_path, "plugin");
        }

        let error = stage_generated_input(
            &repository_root,
            &temp.path().join("missing"),
            &temp.path().join("out"),
        )
        .expect_err("missing handoff directory must fail");
        assert!(error.to_string().contains("inventory"));
        assert!(error.to_string().contains("is unavailable"));
    }

    #[test]
    fn valid_generated_input_is_copied_with_matching_inventory() {
        let (temp, repository_root, handoff_root) = create_fixture();
        let out_dir = temp.path().join("out");

        stage_generated_input(&repository_root, &handoff_root, &out_dir)
            .expect("valid handoff should be staged");

        let expected =
            inventory_for_paths(&handoff_root, &[PKL_OUTPUT_DIR]).expect("source inventory");
        let actual = inventory_for_paths(&out_dir, &[PKL_OUTPUT_DIR]).expect("copied inventory");
        assert_eq!(actual, expected);
    }

    #[test]
    fn incomplete_generated_input_is_rejected() {
        let (temp, repository_root, handoff_root) = create_fixture();
        fs::remove_file(handoff_root.join("pkl-generated/config/.claude/settings.json"))
            .expect("remove generated fixture file");

        let error =
            stage_generated_input(&repository_root, &handoff_root, &temp.path().join("out"))
                .expect_err("incomplete handoff must fail");
        assert!(error
            .to_string()
            .contains("generated Pkl payload inventory"));
    }

    #[test]
    fn stale_generated_input_is_rejected() {
        let (temp, repository_root, handoff_root) = create_fixture();
        write_file(
            &repository_root,
            "config/lib/pi-plugin/sce-pi-extension.ts",
            "changed extension",
        );

        let error =
            stage_generated_input(&repository_root, &handoff_root, &temp.path().join("out"))
                .expect_err("stale handoff must fail");
        assert!(error.to_string().contains("is stale"));
        assert!(error.to_string().contains("Regenerate it"));
    }
}
