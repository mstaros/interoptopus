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
//! `extract` runs rustdoc with JSON output for a crate and its prefixed dependencies, and
//! `analyze` reports the public function surface of one crate's JSON. `generate` answers
//! with `not_implemented`.

use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use rustdoc_types::{Crate, FORMAT_VERSION, Id, ItemEnum, Type, Visibility};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

mod cargo_doc;

pub use cargo_doc::{CrateJson, ExtractRequest, ExtractResult, extract};

/// The rustdoc `crate_id` of the documented crate itself.
const LOCAL_CRATE: u32 = 0;

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
}

/// Result of `analyze`: the public function surface of one crate.
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

/// Loads one crate's rustdoc JSON and counts its public function surface.
pub fn analyze(request: &AnalyzeRequest) -> Result<AnalyzeResult, EngineError> {
    let json = fs::read_to_string(&request.rustdoc_json)
        .map_err(|e| EngineError::new(ErrorCode::Io, format!("cannot read {}: {e}", request.rustdoc_json.display())))?;
    let krate = load_crate(&json)?;
    Ok(surface(&krate))
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

/// Counts the public, non-deprecated inherent methods and free functions of the documented crate.
#[must_use]
pub fn surface(krate: &Crate) -> AnalyzeResult {
    let public_methods = krate
        .index
        .values()
        .filter(|item| item.crate_id == LOCAL_CRATE)
        .filter_map(|item| match &item.inner {
            ItemEnum::Impl(imp) if imp.trait_.is_none() && !imp.is_synthetic && imp.blanket_impl.is_none() => Some(imp),
            _ => None,
        })
        .filter(|imp| matches!(&imp.for_, Type::ResolvedPath(path) if krate.paths.contains_key(&path.id)))
        .map(|imp| imp.items.iter().filter(|id| is_public_function(krate, **id)).count())
        .sum();
    AnalyzeResult {
        format_version: krate.format_version,
        crate_name: krate.index.get(&krate.root).and_then(|root| root.name.clone()).unwrap_or_default(),
        index_items: krate.index.len(),
        public_methods,
        public_free_functions: count_free_functions(krate, krate.root),
    }
}

fn is_public_function(krate: &Crate, id: Id) -> bool {
    krate.index.get(&id).is_some_and(|item| {
        matches!(item.inner, ItemEnum::Function(_)) && matches!(item.visibility, Visibility::Public) && item.deprecation.is_none()
    })
}

fn count_free_functions(krate: &Crate, module_id: Id) -> usize {
    let Some(ItemEnum::Module(module)) = krate.index.get(&module_id).map(|item| &item.inner) else {
        return 0;
    };
    module
        .items
        .iter()
        .filter_map(|id| krate.index.get(id).filter(|item| matches!(item.visibility, Visibility::Public)).map(|item| (*id, item)))
        .map(|(id, item)| match &item.inner {
            ItemEnum::Module(_) => count_free_functions(krate, id),
            ItemEnum::Function(_) if item.deprecation.is_none() => 1,
            _ => 0,
        })
        .sum()
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
    fn response_omits_absent_fields() {
        let json = serde_json::to_string(&run(Command::Extract, "{}")).unwrap();
        assert!(json.contains(r#""command":"extract""#));
        assert!(!json.contains(r#""result""#));
    }
}