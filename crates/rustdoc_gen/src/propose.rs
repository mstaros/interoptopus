//! Type-map proposals (todo E3c). `analyze` collects the types that block pending items and
//! proposes a mapping for each from what it implements: `string` for `Display` plus `FromStr` or
//! `TryFrom<&str>`; `bytes` for `From<&[u8]>` or `From<Vec<u8>>` (in) and `AsRef<[u8]>` or a
//! `Deref` to bytes (out). Owners blocked as receivers become handle candidates, listed apart
//! until the per-type handle kind exists (E4, E5). Each proposal is ranked by the blocked items
//! it unblocks, and the items it removes a blocker from, when re-classified with it applied. A
//! person accepts a proposal by moving it into the type map; whether the emitter can lower the
//! accepted mapping is a separate question (E5's convert kind).

use std::collections::{BTreeMap, HashMap, HashSet};

use rustdoc_types::Type;

use crate::classify::{Category, Classifier, Implementations, Mapping, Reason};
use crate::manifest::Proposal;
use crate::render::{Renderer, type_args};
use crate::surface::{Function, Receiver};

const DISPLAY: &str = "core::fmt::Display";
const FROM_STR: &str = "core::str::traits::FromStr";
const TRY_FROM_STR: &str = "core::convert::TryFrom<&str>";
const AS_REF_BYTES: &str = "core::convert::AsRef<[u8]>";
const FROM_BYTES: &str = "core::convert::From<&[u8]>";
const FROM_BYTE_VEC: &str = "core::convert::From<alloc::vec::Vec<u8>>";
const SEND: &str = "core::marker::Send";
const SYNC: &str = "core::marker::Sync";

/// Ranked type-map proposals and handle candidates.
#[derive(Debug, Default)]
pub struct Proposals {
    /// Type-map entries a person can accept.
    pub entries: Vec<Proposal>,
    /// Types that could become handles once the per-type handle kind exists.
    pub handles: Vec<Proposal>,
}

/// What `propose` reads besides the functions.
pub struct Context<'a> {
    /// Renders paths of the root crate.
    pub renderer: &'a Renderer,
    /// The classifier with the manifest's type map.
    pub classifier: &'a Classifier<'a>,
    /// Trait implementations of the root crate and its dependencies.
    pub implementations: &'a Implementations,
    /// Public path to canonical path, as `surface::public_to_canonical` returns it.
    pub public: &'a HashMap<String, String>,
}

/// Proposes mappings for the types that block the `pending` items.
#[must_use]
pub fn propose(functions: &[Function], pending: &HashSet<&str>, context: &Context<'_>) -> Proposals {
    let blocked: Vec<(&Function, Vec<Reason>)> = functions
        .iter()
        .filter(|function| pending.contains(function.path.as_str()))
        .map(|function| (function, context.classifier.classify(function).reasons))
        .filter(|(_, reasons)| !reasons.is_empty())
        .collect();
    let mut types: BTreeMap<&str, bool> = BTreeMap::new();
    for (_, reasons) in &blocked {
        for reason in reasons {
            if let Some(ty) = reason.ty.as_deref() {
                *types.entry(ty).or_default() |= reason.category == Category::Receiver;
            }
        }
    }
    let names = public_names(context.public);
    let mut proposals = Proposals::default();
    for (ty, receiver) in types {
        let name = names.get(ty).map_or(ty, String::as_str);
        let measure = |mapping: Mapping, directions: Vec<&str>, evidence: Vec<String>| ranked(&blocked, context, ty, name, mapping, directions, evidence);
        if let Some((directions, evidence)) = string_like(ty, context.implementations) {
            proposals.entries.push(measure(Mapping::String, directions, evidence));
        }
        if let Some((directions, evidence)) = byte_like(ty, context) {
            proposals.entries.push(measure(Mapping::Bytes, directions, evidence));
        }
        if receiver {
            proposals.handles.push(measure(Mapping::Handle, Vec::new(), handle_evidence(ty, functions, context)));
        }
    }
    for list in [&mut proposals.entries, &mut proposals.handles] {
        list.retain(|proposal| proposal.reduces > 0);
        list.sort_by(|a, b| {
            b.unblocks.cmp(&a.unblocks).then(b.reduces.cmp(&a.reduces)).then_with(|| a.ty.cmp(&b.ty)).then_with(|| a.mapping.cmp(&b.mapping))
        });
    }
    proposals
}

/// The proposal for `ty`, measured by re-classifying the items it blocks with `mapping` applied.
fn ranked(
    blocked: &[(&Function, Vec<Reason>)],
    context: &Context<'_>,
    ty: &str,
    name: &str,
    mapping: Mapping,
    directions: Vec<&str>,
    evidence: Vec<String>,
) -> Proposal {
    let label = match &mapping {
        Mapping::String => "string",
        Mapping::Bytes => "bytes",
        Mapping::Path => "path",
        Mapping::Handle => "handle",
        Mapping::Custom(_) => "custom",
    };
    let applied = context.classifier.with(ty, mapping);
    let (mut unblocks, mut reduces) = (0, 0);
    for (function, reasons) in blocked.iter().filter(|(_, reasons)| reasons.iter().any(|reason| reason.ty.as_deref() == Some(ty))) {
        let remaining = applied.classify(function).reasons.len();
        if remaining == 0 {
            unblocks += 1;
        }
        if remaining < reasons.len() {
            reduces += 1;
        }
    }
    Proposal {
        ty: name.to_owned(),
        mapping: label.to_owned(),
        directions: directions.into_iter().map(str::to_owned).collect(),
        evidence,
        unblocks,
        reduces,
    }
}

/// `Display` plus a way to parse: crosses as text both ways.
fn string_like(ty: &str, implementations: &Implementations) -> Option<(Vec<&'static str>, Vec<String>)> {
    if !implementations.implements(ty, DISPLAY) {
        return None;
    }
    let parsers: Vec<String> = [(FROM_STR, "FromStr"), (TRY_FROM_STR, "TryFrom<&str>")]
        .into_iter()
        .filter(|(reference, _)| implementations.implements_reference(ty, reference))
        .map(|(_, name)| name.to_owned())
        .collect();
    (!parsers.is_empty()).then(|| (vec!["in", "out"], std::iter::once("Display".to_owned()).chain(parsers).collect()))
}

/// Built from bytes (in), or viewed as bytes (out).
fn byte_like(ty: &str, context: &Context<'_>) -> Option<(Vec<&'static str>, Vec<String>)> {
    let implementations = context.implementations;
    let inputs: Vec<String> = [(FROM_BYTES, "From<&[u8]>"), (FROM_BYTE_VEC, "From<Vec<u8>>")]
        .into_iter()
        .filter(|(reference, _)| implementations.implements_reference(ty, reference))
        .map(|(_, name)| name.to_owned())
        .collect();
    let mut outputs = Vec::new();
    if implementations.implements_reference(ty, AS_REF_BYTES) {
        outputs.push("AsRef<[u8]>".to_owned());
    }
    if let Some(target) = implementations.deref_target(ty)
        && (target == "[u8]" || context.classifier.mapping(target) == Some(Mapping::Bytes))
    {
        outputs.push(format!("Deref<Target = {target}>"));
    }
    let mut directions = Vec::new();
    if !inputs.is_empty() {
        directions.push("in");
    }
    if !outputs.is_empty() {
        directions.push("out");
    }
    (!directions.is_empty()).then(|| (directions, inputs.into_iter().chain(outputs).collect()))
}

/// What speaks for or against `ty` as a handle.
fn handle_evidence(ty: &str, functions: &[Function], context: &Context<'_>) -> Vec<String> {
    let methods = functions.iter().filter(|function| function.owner.as_deref() == Some(ty) && function.receiver == Receiver::Shared).count();
    let producers = functions.iter().filter(|function| function.output.as_ref().is_some_and(|output| returns(output, ty, context.renderer))).count();
    let mut evidence = vec![format!("&self methods: {methods}"), format!("returned by: {producers}")];
    if context.implementations.has_lifetime(ty) {
        evidence.push("borrows a lifetime: needs a key".to_owned());
    }
    let shared = context.implementations.implements(ty, SEND) && context.implementations.implements(ty, SYNC);
    evidence.push(if shared { "Send + Sync" } else { "not Send + Sync" }.to_owned());
    evidence
}

/// Whether `output`, through `Result` and `Option`, is the type at `ty`.
fn returns(output: &Type, ty: &str, renderer: &Renderer) -> bool {
    let Type::ResolvedPath(path) = output else { return false };
    let canonical = renderer.path(path);
    canonical == ty
        || (matches!(canonical.as_str(), "core::result::Result" | "core::option::Option")
            && type_args(path).first().is_some_and(|inner| returns(inner, ty, renderer)))
}

/// The shortest public path of each canonical path.
fn public_names(public: &HashMap<String, String>) -> HashMap<&str, String> {
    let mut names: HashMap<&str, String> = HashMap::new();
    for (path, canonical) in public {
        let name = names.entry(canonical.as_str()).or_insert_with(|| path.clone());
        if (path.len(), path.as_str()) < (name.len(), name.as_str()) {
            name.clone_from(path);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::render::tests::resolved;

    fn renderer() -> Renderer {
        Renderer::from_paths([(1, "demo::Repo"), (2, "demo::inner::Name"), (3, "demo::Blob"), (4, "demo::Commit"), (5, "core::result::Result"), (6, "demo::Error")])
    }

    fn function(path: &str, owner: Option<&str>, receiver: Receiver, params: Vec<(&str, Type)>, output: Option<Type>) -> Function {
        Function {
            path: path.to_owned(),
            name: path.rsplit("::").next().unwrap_or(path).to_owned(),
            owner: owner.map(str::to_owned),
            owner_public: owner.map(str::to_owned),
            receiver,
            params: params.into_iter().map(|(name, ty)| (name.to_owned(), ty)).collect(),
            output,
            generics: BTreeMap::new(),
            deprecated: false,
            is_async: false,
            is_unsafe: false,
        }
    }

    #[test]
    fn proposals_are_ranked_by_what_they_unblock() {
        let name = || resolved(2, Vec::new());
        let functions = vec![
            function("demo::parse", None, Receiver::None, vec![("name", name())], None),
            function("demo::Repo::blob", Some("demo::Repo"), Receiver::Shared, vec![("name", name())], Some(resolved(3, Vec::new()))),
            function("demo::Commit::name", Some("demo::Commit"), Receiver::Shared, Vec::new(), Some(name())),
            function("demo::commit", None, Receiver::None, Vec::new(), Some(resolved(4, Vec::new()))),
        ];
        let renderer = renderer();
        let implementations = Implementations::from_pairs([
            (DISPLAY, "demo::inner::Name"),
            (FROM_STR, "demo::inner::Name"),
            (AS_REF_BYTES, "demo::Blob"),
            (SEND, "demo::Commit"),
            (SYNC, "demo::Commit"),
        ])
        .with_facts([], &["demo::Commit"]);
        let type_map = BTreeMap::from([("demo::Repo".to_owned(), Mapping::Handle)]);
        let classifier = Classifier::new(&renderer, &type_map, &[], &implementations);
        let public = HashMap::from([("demo::Name".to_owned(), "demo::inner::Name".to_owned())]);
        let pending: HashSet<&str> = functions.iter().map(|function| function.path.as_str()).collect();
        let context = Context { renderer: &renderer, classifier: &classifier, implementations: &implementations, public: &public };
        let proposals = propose(&functions, &pending, &context);

        let entries: Vec<(&str, &str, usize, usize)> =
            proposals.entries.iter().map(|proposal| (proposal.ty.as_str(), proposal.mapping.as_str(), proposal.unblocks, proposal.reduces)).collect();
        assert_eq!(entries, [("demo::Name", "string", 1, 3), ("demo::Blob", "bytes", 0, 1)]);
        assert_eq!(proposals.entries[0].directions, ["in", "out"]);
        assert_eq!(proposals.entries[0].evidence, ["Display", "FromStr"]);
        assert_eq!(proposals.entries[1].directions, ["out"]);

        let [commit] = proposals.handles.as_slice() else { panic!("one handle candidate: {:?}", proposals.handles) };
        assert_eq!((commit.ty.as_str(), commit.mapping.as_str(), commit.unblocks, commit.reduces), ("demo::Commit", "handle", 1, 2));
        assert_eq!(commit.evidence, ["&self methods: 1", "returned by: 1", "borrows a lifetime: needs a key", "Send + Sync"]);
    }

    #[test]
    fn bytes_come_from_conversions_and_deref_targets() {
        let renderer = renderer();
        let implementations = Implementations::from_pairs([(FROM_BYTES, "&demo::Blob")]).with_facts([("demo::Blob", "[u8]")], &[]);
        let type_map = BTreeMap::new();
        let classifier = Classifier::new(&renderer, &type_map, &[], &implementations);
        let public = HashMap::new();
        let context = Context { renderer: &renderer, classifier: &classifier, implementations: &implementations, public: &public };
        let (directions, evidence) = byte_like("demo::Blob", &context).expect("byte-like");
        assert_eq!(directions, ["in", "out"]);
        assert_eq!(evidence, ["From<&[u8]>", "Deref<Target = [u8]>"]);
        assert!(string_like("demo::Blob", &implementations).is_none());
    }
}