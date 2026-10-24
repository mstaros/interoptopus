//! The per-target manifest (design section "Manifest"). People write the configuration and the
//! decisions; `analyze` rewrites the items and the summary.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::classify::{Class, Classifier, Mapping};
use crate::hash::Fnv;
use crate::render::Renderer;
use crate::surface::Function;
use crate::{EngineError, ErrorCode};

/// One manifest file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Settings for extraction, analysis and generation.
    pub configuration: Configuration,
    /// Decisions for items that are not automatic.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<Decision>,
    /// Counts from the last `analyze`.
    #[serde(default)]
    pub summary: Summary,
    /// What the last `analyze` read, so `generate` can refuse a manifest analyzed from other
    /// input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    /// Every surface function from the last `analyze`, sorted by path.
    #[serde(default)]
    pub items: Vec<Item>,
}

/// Manifest settings; generation settings are carried for later stages and unused by `analyze`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    /// The workspace `Cargo.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_manifest: Option<PathBuf>,
    /// The crate whose surface is generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_crate: Option<String>,
    /// Package-name prefix of the dependencies documented with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crate_prefix: Option<String>,
    /// Features enabled on the root crate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// The pinned nightly toolchain for rustdoc JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<String>,
    /// The interoptopus version the facade uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interoptopus_version: Option<String>,
    /// The generated facade crate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facade_crate: Option<String>,
    /// Output folder for generated Rust.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust_output: Option<PathBuf>,
    /// Output folder for generated C#.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csharp_output: Option<PathBuf>,
    /// Path of the hand-written runtime module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_module: Option<String>,
    /// Root namespace of the public C# layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_namespace: Option<String>,
    /// Namespace of the generated interop layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_namespace: Option<String>,
    /// Name of the native library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_name: Option<String>,
    /// Type-map entries by canonical or public type path, extending and overriding the defaults.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub type_map: BTreeMap<String, Mapping>,
    /// Item path prefixes excluded by rule, such as `gix::config::tree`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_paths: Vec<String>,
}

/// The kind of a decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    /// Re-create a handle from a key.
    Handle,
    /// Copy owned fields.
    Record,
    /// Map arguments.
    Convert,
    /// Leave the item out.
    Exclude,
    /// Implemented by hand.
    Manual,
}

/// A decision for one item, valid while its signature hash matches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// The item path.
    pub path: String,
    /// The signature hash the decision was made for.
    pub signature_hash: String,
    /// What to do.
    pub kind: DecisionKind,
    /// Kind-specific fields, kept as written.
    #[serde(flatten)]
    pub details: BTreeMap<String, serde_json::Value>,
}

/// Where an item stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Automatic and ready to generate.
    Ready,
    /// Needs a decision.
    Unresolved,
    /// A decision matches the current signature.
    Resolved,
    /// A decision exists for the path but for another signature.
    Stale,
    /// Left out by rule.
    Excluded,
}

/// One surface function in the manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    /// Public path.
    pub path: String,
    /// Canonical, lifetime-free signature.
    pub signature: String,
    /// Hash of path and signature.
    pub signature_hash: String,
    /// Automatic, decision or excluded.
    pub class: Class,
    /// Where the item stands.
    pub status: Status,
    /// Why the item is not automatic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Hashes of the input an `analyze` read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// Hash of the rustdoc JSON text.
    pub rustdoc_json: String,
    /// Hash of the configuration.
    pub configuration: String,
}

impl Provenance {
    /// The provenance of analyzing the rustdoc JSON texts `sources` (the root crate's first)
    /// under `configuration`.
    ///
    /// # Errors
    ///
    /// Fails only if the configuration cannot be serialized.
    pub fn of(sources: &[&str], configuration: &Configuration) -> Result<Self, EngineError> {
        let settings = serde_json::to_string(configuration).map_err(|error| EngineError::new(ErrorCode::Internal, error.to_string()))?;
        let mut source = Fnv::new();
        for json in sources {
            source.field(json.as_bytes());
        }
        let mut options = Fnv::new();
        options.field(settings.as_bytes());
        Ok(Self { rustdoc_json: source.hex(), configuration: options.hex() })
    }
}

/// Counts over the items.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    /// All surface functions, including excluded ones.
    pub total: usize,
    /// Automatic items.
    pub automatic: usize,
    /// Items excluded by rule.
    pub excluded: usize,
    /// Items whose decision matches their signature.
    pub resolved: usize,
    /// Items that still need a decision.
    pub unresolved: usize,
    /// Items whose decision was made for another signature.
    pub stale: usize,
    /// Decisions whose path is no longer on the surface.
    pub orphaned_decisions: usize,
    /// Whether generation may run: nothing unresolved, stale or orphaned.
    pub ready: bool,
    /// Unresolved, stale and excluded items per reason category.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reasons: BTreeMap<String, usize>,
}

impl Manifest {
    /// Reads a manifest file.
    pub fn read(path: &Path) -> Result<Self, EngineError> {
        let text = fs::read_to_string(path).map_err(|error| EngineError::new(ErrorCode::Io, format!("{}: {error}", path.display())))?;
        serde_json::from_str(&text)
            .map_err(|error| EngineError::new(ErrorCode::InvalidRequest, format!("invalid manifest {}: {error}", path.display())))
    }

    /// The manifest as pretty JSON with a trailing newline.
    pub fn to_json(&self) -> Result<String, EngineError> {
        let mut text = serde_json::to_string_pretty(self).map_err(|error| EngineError::new(ErrorCode::Internal, error.to_string()))?;
        text.push('\n');
        Ok(text)
    }

    /// Writes the manifest when its text changed, through a temporary file and a rename; returns
    /// whether the file was written.
    pub fn write(&self, path: &Path) -> Result<bool, EngineError> {
        let text = self.to_json()?;
        if fs::read_to_string(path).is_ok_and(|current| current == text) {
            return Ok(false);
        }
        let io = |error: std::io::Error| EngineError::new(ErrorCode::Io, format!("{}: {error}", path.display()));
        let staging = path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::write(&staging, text).map_err(io)?;
        fs::rename(&staging, path).map_err(io)?;
        Ok(true)
    }
}

/// The hash a decision is checked against.
#[must_use]
pub fn signature_hash(path: &str, signature: &str) -> String {
    let mut hash = Fnv::new();
    hash.field(path.as_bytes());
    hash.field(signature.as_bytes());
    hash.hex()
}

/// Classifies the surface, sets each item's status from the decisions and counts the result.
#[must_use]
pub fn items(functions: &[Function], renderer: &Renderer, classifier: &Classifier<'_>, decisions: &[Decision]) -> (Vec<Item>, Summary) {
    let decided: HashSet<&str> = decisions.iter().map(|decision| decision.signature_hash.as_str()).collect();
    let decided_paths: HashSet<&str> = decisions.iter().map(|decision| decision.path.as_str()).collect();
    let mut summary = Summary::default();
    let mut items = Vec::with_capacity(functions.len());
    for function in functions {
        let signature = function.signature(renderer);
        let signature_hash = signature_hash(&function.path, &signature);
        let verdict = classifier.classify(function);
        let status = match verdict.class {
            _ if decided.contains(signature_hash.as_str()) => Status::Resolved,
            Class::Automatic => Status::Ready,
            Class::Excluded => Status::Excluded,
            Class::Decision if decided_paths.contains(function.path.as_str()) => Status::Stale,
            Class::Decision => Status::Unresolved,
        };
        summary.total += 1;
        match status {
            Status::Ready => summary.automatic += 1,
            Status::Excluded => summary.excluded += 1,
            Status::Resolved => summary.resolved += 1,
            Status::Unresolved => summary.unresolved += 1,
            Status::Stale => summary.stale += 1,
        }
        if let Some(reason) = verdict.reason.as_ref().filter(|_| matches!(status, Status::Unresolved | Status::Stale | Status::Excluded)) {
            *summary.reasons.entry(reason.category.name().to_owned()).or_default() += 1;
        }
        items.push(Item {
            path: function.path.clone(),
            signature,
            signature_hash,
            class: verdict.class,
            status,
            reason: verdict.reason.map(|reason| reason.to_string()),
        });
    }
    let paths: HashSet<&str> = functions.iter().map(|function| function.path.as_str()).collect();
    summary.orphaned_decisions = decisions.iter().filter(|decision| !paths.contains(decision.path.as_str())).count();
    summary.ready = summary.unresolved == 0 && summary.stale == 0 && summary.orphaned_decisions == 0;
    (items, summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::Receiver;
    use rustdoc_types::Type;

    fn function(path: &str, receiver: Receiver) -> Function {
        Function {
            path: path.to_owned(),
            name: path.rsplit("::").next().unwrap_or_default().to_owned(),
            owner: Some("demo::Repo".to_owned()),
            owner_public: None,
            receiver,
            params: Vec::new(),
            output: Some(Type::Primitive("bool".to_owned())),
            generics: BTreeMap::new(),
            deprecated: false,
            is_async: false,
            is_unsafe: false,
        }
    }

    fn decision(path: &str, signature_hash: String) -> Decision {
        Decision { path: path.to_owned(), signature_hash, kind: DecisionKind::Exclude, details: BTreeMap::new() }
    }

    #[test]
    fn decisions_resolve_by_signature_and_go_stale_when_it_changes() {
        let renderer = Renderer::from_paths([]);
        let types = BTreeMap::from([("demo::Repo".to_owned(), Mapping::Handle)]);
        let implementations = crate::classify::Implementations::default();
        let classifier = Classifier::new(&renderer, &types, &[], &implementations);
        let functions = vec![
            function("demo::Repo::automatic", Receiver::Shared),
            function("demo::Repo::decided", Receiver::Exclusive),
            function("demo::Repo::changed", Receiver::Exclusive),
            function("demo::Repo::open", Receiver::Exclusive),
        ];
        let decided_hash = signature_hash("demo::Repo::decided", &functions[1].signature(&renderer));
        let decisions = vec![
            decision("demo::Repo::decided", decided_hash),
            decision("demo::Repo::changed", "0000000000000000".to_owned()),
            decision("demo::Repo::removed", "0000000000000001".to_owned()),
        ];

        let (items, summary) = items(&functions, &renderer, &classifier, &decisions);
        let statuses: Vec<Status> = items.iter().map(|item| item.status).collect();
        assert_eq!(statuses, [Status::Ready, Status::Resolved, Status::Stale, Status::Unresolved]);
        assert_eq!((summary.total, summary.automatic, summary.resolved, summary.stale, summary.unresolved), (4, 1, 1, 1, 1));
        assert_eq!(summary.orphaned_decisions, 1);
        assert!(!summary.ready);
        assert_eq!(summary.reasons.get("mut_handle"), Some(&2));
    }

    #[test]
    fn writing_an_unchanged_manifest_leaves_the_file_alone() {
        let dir = tempfile::tempdir().expect("temporary directory can be created");
        let path = dir.path().join("manifest.json");
        let manifest = Manifest {
            configuration: Configuration { root_crate: Some("demo".to_owned()), ..Configuration::default() },
            ..Manifest::default()
        };
        assert!(manifest.write(&path).expect("first write succeeds"));
        assert!(!manifest.write(&path).expect("second write succeeds"));
        assert_eq!(Manifest::read(&path).expect("the manifest reads back"), manifest);
        let text = fs::read_to_string(&path).expect("the manifest is readable");
        assert!(
            !text.contains("\"decisions\"") && !text.contains("\"type_map\""),
            "empty sections stay out of the file"
        );
    }
}