//! The `extract` command: rustdoc JSON for a root crate and for its dependencies that share
//! the root's package-name prefix.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use rustdoc_types::FORMAT_VERSION;
use serde::{Deserialize, Serialize};

use crate::hash::Fnv;
use crate::{EngineError, ErrorCode};

/// Directories inside a crate that never contribute to its documentation.
const SKIPPED_DIRS: [&str; 6] = ["target", ".git", "tests", "benches", "examples", "fuzz"];

/// File that marks a complete cache entry and records its result.
const MANIFEST: &str = "extract.json";

/// Request for `extract`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractRequest {
    /// The workspace `Cargo.toml` that contains the root crate.
    pub workspace_manifest: PathBuf,
    /// The crate whose public API is documented, for example `gix`.
    pub root_crate: String,
    /// Dependencies are documented only when their package name starts with this prefix.
    pub crate_prefix: String,
    /// Features enabled on the root crate.
    #[serde(default)]
    pub features: Vec<String>,
    /// The rustup toolchain whose rustdoc writes the JSON, for example `nightly-2026-07-18`.
    pub toolchain: String,
    /// Directory for build output and cached results; keep it outside the repository.
    pub cache_dir: PathBuf,
}

/// Result of `extract`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractResult {
    /// Content hash of all inputs; names the cache entry.
    pub cache_key: String,
    /// Whether the result came from the cache without running rustdoc.
    pub cache_hit: bool,
    /// rustdoc JSON format version of the root crate's file.
    pub format_version: u32,
    /// Documented crates, sorted by package name.
    pub crates: Vec<CrateJson>,
}

/// One documented crate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateJson {
    /// Package name, for example `gix-hash`.
    pub name: String,
    /// The crate's rustdoc JSON file inside the cache entry.
    pub json: PathBuf,
}

/// Runs `extract`, reusing the cached result when no input has changed.
pub fn extract(request: &ExtractRequest) -> Result<ExtractResult, EngineError> {
    let metadata = cargo_metadata(request)?;
    let crates = select_crates(&metadata, &request.root_crate, &request.crate_prefix)?;
    let cache_key = cache_key(request, &crates)?;
    let entry = request.cache_dir.join(&cache_key);
    if let Some(cached) = read_entry(&entry)? {
        return Ok(ExtractResult { cache_hit: true, ..cached });
    }
    let doc_dir = cargo_doc(request, &crates)?;
    publish_entry(&entry, &cache_key, &doc_dir, &crates, &request.root_crate)
}

/// A package selected for documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectedCrate {
    name: String,
    version: String,
    lib_name: String,
    dir: PathBuf,
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    resolve: Option<Resolve>,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    version: String,
    manifest_path: PathBuf,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<NodeDep>,
}

#[derive(Deserialize)]
struct NodeDep {
    pkg: String,
    dep_kinds: Vec<DepKind>,
}

#[derive(Deserialize)]
struct DepKind {
    kind: Option<String>,
}

#[derive(Deserialize)]
struct FormatProbe {
    format_version: u32,
}

fn cargo(toolchain: &str) -> Process {
    let mut command = Process::new("cargo");
    command.arg(format!("+{toolchain}"));
    command
}

/// The root crate's features as `root/feature`, sorted and deduplicated.
fn feature_list(request: &ExtractRequest) -> Option<String> {
    let features: BTreeSet<String> = request.features.iter().map(|feature| format!("{}/{feature}", request.root_crate)).collect();
    if features.is_empty() { None } else { Some(features.into_iter().collect::<Vec<_>>().join(",")) }
}

fn run(mut command: Process, tool: &str) -> Result<Vec<u8>, EngineError> {
    let output = command.output().map_err(|error| EngineError::new(ErrorCode::Io, format!("cannot start {tool}: {error}")))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    let tail = lines[lines.len().saturating_sub(40)..].join("\n");
    Err(EngineError::new(ErrorCode::ToolFailed, format!("{tool} failed ({}):\n{tail}", output.status)))
}

fn cargo_metadata(request: &ExtractRequest) -> Result<Metadata, EngineError> {
    let mut command = cargo(&request.toolchain);
    command.args(["metadata", "--locked", "--format-version", "1", "--manifest-path"]).arg(&request.workspace_manifest);
    if let Some(features) = feature_list(request) {
        command.arg("--features").arg(features);
    }
    let stdout = run(command, "cargo metadata")?;
    serde_json::from_slice(&stdout)
        .map_err(|error| EngineError::new(ErrorCode::Internal, format!("unexpected cargo metadata output: {error}")))
}

/// The root crate plus every crate reachable through normal dependencies whose package name
/// starts with `prefix`, sorted by name.
fn select_crates(metadata: &Metadata, root: &str, prefix: &str) -> Result<Vec<SelectedCrate>, EngineError> {
    let packages: BTreeMap<&str, &Package> = metadata.packages.iter().map(|package| (package.id.as_str(), package)).collect();
    let root_id = metadata
        .workspace_members
        .iter()
        .map(String::as_str)
        .find(|id| packages.get(*id).is_some_and(|package| package.name == root))
        .ok_or_else(|| EngineError::new(ErrorCode::InvalidRequest, format!("`{root}` is not a member of the workspace")))?;
    let resolve = metadata
        .resolve
        .as_ref()
        .ok_or_else(|| EngineError::new(ErrorCode::Internal, "cargo metadata returned no dependency resolution"))?;
    let nodes: BTreeMap<&str, &Node> = resolve.nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut selected = BTreeSet::from([root_id]);
    let mut pending = VecDeque::from([root_id]);
    while let Some(id) = pending.pop_front() {
        let Some(node) = nodes.get(id) else { continue };
        for dep in &node.deps {
            let normal = dep.dep_kinds.iter().any(|kind| kind.kind.is_none());
            let prefixed = packages.get(dep.pkg.as_str()).is_some_and(|package| package.name.starts_with(prefix));
            if normal && prefixed && selected.insert(dep.pkg.as_str()) {
                pending.push_back(dep.pkg.as_str());
            }
        }
    }
    let mut crates = selected
        .into_iter()
        .filter_map(|id| packages.get(id).copied())
        .map(selected_crate)
        .collect::<Result<Vec<_>, _>>()?;
    crates.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(crates)
}

fn selected_crate(package: &Package) -> Result<SelectedCrate, EngineError> {
    let lib_name = package
        .targets
        .iter()
        .find(|target| target.kind.iter().any(|kind| matches!(kind.as_str(), "lib" | "rlib" | "dylib" | "proc-macro")))
        .map(|target| target.name.replace('-', "_"))
        .ok_or_else(|| EngineError::new(ErrorCode::InvalidRequest, format!("`{}` has no library target", package.name)))?;
    let dir = package
        .manifest_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| EngineError::new(ErrorCode::Internal, format!("`{}` has no manifest directory", package.name)))?;
    Ok(SelectedCrate { name: package.name.clone(), version: package.version.clone(), lib_name, dir })
}

/// Hash of everything that can change the rustdoc output: engine version, toolchain, features,
/// `Cargo.lock` and the sources of every selected crate.
fn cache_key(request: &ExtractRequest, crates: &[SelectedCrate]) -> Result<String, EngineError> {
    let mut hash = Fnv::new();
    hash.field(env!("CARGO_PKG_VERSION").as_bytes());
    hash.field(request.toolchain.as_bytes());
    hash.field(feature_list(request).unwrap_or_default().as_bytes());
    hash.field(&read(&request.workspace_manifest.with_file_name("Cargo.lock"))?);
    for krate in crates {
        hash.field(krate.name.as_bytes());
        hash.field(krate.version.as_bytes());
        for file in source_files(&krate.dir)? {
            let relative = file.strip_prefix(&krate.dir).unwrap_or(&file);
            hash.field(relative.to_string_lossy().as_bytes());
            hash.field(&read(&file)?);
        }
    }
    Ok(hash.hex())
}

/// All files below `dir`, sorted, skipping directories that never feed rustdoc.
fn source_files(dir: &Path) -> Result<Vec<PathBuf>, EngineError> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).map_err(|error| io_error(&current, &error))? {
            let entry = entry.map_err(|error| io_error(&current, &error))?;
            let path = entry.path();
            if entry.file_type().map_err(|error| io_error(&path, &error))?.is_dir() {
                if !SKIPPED_DIRS.iter().any(|skipped| entry.file_name() == *skipped) {
                    pending.push(path);
                }
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn cargo_doc(request: &ExtractRequest, crates: &[SelectedCrate]) -> Result<PathBuf, EngineError> {
    let workspace = request
        .workspace_manifest
        .parent()
        .ok_or_else(|| EngineError::new(ErrorCode::InvalidRequest, "workspace_manifest has no parent directory"))?;
    let canonical = fs::canonicalize(workspace).map_err(|error| io_error(workspace, &error))?;
    let mut checkout = Fnv::new();
    checkout.field(canonical.to_string_lossy().as_bytes());
    // One target directory per checkout. Cargo names path-dependency artifacts identically in
    // every checkout and judges freshness by modification time, so a shared directory can link
    // artifacts built from another checkout's sources.
    let target = request.cache_dir.join(format!("target-{}", checkout.hex()));
    let mut command = cargo(&request.toolchain);
    command.args(["doc", "--locked", "--no-deps", "--manifest-path"]).arg(&request.workspace_manifest);
    for krate in crates {
        command.arg("-p").arg(format!("{}@{}", krate.name, krate.version));
    }
    if let Some(features) = feature_list(request) {
        command.arg("--features").arg(features);
    }
    command
        .env("RUSTDOCFLAGS", "-Z unstable-options --output-format json")
        .env_remove("CARGO_ENCODED_RUSTDOCFLAGS")
        .env("CARGO_TARGET_DIR", &target);
    run(command, "cargo doc")?;
    Ok(target.join("doc"))
}

/// Copies the JSON files into a staging directory and renames it into place, so a cache entry is
/// either complete or absent.
fn publish_entry(entry: &Path, cache_key: &str, doc_dir: &Path, crates: &[SelectedCrate], root: &str) -> Result<ExtractResult, EngineError> {
    let staging = entry.with_extension(format!("staging-{}", std::process::id()));
    let staged_doc = staging.join("doc");
    fs::create_dir_all(&staged_doc).map_err(|error| io_error(&staged_doc, &error))?;
    let mut documented = Vec::with_capacity(crates.len());
    let mut root_json = None;
    for krate in crates {
        let file_name = format!("{}.json", krate.lib_name);
        let source = doc_dir.join(&file_name);
        fs::copy(&source, staged_doc.join(&file_name)).map_err(|error| io_error(&source, &error))?;
        if krate.name == root {
            root_json = Some(staged_doc.join(&file_name));
        }
        documented.push(CrateJson { name: krate.name.clone(), json: entry.join("doc").join(&file_name) });
    }
    let root_json = root_json.ok_or_else(|| EngineError::new(ErrorCode::Internal, format!("`{root}` was not documented")))?;
    let format_version = read_format_version(&root_json)?;
    if format_version != FORMAT_VERSION {
        return Err(EngineError::new(
            ErrorCode::UnsupportedFormat,
            format!("{} has rustdoc JSON format {format_version}, this engine reads format {FORMAT_VERSION}", root_json.display()),
        ));
    }
    let result = ExtractResult { cache_key: cache_key.to_owned(), cache_hit: false, format_version, crates: documented };
    let manifest = serde_json::to_vec_pretty(&result).map_err(|error| EngineError::new(ErrorCode::Internal, error.to_string()))?;
    fs::write(staging.join(MANIFEST), manifest).map_err(|error| io_error(&staging, &error))?;
    if let Err(error) = fs::rename(&staging, entry) {
        // Another run may have published the same key first; its content is identical.
        if let Some(existing) = read_entry(entry)? {
            let _ = fs::remove_dir_all(&staging);
            return Ok(existing);
        }
        return Err(io_error(entry, &error));
    }
    Ok(result)
}

fn read_entry(entry: &Path) -> Result<Option<ExtractResult>, EngineError> {
    let manifest = entry.join(MANIFEST);
    if !manifest.is_file() {
        return Ok(None);
    }
    let result: ExtractResult = serde_json::from_slice(&read(&manifest)?)
        .map_err(|error| EngineError::new(ErrorCode::Internal, format!("{}: {error}", manifest.display())))?;
    Ok(result.crates.iter().all(|krate| krate.json.is_file()).then_some(result))
}

fn read_format_version(path: &Path) -> Result<u32, EngineError> {
    let probe: FormatProbe = serde_json::from_slice(&read(path)?)
        .map_err(|error| EngineError::new(ErrorCode::InvalidRequest, format!("{} is not rustdoc JSON: {error}", path.display())))?;
    Ok(probe.format_version)
}

fn read(path: &Path) -> Result<Vec<u8>, EngineError> {
    fs::read(path).map_err(|error| io_error(path, &error))
}

fn io_error(path: &Path, error: &io::Error) -> EngineError {
    EngineError::new(ErrorCode::Io, format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOOLCHAIN: &str = "nightly-2026-07-18";

    const METADATA: &str = r#"{
        "packages": [
            {"id": "root", "name": "gix", "version": "1.0.0", "manifest_path": "/w/gix/Cargo.toml", "targets": [{"name": "gix", "kind": ["lib"]}]},
            {"id": "hash", "name": "gix-hash", "version": "1.0.0", "manifest_path": "/w/gix-hash/Cargo.toml", "targets": [{"name": "gix_hash", "kind": ["lib"]}]},
            {"id": "feat", "name": "gix-features", "version": "1.0.0", "manifest_path": "/w/gix-features/Cargo.toml", "targets": [{"name": "gix_features", "kind": ["lib"]}]},
            {"id": "tools", "name": "gix-testtools", "version": "1.0.0", "manifest_path": "/w/tests/tools/Cargo.toml", "targets": [{"name": "gix_testtools", "kind": ["lib"]}]},
            {"id": "bstr", "name": "bstr", "version": "1.0.0", "manifest_path": "/r/bstr/Cargo.toml", "targets": [{"name": "bstr", "kind": ["lib"]}]}
        ],
        "workspace_members": ["root", "hash", "feat", "tools"],
        "resolve": {"nodes": [
            {"id": "root", "deps": [
                {"pkg": "hash", "dep_kinds": [{"kind": null}]},
                {"pkg": "bstr", "dep_kinds": [{"kind": null}]},
                {"pkg": "tools", "dep_kinds": [{"kind": "dev"}]}
            ]},
            {"id": "hash", "deps": [{"pkg": "feat", "dep_kinds": [{"kind": null}]}]},
            {"id": "feat", "deps": []},
            {"id": "tools", "deps": []},
            {"id": "bstr", "deps": []}
        ]}
    }"#;

    fn metadata() -> Metadata {
        serde_json::from_str(METADATA).unwrap()
    }

    fn demo_request(root: &Path, features: &[&str]) -> ExtractRequest {
        ExtractRequest {
            workspace_manifest: root.join("Cargo.toml"),
            root_crate: "demo".to_owned(),
            crate_prefix: "demo".to_owned(),
            features: features.iter().map(|feature| (*feature).to_owned()).collect(),
            toolchain: TOOLCHAIN.to_owned(),
            cache_dir: root.join("cache"),
        }
    }

    #[test]
    fn selects_the_root_and_prefixed_normal_dependencies() {
        let crates = select_crates(&metadata(), "gix", "gix").unwrap();
        let names: Vec<&str> = crates.iter().map(|krate| krate.name.as_str()).collect();
        assert_eq!(names, ["gix", "gix-features", "gix-hash"], "dev and unprefixed dependencies are left out");
        assert_eq!(crates[1].lib_name, "gix_features");
        assert_eq!(crates[2].dir, PathBuf::from("/w/gix-hash"));
    }

    #[test]
    fn rejects_a_root_outside_the_workspace() {
        let error = select_crates(&metadata(), "bstr", "gix").unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn features_are_qualified_sorted_and_deduplicated() {
        let root = Path::new("/w");
        assert_eq!(feature_list(&demo_request(root, &["b", "a", "b"])).as_deref(), Some("demo/a,demo/b"));
        assert_eq!(feature_list(&demo_request(root, &[])), None);
    }

    #[test]
    fn cache_key_tracks_sources_and_ignores_feature_order_and_test_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Cargo.lock"), "lock").unwrap();
        fs::create_dir_all(root.join("demo/src")).unwrap();
        fs::create_dir_all(root.join("demo/tests")).unwrap();
        fs::write(root.join("demo/src/lib.rs"), "pub fn first() {}").unwrap();
        let crates = [SelectedCrate { name: "demo".to_owned(), version: "0.1.0".to_owned(), lib_name: "demo".to_owned(), dir: root.join("demo") }];

        let key = cache_key(&demo_request(root, &["b", "a"]), &crates).unwrap();
        assert_eq!(key, cache_key(&demo_request(root, &["a", "b"]), &crates).unwrap());
        fs::write(root.join("demo/tests/extra.rs"), "#[test] fn extra() {}").unwrap();
        assert_eq!(key, cache_key(&demo_request(root, &["a", "b"]), &crates).unwrap(), "tests do not feed rustdoc");
        fs::write(root.join("demo/src/lib.rs"), "pub fn second() {}").unwrap();
        assert_ne!(key, cache_key(&demo_request(root, &["a", "b"]), &crates).unwrap());
    }

    #[test]
    #[ignore = "runs cargo doc with the pinned nightly toolchain"]
    fn extract_documents_a_workspace_and_reuses_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let files = [
            ("Cargo.toml", "[workspace]\nmembers = [\"demo\", \"demo-core\", \"other\"]\nresolver = \"3\"\n"),
            (
                "demo/Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\ndemo-core = { path = \"../demo-core\" }\nother = { path = \"../other\" }\n\n[features]\nextra = []\n",
            ),
            ("demo/src/lib.rs", "pub fn answer() -> demo_core::Value {\n    demo_core::Value(42)\n}\n"),
            ("demo-core/Cargo.toml", "[package]\nname = \"demo-core\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
            ("demo-core/src/lib.rs", "pub struct Value(pub u32);\n"),
            ("other/Cargo.toml", "[package]\nname = \"other\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
            ("other/src/lib.rs", "pub fn noop() {}\n"),
        ];
        for (relative, content) in files {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        let mut lock = cargo(TOOLCHAIN);
        lock.args(["generate-lockfile", "--offline", "--manifest-path"]).arg(root.join("Cargo.toml"));
        run(lock, "cargo generate-lockfile").unwrap();

        let request = demo_request(root, &["extra"]);
        let first = extract(&request).unwrap();
        assert!(!first.cache_hit);
        assert_eq!(first.format_version, FORMAT_VERSION);
        let names: Vec<&str> = first.crates.iter().map(|krate| krate.name.as_str()).collect();
        assert_eq!(names, ["demo", "demo-core"]);
        assert!(first.crates.iter().all(|krate| krate.json.is_file()));

        let second = extract(&request).unwrap();
        assert!(second.cache_hit);
        assert_eq!(second.cache_key, first.cache_key);
        assert_eq!(second.crates, first.crates);
    }
}