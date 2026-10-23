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
//! manifest, rewrites its items and summary. `generate` answers with `not_implemented`.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use rustdoc_types::{Crate, FORMAT_VERSION};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

mod cargo_doc;
mod classify;
mod hash;
mod manifest;
mod render;
mod surface;

pub use cargo_doc::{CrateJson, ExtractRequest, ExtractResult, extract};
pub use classify::{Category, Class, Mapping};
pub use manifest::{Configuration, Decision, DecisionKind, Item, Manifest, Status, Summary};

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

/// Runs one command against a JSON request and returns the response envelope.
#[must_use]
pub fn run(command: Command, request_json: &str) -> Response {
    let outcome = match command {
        Command::Extract => handle(command, request_json, extract),
        Command::Analyze => handle(command, request_json, analyze),
        Command::Generate => Err(EngineError::new(ErrorCode::NotImplemented, format!("`{command}` is not implemented yet"))),
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
    let aliases = surface::public_to_canonical(&krate);
    let type_map: BTreeMap<String, Mapping> = configuration
        .type_map
        .iter()
        .map(|(path, mapping)| (aliases.get(path).cloned().unwrap_or_else(|| path.clone()), *mapping))
        .collect();
    let classifier = classify::Classifier::new(&renderer, &type_map, &configuration.exclude_paths);
    let decisions = manifest.as_ref().map_or(&[][..], |manifest| manifest.decisions.as_slice());
    let (items, summary) = manifest::items(&functions, &renderer, &classifier, decisions);
    let manifest_updated = match (manifest.as_mut(), request.manifest.as_deref()) {
        (Some(manifest), Some(path)) => {
            manifest.items = items;
            manifest.summary = summary.clone();
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
    fn unimplemented_commands_report_structured_error() {
        let response = run(Command::Generate, "{}");
        assert!(!response.is_success());
        assert_eq!(response.error.map(|error| error.code), Some(ErrorCode::NotImplemented));
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
        fs::write(&manifest_path, r#"{"configuration": {"type_map": {"demo::Repo": "repository"}, "exclude_paths": ["demo::schema"]}}"#)
            .expect("the manifest can be written");
        let request = AnalyzeRequest { rustdoc_json: extracted.crates[0].json.clone(), manifest: Some(manifest_path.clone()) };

        let first = analyze(&request).expect("the surface is analyzed");
        assert!(first.manifest_updated);
        assert_eq!((first.public_methods, first.public_free_functions), (5, 1));
        let manifest = Manifest::read(&manifest_path).expect("the manifest reads back");
        let state = |path: &str| manifest.items.iter().find(|item| item.path == path).map(|item| (item.class, item.status));
        assert_eq!(state("demo::Repo::name"), Some((Class::Automatic, Status::Ready)));
        assert_eq!(state("demo::open"), Some((Class::Automatic, Status::Ready)), "public type-map names reach canonical paths");
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

        #[deprecated]
        pub fn old(&self) {}
    }
}

pub use inner::Repo;

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
    fn response_omits_absent_fields() {
        let json = serde_json::to_string(&run(Command::Extract, "{}")).unwrap();
        assert!(json.contains(r#""command":"extract""#));
        assert!(!json.contains(r#""result""#));
    }
}