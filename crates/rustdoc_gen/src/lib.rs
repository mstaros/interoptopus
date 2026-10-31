//! Generates interoptopus facades and C# bindings from rustdoc JSON.
//!
//! The `interoptopus-rustdoc-gen` binary reads a JSON request file and writes a JSON
//! response file (see `docs/rust-interop-service.md`):
//!
//! ```text
//! interoptopus-rustdoc-gen <extract|analyze|generate> --request <file> --response <file>
//! ```
//!
//! Exit codes: `0` success, `1` the command failed and the response file holds the error,
//! `2` usage or file error before a response could be written (message on stderr).
//!
//! `extract` runs rustdoc with JSON output for a crate and its prefixed dependencies.
//! `analyze` classifies the public function surface of the root crate's JSON and, given a
//! manifest, rewrites its items and summary. `generate` emits the Rust facade for the manifest's
//! ready automatic items into a worktree. The C# bindings are interoptopus': its C# backend
//! generates them from the facade, and the engine writes no C#.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use rustdoc_types::{Crate, FORMAT_VERSION};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

mod cargo_doc;
mod classify;
mod facade;
mod forbidden;
mod hash;
mod manifest;
mod propose;
mod render;
mod surface;

pub use cargo_doc::{CrateJson, ExtractRequest, ExtractResult, extract};
pub use classify::{Carrier, Category, Class, Custom, Implementations, Mapping};
pub use manifest::{Configuration, Decision, DecisionKind, Item, Manifest, Proposal, Provenance, Status, Summary};

/// An engine subcommand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// Run `cargo doc` and collect rustdoc JSON.
    Extract,
    /// Classify the public surface and update the manifest.
    Analyze,
    /// Emit the Rust facade and the C# layer.
    Generate,
}

impl Command {
    /// The command-line spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Extract => "extract",
            Self::Analyze => "analyze",
            Self::Generate => "generate",
        }
    }
}

impl FromStr for Command {
    type Err = EngineError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        [Self::Extract, Self::Analyze, Self::Generate]
            .into_iter()
            .find(|command| command.name() == value)
            .ok_or_else(|| EngineError::new(ErrorCode::InvalidRequest, format!("unknown command `{value}`")))
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Machine-readable error category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The command line or the request JSON is malformed.
    InvalidRequest,
    /// A file could not be read or written.
    Io,
    /// The rustdoc JSON format version differs from the one this engine reads.
    UnsupportedFormat,
    /// The manifest does not match the current surface; run `analyze` first.
    StaleManifest,
    /// Items are undecided, stale or decided with a kind generation does not support yet, and the
    /// request did not ask for partial generation.
    NotReady,
    /// The subcommand is not implemented yet.
    NotImplemented,
    /// An external tool such as cargo or rustdoc failed.
    ToolFailed,
    /// An unexpected internal failure.
    Internal,
}

/// A structured engine error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EngineError {
    /// Error category.
    pub code: ErrorCode,
    /// Diagnostic message.
    pub message: String,
}

impl EngineError {
    /// Creates an error.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for EngineError {}

/// The response envelope written by the binary.
#[derive(Debug, Serialize)]
pub struct Response {
    /// The command that produced this response.
    pub command: Command,
    /// The command result, present on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// The error, present on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<EngineError>,
}

impl Response {
    /// Whether the command succeeded.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }
}

/// Request for `analyze`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyzeRequest {
    /// The root crate's rustdoc JSON file.
    pub rustdoc_json: PathBuf,
    /// The manifest to classify against and update; without one, defaults apply and nothing
    /// is written.
    #[serde(default)]
    pub manifest: Option<PathBuf>,
    /// The dependency crates' rustdoc JSON files, read for their trait implementations.
    #[serde(default)]
    pub dependency_json: Vec<PathBuf>,
}

/// Result of `analyze`: the classified public function surface of one crate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AnalyzeResult {
    /// rustdoc JSON format version of the input.
    pub format_version: u32,
    /// Name of the documented crate.
    pub crate_name: String,
    /// Number of items in the rustdoc index.
    pub index_items: usize,
    /// Public, non-deprecated inherent methods of public types.
    pub public_methods: usize,
    /// Public, non-deprecated free functions reachable through public modules.
    pub public_free_functions: usize,
    /// Counts over the classified surface.
    pub summary: Summary,
    /// Whether the manifest file changed.
    pub manifest_updated: bool,
}

/// Request for `generate`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateRequest {
    /// The root crate's rustdoc JSON file.
    pub rustdoc_json: PathBuf,
    /// The manifest; its items must match the current surface.
    pub manifest: PathBuf,
    /// The transaction worktree that receives the generated files; the manifest's output
    /// folders are relative to it.
    pub worktree: PathBuf,
    /// Emit the ready items only and report the rest as pending, instead of refusing.
    #[serde(default)]
    pub partial: bool,
    /// The dependency crates' rustdoc JSON files, read for their trait implementations.
    #[serde(default)]
    pub dependency_json: Vec<PathBuf>,
}

/// An item left out of the facade.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Dropped {
    /// The item path.
    pub path: String,
    /// Why it was left out.
    pub reason: String,
}

/// Result of `generate`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GenerateResult {
    /// The generated facade module, `mod.rs` in the manifest's Rust output folder.
    pub rust_module: PathBuf,
    /// Whether the module file changed.
    pub written: bool,
    /// Items emitted into the facade.
    pub emitted: usize,
    /// Ready items left out, such as another handle type's duplicate of a method.
    pub dropped: Vec<Dropped>,
    /// Items not generated yet: unresolved, stale, or decided with a kind the emitter does not
    /// support yet. Items decided `exclude` or `manual` need no generated code.
    pub pending: usize,
}

/// Runs one command against a JSON request and returns the response envelope.
#[must_use]
pub fn run(command: Command, request_json: &str) -> Response {
    let outcome = match command {
        Command::Extract => handle(command, request_json, extract),
        Command::Analyze => handle(command, request_json, analyze),
        Command::Generate => handle(command, request_json, generate),
    };
    match outcome {
        Ok(result) => Response { command, result: Some(result), error: None },
        Err(error) => Response { command, result: None, error: Some(error) },
    }
}

/// Parses a typed request, runs `handler` and serializes its result.
fn handle<Request, Output>(
    command: Command,
    request_json: &str,
    handler: impl FnOnce(&Request) -> Result<Output, EngineError>,
) -> Result<serde_json::Value, EngineError>
where
    Request: DeserializeOwned,
    Output: Serialize,
{
    let request = serde_json::from_str::<Request>(request_json)
        .map_err(|e| EngineError::new(ErrorCode::InvalidRequest, format!("invalid {command} request: {e}")))?;
    let output = handler(&request)?;
    serde_json::to_value(output).map_err(|e| EngineError::new(ErrorCode::Internal, e.to_string()))
}

/// Classifies the public function surface of one crate's rustdoc JSON and updates the manifest.
pub fn analyze(request: &AnalyzeRequest) -> Result<AnalyzeResult, EngineError> {
    let json = fs::read_to_string(&request.rustdoc_json)
        .map_err(|e| EngineError::new(ErrorCode::Io, format!("cannot read {}: {e}", request.rustdoc_json.display())))?;
    let krate = load_crate(&json)?;
    let mut manifest = request.manifest.as_deref().map(Manifest::read).transpose()?;
    let configuration = manifest.as_ref().map(|manifest| manifest.configuration.clone()).unwrap_or_default();
    let renderer = render::Renderer::new(&krate);
    let functions = surface::collect(&krate, &renderer);
    let type_map = type_map(&krate, &configuration);
    let (implementations, digests) = dependencies(&krate, &request.dependency_json)?;
    let classifier = classify::Classifier::new(&renderer, &type_map, &configuration.exclude_paths, &implementations);
    classifier.validate().map_err(|why| EngineError::new(ErrorCode::InvalidRequest, why))?;
    let decisions = manifest.as_ref().map_or(&[][..], |manifest| manifest.decisions.as_slice());
    let runtime = configuration.runtime_module.as_deref().unwrap_or("crate::runtime");
    let lowering = facade::Context { renderer: &renderer, classifier: &classifier, runtime, provenance: "" };
    let (items, summary) = manifest::items(&functions, &renderer, |function| facade::verdict(function, &lowering), decisions);
    let proposals = {
        let pending: std::collections::HashSet<&str> = items
            .iter()
            .filter(|item| matches!(item.status, manifest::Status::Unresolved | manifest::Status::Stale))
            .map(|item| item.path.as_str())
            .collect();
        let public = surface::public_to_canonical(&krate);
        let context = propose::Context { renderer: &renderer, classifier: &classifier, implementations: &implementations, public: &public };
        propose::propose(&functions, &pending, &context)
    };
    let manifest_updated = match (manifest.as_mut(), request.manifest.as_deref()) {
        (Some(manifest), Some(path)) => {
            manifest.proposals = proposals.entries;
            manifest.handle_candidates = proposals.handles;
            manifest.items = items;
            manifest.summary = summary.clone();
            manifest.provenance = Some(Provenance::of(&sources(&json, &digests), &configuration)?);
            manifest.write(path)?
        }
        _ => false,
    };
    let current = |owned: bool| functions.iter().filter(|function| function.owner.is_some() == owned && !function.deprecated).count();
    Ok(AnalyzeResult {
        format_version: krate.format_version,
        crate_name: krate.index.get(&krate.root).and_then(|root| root.name.clone()).unwrap_or_default(),
        index_items: krate.index.len(),
        public_methods: current(true),
        public_free_functions: current(false),
        summary,
        manifest_updated,
    })
}

/// Emits the Rust facade for the manifest's ready automatic items.
///
/// # Errors
///
/// Fails when the manifest was analyzed from other input or its items differ from a fresh
/// analysis (`stale_manifest`), when items are not ready and the request does not ask for
/// partial generation (`not_ready`), when the Rust output folder is missing or leaves the
/// worktree, or when a ready item cannot be lowered.
pub fn generate(request: &GenerateRequest) -> Result<GenerateResult, EngineError> {
    let json = fs::read_to_string(&request.rustdoc_json)
        .map_err(|e| EngineError::new(ErrorCode::Io, format!("cannot read {}: {e}", request.rustdoc_json.display())))?;
    let krate = load_crate(&json)?;
    let manifest = Manifest::read(&request.manifest)?;
    let configuration = &manifest.configuration;
    let rust_output = configuration
        .rust_output
        .as_ref()
        .ok_or_else(|| EngineError::new(ErrorCode::InvalidRequest, "the manifest configuration names no `rust_output` folder"))?;
    let rust_module = contained(&request.worktree, rust_output)?.join("mod.rs");
    let (implementations, digests) = dependencies(&krate, &request.dependency_json)?;
    if manifest.provenance.as_ref() != Some(&Provenance::of(&sources(&json, &digests), configuration)?) {
        return Err(EngineError::new(
            ErrorCode::StaleManifest,
            "the manifest was analyzed from other rustdoc JSON or another configuration; run analyze first",
        ));
    }
    let renderer = render::Renderer::new(&krate);
    let functions = surface::collect(&krate, &renderer);
    let type_map = type_map(&krate, configuration);
    let classifier = classify::Classifier::new(&renderer, &type_map, &configuration.exclude_paths, &implementations);
    classifier.validate().map_err(|why| EngineError::new(ErrorCode::InvalidRequest, why))?;
    let runtime = configuration.runtime_module.as_deref().unwrap_or("crate::runtime");
    let lowering = facade::Context { renderer: &renderer, classifier: &classifier, runtime, provenance: "" };
    let (items, _) = manifest::items(&functions, &renderer, |function| facade::verdict(function, &lowering), &manifest.decisions);
    if items != manifest.items {
        return Err(EngineError::new(ErrorCode::StaleManifest, "the manifest items differ from a fresh analysis; run analyze first"));
    }
    let ready: BTreeSet<&str> =
        items.iter().filter(|item| item.class == Class::Automatic && item.status == Status::Ready).map(|item| item.path.as_str()).collect();
    let settled = manifest::settled(&manifest.decisions);
    let pending = items
        .iter()
        .filter(|item| item.status != Status::Excluded && !ready.contains(item.path.as_str()) && !settled.contains(item.signature_hash.as_str()))
        .count();
    if pending > 0 && !request.partial {
        return Err(EngineError::new(
            ErrorCode::NotReady,
            format!("{pending} items are undecided, stale or decided with a kind generation does not support yet; decide them or request partial generation"),
        ));
    }
    let selected: Vec<&surface::Function> = functions.iter().filter(|function| ready.contains(function.path.as_str())).collect();
    let crate_name = krate.index.get(&krate.root).and_then(|root| root.name.clone()).unwrap_or_default();
    let header = format!(
        "Generated by interoptopus-rustdoc-gen {} from `{crate_name}` {} (rustdoc format {}). Do not edit.",
        env!("CARGO_PKG_VERSION"),
        krate.crate_version.as_deref().unwrap_or("(unversioned)"),
        krate.format_version
    );
    let context = facade::Context { renderer: &renderer, classifier: &classifier, runtime, provenance: &header };
    let facade = facade::emit(&selected, &context)
        .map_err(|(path, why)| EngineError::new(ErrorCode::Internal, format!("cannot emit `{path}`: {why}")))?;
    let written = write_if_changed(&rust_module, &facade.source)?;
    Ok(GenerateResult {
        rust_module,
        written,
        emitted: facade.emitted.len(),
        dropped: facade.dropped.into_iter().map(|(path, reason)| Dropped { path, reason }).collect(),
        pending,
    })
}

/// `worktree` joined with `folder`, which must be relative and stay inside it.
fn contained(worktree: &Path, folder: &Path) -> Result<PathBuf, EngineError> {
    let inside = folder.components().all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
        && folder.components().any(|component| matches!(component, Component::Normal(_)));
    if !inside {
        return Err(EngineError::new(
            ErrorCode::InvalidRequest,
            format!("the output folder `{}` must be relative and stay inside the worktree", folder.display()),
        ));
    }
    Ok(worktree.join(folder))
}

/// The manifest's type map keyed by canonical paths.
fn type_map(krate: &Crate, configuration: &Configuration) -> BTreeMap<String, Mapping> {
    let aliases = surface::public_to_canonical(krate);
    let canonical = |path: &String| aliases.get(path).cloned().unwrap_or_else(|| path.clone());
    configuration
        .type_map
        .iter()
        .map(|(path, mapping)| {
            let mapping = match mapping {
                Mapping::Keyed(keyed) => Mapping::Keyed(classify::Keyed {
                    handle: canonical(&keyed.handle),
                    key: keyed.key.as_ref().map(canonical),
                    name: path.clone(),
                    ..keyed.clone()
                }),
                other => other.clone(),
            };
            (canonical(path), mapping)
        })
        .collect()
}

/// The trait implementations and mirror definitions of the root crate and of the dependency
/// crates' JSON, with a digest of each dependency's JSON for provenance.
fn dependencies(krate: &Crate, dependency_json: &[PathBuf]) -> Result<(classify::Implementations, Vec<String>), EngineError> {
    let mut implementations = classify::Implementations::default();
    implementations.add_root(krate);
    let mut digests = Vec::with_capacity(dependency_json.len());
    for path in dependency_json {
        let text = fs::read_to_string(path).map_err(|e| EngineError::new(ErrorCode::Io, format!("cannot read {}: {e}", path.display())))?;
        implementations.add(&load_crate(&text)?);
        let mut digest = hash::Fnv::new();
        digest.field(text.as_bytes());
        digests.push(digest.hex());
    }
    implementations.finish();
    Ok((implementations, digests))
}

/// The provenance sources: the root crate's JSON, then each dependency's digest.
fn sources<'a>(json: &'a str, digests: &'a [String]) -> Vec<&'a str> {
    std::iter::once(json).chain(digests.iter().map(String::as_str)).collect()
}

/// Replaces `path` with `text` through a temporary file, unless it already holds exactly that.
fn write_if_changed(path: &Path, text: &str) -> Result<bool, EngineError> {
    if fs::read_to_string(path).is_ok_and(|existing| existing == text) {
        return Ok(false);
    }
    let io = |e: std::io::Error| EngineError::new(ErrorCode::Io, format!("cannot write {}: {e}", path.display()));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io)?;
    }
    let temporary = path.with_extension("rs.tmp");
    fs::write(&temporary, text).map_err(io)?;
    fs::rename(&temporary, path).map_err(io)?;
    Ok(true)
}

/// Parses rustdoc JSON after checking that its format version is the one this engine reads.
pub fn load_crate(json: &str) -> Result<Crate, EngineError> {
    #[derive(Deserialize)]
    struct FormatProbe {
        format_version: u32,
    }

    let probe: FormatProbe =
        serde_json::from_str(json).map_err(|e| EngineError::new(ErrorCode::InvalidRequest, format!("not rustdoc JSON: {e}")))?;
    if probe.format_version != FORMAT_VERSION {
        return Err(EngineError::new(
            ErrorCode::UnsupportedFormat,
            format!("rustdoc JSON format {} is not supported, this engine reads format {FORMAT_VERSION}", probe.format_version),
        ));
    }
    serde_json::from_str(json).map_err(|e| EngineError::new(ErrorCode::InvalidRequest, format!("invalid rustdoc JSON: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_commands() {
        assert_eq!("analyze".parse::<Command>().unwrap(), Command::Analyze);
        assert_eq!("extract".parse::<Command>().unwrap(), Command::Extract);
        assert_eq!("generate".parse::<Command>().unwrap(), Command::Generate);
    }

    #[test]
    fn rejects_unknown_command() {
        let error = "publish".parse::<Command>().unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn extract_rejects_malformed_request() {
        let response = run(Command::Extract, r#"{"workspace_manifest": "Cargo.toml"}"#);
        assert_eq!(response.error.map(|error| error.code), Some(ErrorCode::InvalidRequest));
    }

    #[test]
    fn generate_rejects_malformed_request() {
        let response = run(Command::Generate, "{}");
        assert!(!response.is_success());
        assert_eq!(response.error.map(|error| error.code), Some(ErrorCode::InvalidRequest));
    }

    #[test]
    fn output_folders_must_stay_inside_the_worktree() {
        let worktree = Path::new("worktree");
        assert_eq!(contained(worktree, Path::new("facade/src/generated")).unwrap(), worktree.join("facade/src/generated"));
        for escaping in ["../outside", "facade/../../outside", "", ".", "/absolute"] {
            assert_eq!(contained(worktree, Path::new(escaping)).unwrap_err().code, ErrorCode::InvalidRequest, "{escaping}");
        }
    }

    #[test]
    fn provenance_follows_the_rustdoc_json_and_the_configuration() {
        let configuration = Configuration::default();
        let first = Provenance::of(&["{}"], &configuration).unwrap();
        assert_eq!(first, Provenance::of(&["{}"], &configuration).unwrap());
        assert_ne!(first.rustdoc_json, Provenance::of(&["{ }"], &configuration).unwrap().rustdoc_json);
        assert_ne!(first.rustdoc_json, Provenance::of(&["{}", "dependency"], &configuration).unwrap().rustdoc_json);
        let moved = Configuration { runtime_module: Some("crate::other".to_owned()), ..Configuration::default() };
        assert_ne!(first.configuration, Provenance::of(&["{}"], &moved).unwrap().configuration);
    }

    #[test]
    fn analyze_rejects_malformed_request() {
        let response = run(Command::Analyze, r#"{"unexpected": true}"#);
        assert_eq!(response.error.map(|error| error.code), Some(ErrorCode::InvalidRequest));
    }

    #[test]
    fn analyze_reports_missing_file_as_io_error() {
        let response = run(Command::Analyze, r#"{"rustdoc_json": "this/file/does/not/exist.json"}"#);
        assert_eq!(response.error.map(|error| error.code), Some(ErrorCode::Io));
    }

    #[test]
    fn rejects_other_format_versions() {
        let error = load_crate(r#"{"format_version": 1}"#).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedFormat);
    }

    #[test]
    #[ignore = "runs cargo doc with the pinned nightly toolchain"]
    fn analyze_classifies_a_workspace_and_rewrites_the_manifest_deterministically() {
        let dir = tempfile::tempdir().expect("temporary directory can be created");
        let root = dir.path();
        let files = [
            ("Cargo.toml", "[workspace]\nmembers = [\"demo\"]\nresolver = \"3\"\n"),
            ("demo/Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
            ("demo/src/lib.rs", DEMO),
        ];
        for (relative, content) in files {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().expect("files have a parent")).expect("directories can be created");
            fs::write(path, content).expect("files can be written");
        }
        let lock = std::process::Command::new("cargo")
            .args(["+nightly-2026-07-18", "generate-lockfile", "--offline", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .status()
            .expect("cargo runs");
        assert!(lock.success());
        let extracted = extract(&ExtractRequest {
            workspace_manifest: root.join("Cargo.toml"),
            root_crate: "demo".to_owned(),
            crate_prefix: "demo".to_owned(),
            features: Vec::new(),
            toolchain: "nightly-2026-07-18".to_owned(),
            cache_dir: root.join("cache"),
        })
        .expect("the workspace is documented");
        let manifest_path = root.join("manifest.json");
        fs::write(&manifest_path, r#"{"configuration": {"type_map": {"demo::Repo": "handle"}, "exclude_paths": ["demo::schema"]}}"#)
            .expect("the manifest can be written");
        let request =
            AnalyzeRequest { rustdoc_json: extracted.crates[0].json.clone(), manifest: Some(manifest_path.clone()), dependency_json: Vec::new() };

        let first = analyze(&request).expect("the surface is analyzed");
        assert!(first.manifest_updated);
        assert_eq!((first.public_methods, first.public_free_functions), (16, 1));
        let manifest = Manifest::read(&manifest_path).expect("the manifest reads back");
        let state = |path: &str| manifest.items.iter().find(|item| item.path == path).map(|item| (item.class, item.status));
        assert_eq!(state("demo::Repo::name"), Some((Class::Automatic, Status::Ready)));
        assert_eq!(state("demo::open"), Some((Class::Automatic, Status::Ready)), "public type-map names reach canonical paths");
        let mirrored = ["demo::Repo::mode", "demo::Repo::set_mode", "demo::Repo::last_event", "demo::Repo::stats", "demo::Repo::set_stats", "demo::Repo::label"];
        for mirrored in mirrored {
            assert_eq!(state(mirrored), Some((Class::Automatic, Status::Ready)), "{mirrored} crosses through mirrors");
        }
        assert_eq!(state("demo::Repo::rename"), Some((Class::Decision, Status::Unresolved)));
        assert_eq!(state("demo::Repo::find"), Some((Class::Decision, Status::Unresolved)));
        assert_eq!(state("demo::Item::id"), Some((Class::Decision, Status::Unresolved)));
        assert_eq!(state("demo::Repo::old"), Some((Class::Excluded, Status::Excluded)));
        assert_eq!(state("demo::schema::Key::name"), Some((Class::Excluded, Status::Excluded)));

        let second = analyze(&request).expect("the surface is analyzed again");
        assert!(!second.manifest_updated, "an unchanged surface leaves the manifest alone");
        assert_eq!(second.summary, first.summary);
    }

    const DEMO: &str = r#"
mod inner {
    pub struct Repo;

    impl Repo {
        pub fn name(&self) -> String {
            String::new()
        }

        pub fn rename(&mut self, name: impl AsRef<str>) {
            let _ = name.as_ref();
        }

        pub fn find(&self, id: u32) -> Option<crate::Item> {
            let _ = id;
            None
        }

        pub fn mode(&self) -> crate::mode::Mode {
            crate::mode::Mode::Read
        }

        pub fn set_mode(&self, mode: crate::mode::Mode) {
            let _ = mode;
        }

        pub fn last_event(&self) -> Option<crate::Event> {
            None
        }

        pub fn stats(&self) -> crate::stats::Stats {
            crate::stats::Stats { count: 0, name: String::new(), mode: crate::mode::Mode::Read, note: None, fallback: None }
        }

        pub fn set_stats(&self, stats: crate::stats::Stats) {
            let _ = stats;
        }

        pub fn label(&self) -> crate::Label<'_> {
            crate::Label { text: "", data: &[] }
        }

        pub fn display_name(&self) -> crate::Name {
            crate::Name(String::new())
        }

        pub fn set_display_name(&self, name: crate::Name) {
            let _ = name;
        }

        pub fn reopen(&self) -> std::io::Result<Repo> {
            Ok(Repo)
        }

        pub fn config(&self) -> crate::Config<'_> {
            crate::Config { repo: self }
        }

        #[deprecated]
        pub fn old(&self) {}
    }
}

pub use inner::Repo;

pub mod mode {
    #[repr(u8)]
    pub enum Mode {
        Read,
        Write = 5,
        Admin(),
    }
}

#[non_exhaustive]
pub enum Event {
    Opened(u32),
    Renamed { from: String, to: String },
    Closed,
}

pub mod stats {
    pub type Count = u32;

    pub struct Stats {
        pub count: Count,
        pub name: String,
        pub mode: crate::mode::Mode,
        pub note: Option<String>,
        pub fallback: Option<crate::mode::Mode>,
    }
}

pub struct Label<'a> {
    pub text: &'a str,
    pub data: &'a [u8],
}

pub struct Name(String);

impl std::str::FromStr for Name {
    type Err = std::convert::Infallible;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(Self(text.to_owned()))
    }
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub struct Config<'a> {
    repo: &'a Repo,
}

impl Config<'_> {
    pub fn depth(&self) -> u32 {
        let _ = self.repo;
        1
    }
}

pub struct Item;

impl Item {
    pub fn id(&self) -> u32 {
        0
    }
}

pub mod schema {
    pub struct Key;

    impl Key {
        pub fn name(&self) -> &'static str {
            "key"
        }
    }
}

pub fn open(path: impl AsRef<std::path::Path>) -> Result<Repo, std::io::Error> {
    let _ = path.as_ref();
    Ok(Repo)
}
"#;

    #[test]
    #[ignore = "runs cargo doc with the pinned nightly toolchain and builds the generated facade"]
    fn generate_emits_a_facade_that_builds_and_registers() {
        let dir = tempfile::tempdir().expect("temporary directory can be created");
        let root = dir.path();
        let core = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../core");
        let facade_manifest = format!(
            "[package]\nname = \"facade\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\ndemo = {{ path = \"../demo\" }}\ninteroptopus = {{ package = \"interoptopus_mps\", path = {:?} }}\n",
            core.display().to_string()
        );
        let files = [
            ("Cargo.toml", "[workspace]\nmembers = [\"demo\", \"facade\"]\nresolver = \"3\"\n".to_owned()),
            ("demo/Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n".to_owned()),
            ("demo/src/lib.rs", DEMO.to_owned()),
            ("facade/Cargo.toml", facade_manifest),
            ("facade/src/lib.rs", FACADE_LIB.to_owned()),
            ("facade/src/runtime.rs", FACADE_RUNTIME.to_owned()),
        ];
        for (relative, content) in files {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().expect("files have a parent")).expect("directories can be created");
            fs::write(path, content).expect("files can be written");
        }
        let cargo = |args: &[&str]| {
            std::process::Command::new("cargo")
                .arg("+nightly-2026-07-18")
                .args(args)
                .arg("--manifest-path")
                .arg(root.join("Cargo.toml"))
                .status()
                .expect("cargo runs")
                .success()
        };
        assert!(cargo(&["generate-lockfile", "--offline"]));
        let extracted = extract(&ExtractRequest {
            workspace_manifest: root.join("Cargo.toml"),
            root_crate: "demo".to_owned(),
            crate_prefix: "demo".to_owned(),
            features: Vec::new(),
            toolchain: "nightly-2026-07-18".to_owned(),
            cache_dir: root.join("cache"),
        })
        .expect("the workspace is documented");
        let manifest_path = root.join("manifest.json");
        let configuration =
            r#"{"configuration": {"type_map": {"demo::Repo": "handle", "demo::Name": "string", "demo::Config": {"keyed": {"handle": "demo::Repo", "rebuild": "{handle}.config()"}}}, "exclude_paths": ["demo::schema"], "rust_output": "facade/src/generated"}}"#;
        fs::write(&manifest_path, configuration).expect("the manifest can be written");
        let rustdoc_json = extracted.crates[0].json.clone();
        let analysis = AnalyzeRequest { rustdoc_json: rustdoc_json.clone(), manifest: Some(manifest_path.clone()), dependency_json: Vec::new() };
        analyze(&analysis).expect("the surface is analyzed");
        let request = GenerateRequest {
            rustdoc_json,
            manifest: manifest_path.clone(),
            worktree: root.to_path_buf(),
            partial: true,
            dependency_json: Vec::new(),
        };
        let complete = GenerateRequest { partial: false, ..request.clone() };
        assert_eq!(generate(&complete).unwrap_err().code, ErrorCode::NotReady, "four items await decisions");

        let generated = generate(&request).expect("the facade is generated");
        assert!(generated.written);
        assert_eq!((generated.emitted, generated.pending), (12, 4), "demo::open, ten Repo methods and a keyless Config method; four items await decisions");
        let source = fs::read_to_string(&generated.rust_module).expect("the facade reads back");
        assert!(source.contains("pub enum ModeMode {") && source.contains("pub enum Event {"), "mirrors of a fieldless and a data enum: {source}");
        assert!(source.contains("pub struct StatsStats {") && source.contains("fn from_stats_stats("), "a record both ways: {source}");
        assert!(source.contains("pub struct Label {") && !source.contains("fn from_label("), "a borrowed view as a result only: {source}");
        assert!(source.contains("name.as_str().parse().map_err(runtime::error)?"), "a type mapped to string parses: {source}");
        assert!(source.contains("    pub fn reopen(repo: &Self) -> ffi::Result<Self, runtime::Error> {\n"), "a method returning the handle takes it as `&Self`: {source}");
        assert!(source.contains("    pub fn config_depth(&self) -> ffi::Result<u32, runtime::Error> {\n        finish(|| {\n            let repo = &self.inner.repo();\n            let config = repo.config();\n            Ok(config.depth())\n"), "a keyless method rebuilds from the handle alone: {source}");
        assert!(!generate(&request).expect("the facade is generated again").written, "regenerating changes nothing");
        assert!(cargo(&["test", "--offline", "-p", "facade"]), "the generated facade builds and registers with an inventory");
        let clippy = std::process::Command::new("cargo")
            .args(["+nightly-2026-07-18", "clippy", "--offline", "-p", "facade", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .args(["--", "-D", "warnings"])
            .status()
            .expect("cargo runs");
        assert!(clippy.success(), "the generated facade passes clippy");

        let mut manifest = Manifest::read(&manifest_path).expect("the manifest reads back");
        manifest.decisions = manifest
            .items
            .iter()
            .filter(|item| item.status == Status::Unresolved)
            .map(|item| Decision { path: item.path.clone(), signature_hash: item.signature_hash.clone(), kind: DecisionKind::Exclude, details: BTreeMap::new() })
            .collect();
        manifest.decisions[0].kind = DecisionKind::Handle;
        manifest.write(&manifest_path).expect("the manifest can be written");
        assert!(!analyze(&analysis).expect("the decisions are analyzed").summary.ready, "a handle decision is not generated yet");
        assert_eq!(generate(&complete).unwrap_err().code, ErrorCode::NotReady, "the handle decision is pending");
        let mut manifest = Manifest::read(&manifest_path).expect("the manifest reads back");
        manifest.decisions[0].kind = DecisionKind::Manual;
        manifest.write(&manifest_path).expect("the manifest can be written");
        assert!(analyze(&analysis).expect("the decisions are analyzed again").summary.ready, "exclude and manual need no generated code");
        let decided = generate(&complete).expect("a ready manifest generates without partial");
        assert_eq!((decided.emitted, decided.pending, decided.written), (12, 0, false), "the decided items stay out of the facade");

        let mut manifest = Manifest::read(&manifest_path).expect("the manifest reads back");
        manifest.items[0].status = Status::Excluded;
        manifest.write(&manifest_path).expect("the manifest can be written");
        assert_eq!(generate(&request).unwrap_err().code, ErrorCode::StaleManifest, "items must match a fresh analysis");

        analyze(&analysis).expect("the surface is analyzed again");
        let mut manifest = Manifest::read(&manifest_path).expect("the manifest reads back");
        manifest.configuration.runtime_module = Some("crate::elsewhere".to_owned());
        manifest.write(&manifest_path).expect("the manifest can be written");
        assert_eq!(generate(&request).unwrap_err().code, ErrorCode::StaleManifest, "a changed configuration needs a new analysis");
    }

    const FACADE_LIB: &str = r#"
pub mod generated;
pub mod runtime;

#[cfg(test)]
mod tests {
    #[test]
    fn inventory_validates() {
        let mut inventory = interoptopus::inventory::RustInventory::new();
        crate::generated::register(&mut inventory);
        let _ = inventory.validate();
    }
}
"#;

    const FACADE_RUNTIME: &str = r#"
use interoptopus::ffi;

#[ffi]
#[derive(Debug, PartialOrd, PartialEq, Copy, Clone)]
pub enum Error {
    Fail,
}

pub fn error<E: std::fmt::Display>(_: E) -> Error {
    Error::Fail
}

pub struct Repo(demo::Repo);

impl From<demo::Repo> for Repo {
    fn from(repo: demo::Repo) -> Self {
        Self(repo)
    }
}

impl Repo {
    pub fn repo(&self) -> &demo::Repo {
        &self.0
    }
}

pub fn path(bytes: &ffi::Slice<u8>) -> Result<std::path::PathBuf, Error> {
    std::str::from_utf8(bytes.as_slice()).map(std::path::PathBuf::from).map_err(error)
}
"#;

    #[test]
    fn response_omits_absent_fields() {
        let json = serde_json::to_string(&run(Command::Extract, "{}")).unwrap();
        assert!(json.contains(r#""command":"extract""#));
        assert!(!json.contains(r#""result""#));
    }
}