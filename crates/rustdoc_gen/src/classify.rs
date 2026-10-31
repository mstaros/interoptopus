//! Sorts each surface function into automatic, needs-a-decision or excluded (design sections
//! "Classification (v1)" and "Type map"). The engine knows only std types: everything else comes
//! from the manifest's type map, and generic bounds are matched against it and against the trait
//! implementations rustdoc reports.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rustdoc_types::{
    AssocItemConstraintKind, Attribute, Crate, Enum, GenericArg, GenericArgs, GenericBound, GenericParamDefKind, Id, Impl, Item, ItemEnum,
    ItemKind, Path, Struct, StructKind, Term, Type, Union, VariantKind,
};
use serde::{Deserialize, Serialize};

use crate::render::{Renderer, type_args};
use crate::surface::{self, Function, Receiver};

/// How a mapped Rust type crosses the boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mapping {
    /// UTF-8 text.
    String,
    /// Bytes.
    Bytes,
    /// A filesystem path, converted by the runtime's `path` and `path_out`.
    Path,
    /// The runtime handle, held by the hand-written runtime.
    Handle,
    /// A type the runtime converts with the two functions the mapping names.
    Custom(Custom),
    /// A type that crosses as its key and is rebuilt from a handle on every call.
    Keyed(Keyed),
}

impl Mapping {
    /// Whether values of the type cross as data (text, bytes, a path or a custom value) rather than
    /// as a handle.
    #[must_use]
    pub const fn is_data(&self) -> bool {
        !matches!(self, Self::Handle | Self::Keyed(_))
    }
}

/// A mapping whose conversions the runtime provides.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Custom {
    /// How the value crosses the boundary.
    pub ffi: Carrier,
    /// The runtime function that turns a reference to the FFI value into the Rust value, or
    /// fails with the runtime error.
    pub into_rust: String,
    /// The runtime function that turns a reference to the Rust value into the FFI value.
    pub into_ffi: String,
}

/// A type that crosses as its key: every call rebuilds it from the handle `handle` names and the
/// key (see the design doc's Handles section).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyed {
    /// The handle type `{handle}` refers to; it must be mapped as `handle`.
    pub handle: String,
    /// The key's type: a primitive, or a type with a mapping that is neither a handle nor keyed.
    pub key: String,
    /// Rust expression rebuilding the value from `{handle}`, a reference to the handle, and `{key}`.
    pub rebuild: String,
    /// Whether `rebuild` returns a `Result`.
    #[serde(default)]
    pub fallible: bool,
    /// Rust expression producing the key from `{value}`.
    pub key_of: String,
    /// The type's path as the type map writes it; set when the type map is resolved.
    #[serde(skip)]
    pub name: String,
}

/// The placeholders of a keyed payload: the handle reference, the key and the value.
pub const HANDLE_PLACEHOLDER: &str = "{handle}";
/// See [`HANDLE_PLACEHOLDER`].
pub const KEY_PLACEHOLDER: &str = "{key}";
/// See [`HANDLE_PLACEHOLDER`].
pub const VALUE_PLACEHOLDER: &str = "{value}";

/// The FFI form of a custom mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Carrier {
    /// Text: `ffi::String`.
    String,
    /// Bytes: `ffi::Slice<u8>` in, `ffi::Vec<u8>` out.
    Bytes,
}

/// How a generic parameter is satisfied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversion {
    /// The mapping of the value passed.
    pub mapping: Mapping,
    /// Whether a reference is passed rather than an owned value.
    pub borrowed: bool,
}

/// The std types the engine maps itself; the manifest's type map extends and overrides them.
const STD_TYPES: [(&str, Mapping); 3] =
    [("alloc::string::String", Mapping::String), ("std::path::Path", Mapping::Path), ("std::path::PathBuf", Mapping::Path)];

/// Primitives that cross the boundary unchanged.
const PRIMITIVES: [&str; 13] = ["bool", "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64"];

/// Traits satisfied by any value convertible to their type argument; the first two take a
/// reference.
const CONVERTING: [&str; 4] = ["core::convert::AsRef", "core::borrow::Borrow", "core::convert::Into", "core::convert::TryInto"];

/// The iterator trait; results implementing it wait for a cursor template.
const ITERATOR: &str = "core::iter::traits::iterator::Iterator";
const DEREF: &str = "core::ops::deref::Deref";

/// `Display`: a type mapped to `string` crosses out as its display text.
pub const DISPLAY: &str = "core::fmt::Display";
/// `FromStr`: a type mapped to `string` crosses in by parsing.
pub const FROM_STR: &str = "core::str::traits::FromStr";
/// `TryFrom<&str>`: a type mapped to `string` crosses in through `TryFrom`.
pub const TRY_FROM_STR: &str = "core::convert::TryFrom<&str>";
/// `AsRef<[u8]>`: a type mapped to `bytes` crosses out as a byte view.
pub const AS_REF_BYTES: &str = "core::convert::AsRef<[u8]>";
/// `From<&[u8]>`: a type mapped to `bytes` crosses in from a byte slice.
pub const FROM_BYTES: &str = "core::convert::From<&[u8]>";
/// `From<Vec<u8>>`: a type mapped to `bytes` crosses in from owned bytes.
pub const FROM_BYTE_VEC: &str = "core::convert::From<alloc::vec::Vec<u8>>";

/// The class of a surface function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// Generated without a decision.
    Automatic,
    /// Needs a manifest decision.
    Decision,
    /// Left out by rule.
    Excluded,
}

/// Why a function is not automatic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// The item is deprecated.
    Deprecated,
    /// The item is under a configured excluded path.
    ExcludedPath,
    /// The receiver is not a runtime handle.
    Receiver,
    /// `&mut self` on the runtime handle needs the persistent runtime holder.
    MutHandle,
    /// A by-value method of a keyed type that returns the type: its key cannot carry the change.
    Builder,
    /// A type is not in the type map.
    Unmapped,
    /// A borrowed type that cannot be copied across.
    Borrowed,
    /// A generic parameter or result without a conversion.
    Generic,
    /// A result implementing `Iterator`, until a cursor template exists.
    Iterator,
    /// A callback parameter.
    Callback,
    /// A cancellation flag.
    Cancellation,
    /// A reader or writer.
    Io,
    /// A construct the engine does not support, such as `async` or raw pointers.
    Unsupported,
    /// The classifier passes the item, but the emitter does not generate it yet.
    Lowering,
}

impl Category {
    /// The `snake_case` name used in the manifest.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Deprecated => "deprecated",
            Self::ExcludedPath => "excluded_path",
            Self::Receiver => "receiver",
            Self::MutHandle => "mut_handle",
            Self::Builder => "builder",
            Self::Unmapped => "unmapped",
            Self::Borrowed => "borrowed",
            Self::Generic => "generic",
            Self::Iterator => "iterator",
            Self::Callback => "callback",
            Self::Cancellation => "cancellation",
            Self::Io => "io",
            Self::Unsupported => "unsupported",
            Self::Lowering => "lowering",
        }
    }
}

/// The category and detail that keep a function from being automatic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reason {
    /// What kind of obstacle this is.
    pub category: Category,
    /// The offending type, parameter or rule.
    pub detail: String,
    /// The canonical path of the type the obstacle is about, when it is one; proposals start
    /// from it.
    pub ty: Option<String>,
}

impl Reason {
    fn new(category: Category, detail: impl Into<String>) -> Self {
        Self { category, detail: detail.into(), ty: None }
    }

    /// An item the emitter does not generate, with the emitter's reason.
    #[must_use]
    pub fn lowering(detail: impl Into<String>) -> Self {
        Self::new(Category::Lowering, detail)
    }

    fn about(self, ty: Option<&str>) -> Self {
        Self { ty: ty.map(str::to_owned), ..self }
    }

    fn in_parameter(self, name: &str) -> Self {
        Self { detail: format!("parameter `{name}`: {}", self.detail), ..self }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.category.name(), self.detail)
    }
}

/// The classification of one function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// The class.
    pub class: Class,
    /// Why the function is not automatic: every blocker, the receiver first, then the parameters
    /// in order, then the result.
    pub reasons: Vec<Reason>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    Input,
    Output,
}

/// A public enum that can cross as an `#[ffi]` mirror (the enum kind in `facade.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumDef {
    /// Its public path in its own crate, such as `demo_object::Kind`.
    pub public: String,
    /// Its variants, in declaration order.
    pub variants: Vec<VariantDef>,
    /// Whether another crate must match it with a wildcard arm: it is `#[non_exhaustive]` or
    /// hides variants.
    pub open: bool,
    /// Whether it has type or const parameters.
    pub generic: bool,
}

/// One variant of an enum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantDef {
    /// The variant's name.
    pub name: String,
    /// The evaluated discriminant, when the source writes one.
    pub discriminant: Option<String>,
    /// Its fields. Payload types name items by canonical path under an id no renderer knows, so
    /// any crate's renderer reads them.
    pub fields: VariantFields,
    /// Whether another crate may construct it: `#[non_exhaustive]` on a variant forbids it.
    pub constructible: bool,
}

/// The fields of a variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VariantFields {
    /// `A`.
    Unit,
    /// `A(..)`, possibly empty.
    Tuple(Vec<Type>),
    /// `A { .. }`, possibly empty.
    Named(Vec<(String, Type)>),
    /// Fields hidden from the documentation.
    Hidden,
}

impl VariantFields {
    /// The payload types, in declaration order.
    #[must_use]
    pub fn types(&self) -> Vec<&Type> {
        match self {
            Self::Unit | Self::Hidden => Vec::new(),
            Self::Tuple(types) => types.iter().collect(),
            Self::Named(fields) => fields.iter().map(|(_, ty)| ty).collect(),
        }
    }

    /// Whether the variant carries no data.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Unit => true,
            Self::Tuple(types) => types.is_empty(),
            Self::Named(fields) => fields.is_empty(),
            Self::Hidden => false,
        }
    }
}

/// A public struct whose named fields are all public, which can cross as an `#[ffi]` mirror (the
/// record kind in `facade.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructDef {
    /// Its public path in its own crate, such as `demo::create::Options`.
    pub public: String,
    /// Its fields in declaration order, typed like `VariantDef::fields`.
    pub fields: Vec<(String, Type)>,
    /// Whether it is declared with a lifetime: a borrowed view, which crosses as a result only.
    pub borrows: bool,
    /// Whether another crate may build it with a struct literal: it is not `#[non_exhaustive]`.
    pub constructible: bool,
    /// Whether it has type or const parameters.
    pub generic: bool,
}

/// The detail for an enum or struct without a mirror name.
const UNNAMED: &str = "not nameable through the root crate, or another type claims its mirror name";

/// An id no renderer knows: a path under it renders as its written, canonical text.
const UNKNOWN_ID: Id = Id(u32::MAX);

/// A type alias: its type parameters with their defaults, and the type it stands for, written
/// like `StructDef::fields`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Alias {
    params: Vec<(String, Option<Type>)>,
    target: Type,
}

/// The canonical paths of std's `io::Result`.
const IO_RESULT: [&str; 2] = ["core::io::error::Result", "std::io::error::Result"];
/// Trait implementations listed in rustdoc JSON, plus the type facts proposals need.
///
/// Each trait keeps its implementing types as the renderer writes them. Blanket and negative
/// implementations are left out.
#[derive(Debug, Default)]
pub struct Implementations {
    by_trait: BTreeMap<String, BTreeSet<String>>,
    /// Trait references with their arguments, such as `core::convert::AsRef<[u8]>`.
    by_reference: BTreeMap<String, BTreeSet<String>>,
    /// `Deref` targets by implementing type.
    deref_targets: BTreeMap<String, String>,
    /// Types declared with a lifetime parameter.
    with_lifetime: BTreeSet<String>,
    /// The crates whose rustdoc JSON was added, so their trait implementations are known.
    crates: BTreeSet<String>,
    /// Public enums by canonical path.
    enums: BTreeMap<String, EnumDef>,
    /// Public structs whose named fields are all public, by canonical path.
    structs: BTreeMap<String, StructDef>,
    /// The root crate's name.
    root: Option<String>,
    /// For each crate, the crates it re-exports as modules, with the shortest path in the
    /// re-exporting crate: `pub use demo_hash as hash`, or a glob such as `pub use demo_diff::*`.
    reexports: BTreeMap<String, BTreeMap<String, String>>,
    /// The public path through the root crate of each crate it reaches through re-exports.
    crate_paths: BTreeMap<String, String>,
    /// Mirror names by canonical path; types whose name another type also claims have none.
    mirrors: BTreeMap<String, String>,
    /// Type aliases by canonical path.
    aliases: BTreeMap<String, Alias>,
}

impl Implementations {
    /// Adds the trait implementations, lifetime-generic types, public enums and structs, and the
    /// crates re-exported as modules that one crate's JSON lists.
    pub fn add(&mut self, krate: &Crate) {
        let renderer = Renderer::new(krate);
        let public_paths = surface::public_paths(krate);
        let name = krate.index.get(&krate.root).and_then(|root| root.name.clone()).unwrap_or_default();
        self.crates.insert(name.clone());
        for (id, public) in &public_paths {
            if let Some(summary) = krate.paths.get(id)
                && let [target] = summary.path.as_slice()
                && summary.crate_id != 0
                && summary.kind == ItemKind::Module
            {
                let entry = self.reexports.entry(name.clone()).or_default().entry(target.clone()).or_insert_with(|| public.clone());
                if (public.len(), public) < (entry.len(), &*entry) {
                    entry.clone_from(public);
                }
            }
        }
        for item in krate.index.values() {
            let local = krate.paths.get(&item.id).filter(|summary| summary.crate_id == 0).zip(public_paths.get(&item.id));
            match (&item.inner, local) {
                (ItemEnum::Enum(enumeration), Some((summary, public))) => {
                    self.enums.insert(summary.path.join("::"), enum_def(krate, &renderer, item, enumeration, public));
                }
                (ItemEnum::Struct(structure), Some((summary, public))) => {
                    if let Some(def) = struct_def(krate, &renderer, item, structure, public) {
                        self.structs.insert(summary.path.join("::"), def);
                    }
                }
                (ItemEnum::TypeAlias(alias), Some((summary, _))) => {
                    let params = alias
                        .generics
                        .params
                        .iter()
                        .filter_map(|param| match &param.kind {
                            GenericParamDefKind::Type { default, .. } => Some((param.name.clone(), default.as_ref().map(|ty| portable(ty, &renderer)))),
                            _ => None,
                        })
                        .collect();
                    self.aliases.insert(summary.path.join("::"), Alias { params, target: portable(&alias.type_, &renderer) });
                }
                _ => {}
            }
            match &item.inner {
                ItemEnum::Impl(implementation) => self.implementation(krate, &renderer, implementation),
                ItemEnum::Struct(Struct { generics, .. }) | ItemEnum::Enum(Enum { generics, .. }) | ItemEnum::Union(Union { generics, .. }) => {
                    if generics.params.iter().any(|param| matches!(param.kind, GenericParamDefKind::Lifetime { .. }))
                        && let Some(summary) = krate.paths.get(&item.id)
                    {
                        self.with_lifetime.insert(summary.path.join("::"));
                    }
                }
                _ => {}
            }
        }
    }

    fn implementation(&mut self, krate: &Crate, renderer: &Renderer, implementation: &Impl) {
        let Some(trait_) = &implementation.trait_ else { return };
        if implementation.is_negative || matches!(implementation.for_, Type::Generic(_)) {
            return;
        }
        let implementer = renderer.ty(&implementation.for_);
        let trait_path = renderer.path(trait_);
        if trait_path == DEREF {
            let target = implementation.items.iter().filter_map(|id| krate.index.get(id)).find_map(|item| match &item.inner {
                ItemEnum::AssocType { type_: Some(target), .. } if item.name.as_deref() == Some("Target") => Some(renderer.ty(target)),
                _ => None,
            });
            if let Some(target) = target {
                self.deref_targets.insert(base(&implementer).to_owned(), target);
            }
        }
        self.by_reference.entry(renderer.ty(&Type::ResolvedPath(trait_.clone()))).or_default().insert(implementer.clone());
        self.by_trait.entry(trait_path).or_default().insert(implementer);
    }

    /// The types implementing a trait.
    pub fn implementers(&self, trait_path: &str) -> impl Iterator<Item = &str> {
        self.by_trait.get(trait_path).into_iter().flatten().map(String::as_str)
    }

    /// Whether the type at `path`, with any generic arguments, implements the trait.
    #[must_use]
    pub fn implements(&self, path: &str, trait_path: &str) -> bool {
        self.implementers(trait_path).any(|implementer| base(implementer) == path)
    }

    /// Whether the type at `path`, or a borrow of it, implements a trait reference with its
    /// arguments, such as `core::convert::AsRef<[u8]>`.
    #[must_use]
    pub fn implements_reference(&self, path: &str, reference: &str) -> bool {
        self.by_reference.get(reference).into_iter().flatten().any(|implementer| base(implementer) == path)
    }

    /// Whether `implementer`, exactly as the renderer writes it (`&bstr::BStr` is not
    /// `bstr::BStr`), implements a trait reference with its arguments.
    #[must_use]
    pub fn implements_exactly(&self, implementer: &str, reference: &str) -> bool {
        self.by_reference.get(reference).is_some_and(|implementers| implementers.contains(implementer))
    }

    /// Whether the crate defining `path` was documented, so its trait implementations are known.
    #[must_use]
    pub fn documents(&self, path: &str) -> bool {
        path.split("::").next().is_some_and(|krate| self.crates.contains(krate))
    }

    /// The `Deref` target of the type at `path`, as the renderer writes it.
    #[must_use]
    pub fn deref_target(&self, path: &str) -> Option<&str> {
        self.deref_targets.get(path).map(String::as_str)
    }

    /// Whether the type at `path` is declared with a lifetime parameter.
    #[must_use]
    pub fn has_lifetime(&self, path: &str) -> bool {
        self.with_lifetime.contains(path)
    }

    /// Adds the root crate: what `add` collects, plus its name, through which other crates' types
    /// are named.
    pub fn add_root(&mut self, krate: &Crate) {
        self.add(krate);
        self.root = krate.index.get(&krate.root).and_then(|root| root.name.clone());
    }

    /// Resolves the paths through the root crate and names every mirror, once all crates are
    /// added; two types claiming one name both go without.
    pub fn finish(&mut self) {
        self.resolve_fields();
        self.crate_paths = self.crate_prefixes();
        let mut claims: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for canonical in self.enums.keys().chain(self.structs.keys()) {
            if let Some(name) = self.nameable(canonical).as_deref().and_then(mirror_name) {
                claims.entry(name).or_default().push(canonical.clone());
            }
        }
        self.mirrors = claims.into_iter().filter_map(|(name, owners)| match owners.as_slice() {
            [owner] => Some((owner.clone(), name)),
            _ => None,
        }).collect();
    }

    /// `ty` with every type alias replaced by the type it stands for, its arguments or their
    /// defaults substituted: the aliases of the documented crates, and std's `io::Result`.
    #[must_use]
    pub fn resolve(&self, ty: &Type, renderer: &Renderer) -> Type {
        match ty {
            Type::ResolvedPath(path) => {
                let path = map_args(path, &|arg| self.resolve(arg, renderer));
                let canonical = renderer.path(&path);
                let Some(alias) = self.aliases.get(&canonical).cloned().or_else(|| io_result(&canonical)) else {
                    return Type::ResolvedPath(path);
                };
                let args = type_args(&path);
                let bindings = alias
                    .params
                    .iter()
                    .enumerate()
                    .filter_map(|(index, (name, default))| Some((name.clone(), args.get(index).copied().cloned().or_else(|| default.clone())?)))
                    .collect();
                self.resolve(&substitute(&alias.target, &bindings), renderer)
            }
            other => map_children(other, &|ty| self.resolve(ty, renderer)),
        }
    }

    /// Sees through the aliases in the enums' and structs' fields.
    fn resolve_fields(&mut self) {
        let renderer = Renderer::default();
        let mut structs = std::mem::take(&mut self.structs);
        for (_, ty) in structs.values_mut().flat_map(|def| def.fields.iter_mut()) {
            *ty = self.resolve(ty, &renderer);
        }
        self.structs = structs;
        let mut enums = std::mem::take(&mut self.enums);
        for variant in enums.values_mut().flat_map(|def| def.variants.iter_mut()) {
            match &mut variant.fields {
                VariantFields::Tuple(types) => types.iter_mut().for_each(|ty| *ty = self.resolve(ty, &renderer)),
                VariantFields::Named(fields) => fields.iter_mut().for_each(|(_, ty)| *ty = self.resolve(ty, &renderer)),
                VariantFields::Unit | VariantFields::Hidden => {}
            }
        }
        self.enums = enums;
    }

    /// The public path through the root crate of every crate it reaches through re-exports,
    /// shortest first: directly (`demo_hash` at `demo::hash`), through a glob (`demo::diff`
    /// glob-imports `demo_diff`) or through another crate (`demo_diff::blob` glob-imports
    /// `demo_text_diff`, so `demo` reaches it at `demo::diff::blob`).
    fn crate_prefixes(&self) -> BTreeMap<String, String> {
        let Some(root) = &self.root else { return BTreeMap::new() };
        let mut prefixes = BTreeMap::from([(root.clone(), root.clone())]);
        let mut changed = true;
        while changed {
            changed = false;
            for (from, targets) in &self.reexports {
                let Some(prefix) = prefixes.get(from).cloned() else { continue };
                for (target, public) in targets {
                    let Some(rest) = public.strip_prefix(from.as_str()).filter(|rest| rest.is_empty() || rest.starts_with("::")) else {
                        continue;
                    };
                    let candidate = format!("{prefix}{rest}");
                    if prefixes.get(target).is_none_or(|known| (candidate.len(), &candidate) < (known.len(), known)) {
                        prefixes.insert(target.clone(), candidate);
                        changed = true;
                    }
                }
            }
        }
        prefixes.remove(root);
        prefixes
    }

    /// The public enum at `canonical`.
    #[must_use]
    pub fn enum_def(&self, canonical: &str) -> Option<&EnumDef> {
        self.enums.get(canonical)
    }

    /// The public struct at `canonical`, if its named fields are all public.
    #[must_use]
    pub fn struct_def(&self, canonical: &str) -> Option<&StructDef> {
        self.structs.get(canonical)
    }

    /// The mirror name of the enum or struct at `canonical`, if it has one.
    #[must_use]
    pub fn mirror(&self, canonical: &str) -> Option<&str> {
        self.mirrors.get(canonical).map(String::as_str)
    }

    /// The path that names the enum or struct at `canonical` through the root crate's public API.
    #[must_use]
    pub fn nameable(&self, canonical: &str) -> Option<String> {
        let public = self.enums.get(canonical).map(|def| &def.public).or_else(|| self.structs.get(canonical).map(|def| &def.public))?;
        let (krate, rest) = public.split_once("::")?;
        if Some(krate) == self.root.as_deref() {
            return Some(public.clone());
        }
        self.crate_paths.get(krate).map(|prefix| format!("{prefix}::{rest}"))
    }

    /// Implementations from (trait, implementing type) pairs.
    #[cfg(test)]
    pub(crate) fn from_pairs<'p>(pairs: impl IntoIterator<Item = (&'p str, &'p str)>) -> Self {
        let mut implementations = Self::default();
        for (reference, implementer) in pairs {
            let trait_path = reference.split('<').next().unwrap_or(reference);
            implementations.by_trait.entry(trait_path.to_owned()).or_default().insert(implementer.to_owned());
            implementations.by_reference.entry(reference.to_owned()).or_default().insert(implementer.to_owned());
        }
        implementations
    }

    /// Records a `Deref` target and the types declared with a lifetime.
    #[cfg(test)]
    pub(crate) fn with_facts<'p>(mut self, derefs: impl IntoIterator<Item = (&'p str, &'p str)>, lifetimes: &[&str]) -> Self {
        self.deref_targets.extend(derefs.into_iter().map(|(path, target)| (path.to_owned(), target.to_owned())));
        self.with_lifetime.extend(lifetimes.iter().map(|path| (*path).to_owned()));
        self
    }

    /// Marks a crate as documented.
    #[cfg(test)]
    pub(crate) fn with_crate(mut self, name: &str) -> Self {
        self.crates.insert(name.to_owned());
        self
    }

    /// Sets the root crate's name.
    #[cfg(test)]
    pub(crate) fn with_root(mut self, root: &str) -> Self {
        self.root = Some(root.to_owned());
        self
    }

    /// Adds a type alias and sees through the aliases again.
    #[cfg(test)]
    pub(crate) fn with_alias(mut self, canonical: &str, params: Vec<(&str, Option<Type>)>, target: Type) -> Self {
        let params = params.into_iter().map(|(name, default)| (name.to_owned(), default)).collect();
        self.aliases.insert(canonical.to_owned(), Alias { params, target });
        self.finish();
        self
    }

    /// Adds an enum and names the mirrors again.
    #[cfg(test)]
    pub(crate) fn with_enum(mut self, canonical: &str, def: EnumDef) -> Self {
        self.enums.insert(canonical.to_owned(), def);
        self.finish();
        self
    }

    /// Adds a struct and names the mirrors again.
    #[cfg(test)]
    pub(crate) fn with_struct(mut self, canonical: &str, def: StructDef) -> Self {
        self.structs.insert(canonical.to_owned(), def);
        self.finish();
        self
    }

    /// Records that crate `from` re-exports crate `target` at `public`, and names the mirrors
    /// again.
    #[cfg(test)]
    pub(crate) fn with_reexport(mut self, from: &str, target: &str, public: &str) -> Self {
        self.reexports.entry(from.to_owned()).or_default().insert(target.to_owned(), public.to_owned());
        self.finish();
        self
    }
}

/// Classifies surface functions against a type map and exclusion rules.
pub struct Classifier<'a> {
    renderer: &'a Renderer,
    types: BTreeMap<String, Mapping>,
    exclude_paths: &'a [String],
    implementations: &'a Implementations,
    /// The enums whose payloads are being checked, against recursion.
    visiting: RefCell<Vec<String>>,
}

impl<'a> Classifier<'a> {
    /// A classifier using the std type map extended by `overrides`.
    #[must_use]
    pub fn new(
        renderer: &'a Renderer,
        overrides: &BTreeMap<String, Mapping>,
        exclude_paths: &'a [String],
        implementations: &'a Implementations,
    ) -> Self {
        let mut types: BTreeMap<String, Mapping> = STD_TYPES.iter().map(|(path, mapping)| ((*path).to_owned(), mapping.clone())).collect();
        types.extend(overrides.iter().map(|(path, mapping)| (path.clone(), mapping.clone())));
        Self { renderer, types, exclude_paths, implementations, visiting: RefCell::default() }
    }

    /// This classifier with one more type-map entry, for measuring a proposal.
    #[must_use]
    pub fn with(&self, canonical: &str, mapping: Mapping) -> Self {
        let mut types = self.types.clone();
        types.insert(canonical.to_owned(), mapping);
        Self { renderer: self.renderer, types, exclude_paths: self.exclude_paths, implementations: self.implementations, visiting: RefCell::default() }
    }

    /// The trait implementations and type facts this classifier reads.
    #[must_use]
    pub const fn implementations(&self) -> &'a Implementations {
        self.implementations
    }

    /// The verdict for one function.
    #[must_use]
    pub fn classify(&self, function: &Function) -> Verdict {
        if function.deprecated {
            return Verdict { class: Class::Excluded, reasons: vec![Reason::new(Category::Deprecated, "deprecated")] };
        }
        if let Some(prefix) = self.exclude_paths.iter().find(|prefix| is_under(&function.path, prefix)) {
            return Verdict { class: Class::Excluded, reasons: vec![Reason::new(Category::ExcludedPath, prefix.clone())] };
        }
        let reasons = self.blockers(&self.transparent(function));
        let class = if reasons.is_empty() { Class::Automatic } else { Class::Decision };
        Verdict { class, reasons }
    }

    /// `function` with its parameter and result types seen through aliases (see
    /// [`Implementations::resolve`]).
    #[must_use]
    pub fn transparent(&self, function: &Function) -> Function {
        let mut function = function.clone();
        for (_, ty) in &mut function.params {
            *ty = self.implementations.resolve(ty, self.renderer);
        }
        function.output = function.output.map(|output| self.implementations.resolve(&output, self.renderer));
        function
    }

    /// The mapping of a canonical type path, if any.
    #[must_use]
    pub fn mapping(&self, canonical: &str) -> Option<Mapping> {
        self.types.get(canonical).cloned()
    }

    /// Checks the type map's keyed mappings: their placeholders, a handle behind `handle`, and a
    /// primitive key or one with a mapping that is neither a handle nor keyed (E4).
    ///
    /// # Errors
    ///
    /// The first malformed entry, with the reason.
    pub fn validate(&self) -> Result<(), String> {
        for (path, mapping) in &self.types {
            let Mapping::Keyed(keyed) = mapping else { continue };
            let problem = if !keyed.rebuild.contains(HANDLE_PLACEHOLDER) || !keyed.rebuild.contains(KEY_PLACEHOLDER) {
                Some("`rebuild` needs `{handle}` and `{key}`".to_owned())
            } else if !keyed.key_of.contains(VALUE_PLACEHOLDER) {
                Some("`key_of` needs `{value}`".to_owned())
            } else if self.types.get(&keyed.handle) != Some(&Mapping::Handle) {
                Some(format!("`handle` names `{}`, which is not mapped as `handle`", keyed.handle))
            } else if !PRIMITIVES.contains(&keyed.key.as_str()) && !self.types.get(&keyed.key).is_some_and(Mapping::is_data) {
                Some(format!("`key` names `{}`, which needs a primitive type or a mapping that is neither a handle nor keyed", keyed.key))
            } else {
                None
            };
            if let Some(problem) = problem {
                return Err(format!("type map entry `{path}`: {problem}"));
            }
        }
        Ok(())
    }

    /// Canonical paths of the types mapped to the runtime handle.
    pub fn handles(&self) -> impl Iterator<Item = &str> {
        self.types.iter().filter(|(_, mapping)| **mapping == Mapping::Handle).map(|(path, _)| path.as_str())
    }

    /// How a generic parameter's bounds are satisfied: a converting trait whose target is mapped,
    /// or a trait rustdoc lists a mapped type as implementing.
    #[must_use]
    pub fn conversion(&self, bounds: &[GenericBound]) -> Option<Conversion> {
        bounds.iter().find_map(|bound| {
            let GenericBound::TraitBound { trait_, .. } = bound else { return None };
            let canonical = self.renderer.path(trait_);
            if CONVERTING.contains(&canonical.as_str()) {
                let arguments = type_args(trait_);
                let [target] = arguments.as_slice() else { return None };
                let borrowed = CONVERTING[..2].contains(&canonical.as_str());
                return self.carrier(target).map(|mapping| Conversion { mapping, borrowed });
            }
            self.implemented(&canonical)
        })
    }

    /// The mapping of the type a converting bound targets.
    fn carrier(&self, target: &Type) -> Option<Mapping> {
        match target {
            Type::Primitive(name) if name == "str" => Some(Mapping::String),
            Type::Slice(element) if is_byte(element) => Some(Mapping::Bytes),
            Type::ResolvedPath(path) => {
                let canonical = self.renderer.path(path);
                match (canonical.as_str(), type_args(path).as_slice()) {
                    ("alloc::vec::Vec", [Type::Primitive(byte)]) if byte == "u8" => Some(Mapping::Bytes),
                    _ => self.types.get(&canonical).filter(|mapping| mapping.is_data()).cloned(),
                }
            }
            _ => None,
        }
    }

    /// The best conversion through a type rustdoc lists as implementing `trait_path`: borrowed std
    /// carriers first, then owned ones, then mapped types.
    fn implemented(&self, trait_path: &str) -> Option<Conversion> {
        self.implementations
            .implementers(trait_path)
            .filter_map(|implementer| self.implementer(implementer))
            .min_by_key(|(rank, _)| *rank)
            .map(|(_, conversion)| conversion)
    }

    /// The conversion through one implementing type, with its rank.
    fn implementer(&self, implementer: &str) -> Option<(u8, Conversion)> {
        let carrier = |mapping: Mapping, borrowed: bool| Some((u8::from(!borrowed), Conversion { mapping, borrowed }));
        match implementer {
            "str" | "&str" => carrier(Mapping::String, true),
            "[u8]" | "&[u8]" => carrier(Mapping::Bytes, true),
            "alloc::string::String" => carrier(Mapping::String, false),
            "alloc::vec::Vec<u8>" => carrier(Mapping::Bytes, false),
            _ => self
                .types
                .get(implementer)
                .filter(|mapping| mapping.is_data())
                .map(|mapping| (2, Conversion { mapping: mapping.clone(), borrowed: false })),
        }
    }

    /// Every blocker: the receiver, then each parameter in order, then the result.
    fn blockers(&self, function: &Function) -> Vec<Reason> {
        if function.is_async || function.is_unsafe {
            return vec![Reason::new(Category::Unsupported, if function.is_async { "async" } else { "unsafe" })];
        }
        let receiver = self.receiver(function).err().or_else(|| self.builder(function));
        let parameters =
            function.params.iter().filter_map(|(name, ty)| self.input(ty, function).err().map(|reason| reason.in_parameter(name)));
        let result = function.output.as_ref().and_then(|output| self.value(output, function, Position::Output).err());
        receiver.into_iter().chain(parameters).chain(result).collect()
    }

    /// A by-value method of a keyed type that returns the type would lose its change.
    fn builder(&self, function: &Function) -> Option<Reason> {
        let owner = function.owner.as_deref()?;
        let keyed = matches!(self.owner_mapping(function), Some(Mapping::Keyed(_)));
        (keyed && function.receiver == Receiver::Value && function.output.as_ref().is_some_and(|output| self.returns_type(output, owner)))
            .then(|| Reason::new(Category::Builder, "returns a modified copy its key cannot carry").about(Some(owner)))
    }

    /// Whether `ty`, through `Result` and `Option`, is `Self` or the type at `path`.
    fn returns_type(&self, ty: &Type, path: &str) -> bool {
        match ty {
            Type::Generic(name) => name == "Self",
            Type::ResolvedPath(resolved) => {
                let canonical = self.renderer.path(resolved);
                canonical == path
                    || (matches!(canonical.as_str(), "core::result::Result" | "core::option::Option")
                        && type_args(resolved).first().is_some_and(|inner| self.returns_type(inner, path)))
            }
            _ => false,
        }
    }

    fn owner_mapping(&self, function: &Function) -> Option<Mapping> {
        [&function.owner, &function.owner_public].into_iter().flatten().find_map(|path| self.types.get(path).cloned())
    }

    fn receiver(&self, function: &Function) -> Result<(), Reason> {
        match (&function.receiver, self.owner_mapping(function)) {
            (Receiver::None, _) | (Receiver::Shared, Some(Mapping::Handle)) => Ok(()),
            (Receiver::Exclusive, Some(Mapping::Handle)) => {
                Err(Reason::new(Category::MutHandle, "&mut self needs the persistent runtime holder"))
            }
            (Receiver::Value | Receiver::Other(_), Some(Mapping::Handle)) => {
                Err(Reason::new(Category::Receiver, "takes the runtime handle by value or wrapped"))
            }
            (Receiver::Shared | Receiver::Value, Some(Mapping::Keyed(_))) => Ok(()),
            (Receiver::Exclusive, Some(Mapping::Keyed(_))) => {
                Err(Reason::new(Category::MutHandle, "&mut self on a keyed handle loses its changes"))
            }
            (Receiver::Other(_), Some(Mapping::Keyed(_))) => Err(Reason::new(Category::Receiver, "takes the keyed handle wrapped")),
            _ => Err(Reason::new(Category::Receiver, format!("{} is not a handle", function.owner.as_deref().unwrap_or("Self")))
                .about(function.owner.as_deref())),
        }
    }

    fn input(&self, ty: &Type, function: &Function) -> Result<(), Reason> {
        match ty {
            Type::ImplTrait(bounds) => self.convertible(bounds, &self.renderer.ty(ty), function),
            Type::Generic(name) if name != "Self" => {
                self.convertible(function.generics.get(name).map_or(&[], Vec::as_slice), name, function)
            }
            _ => self.value(ty, function, Position::Input),
        }
    }

    fn value(&self, ty: &Type, function: &Function, position: Position) -> Result<(), Reason> {
        match ty {
            Type::Primitive(name) if PRIMITIVES.contains(&name.as_str()) => Ok(()),
            Type::Tuple(types) if types.is_empty() => Ok(()),
            Type::Generic(name) if name == "Self" => self.owner_value(function),
            Type::ResolvedPath(path) => self.resolved(path, function, position),
            Type::BorrowedRef { is_mutable: false, type_, .. } => self.borrowed(type_, function, position),
            Type::BorrowedRef { type_, .. } => Err(self.blocked(type_, Category::Borrowed)),
            Type::ImplTrait(bounds) => Err(Reason::new(self.bounds_category(bounds, Category::Generic), self.renderer.ty(ty))),
            Type::DynTrait(_) => Err(self.blocked(ty, Category::Unsupported)),
            Type::Generic(name) => Err(Reason::new(Category::Generic, name.clone())),
            Type::FunctionPointer(_) => Err(Reason::new(Category::Callback, self.renderer.ty(ty))),
            _ => Err(Reason::new(Category::Unsupported, self.renderer.ty(ty))),
        }
    }

    fn resolved(&self, path: &Path, function: &Function, position: Position) -> Result<(), Reason> {
        let canonical = self.renderer.path(path);
        match (canonical.as_str(), type_args(path).as_slice()) {
            ("core::option::Option" | "alloc::boxed::Box", [inner]) => self.value(inner, function, position),
            ("alloc::vec::Vec", [Type::Primitive(byte)]) if byte == "u8" => Ok(()),
            ("alloc::vec::Vec", [inner]) => self.value(inner, function, position),
            ("core::result::Result", [ok, _]) if position == Position::Output => self.value(ok, function, position),
            ("core::result::Result", _) => Err(Reason::new(Category::Unsupported, "Result in a parameter")),
            ("alloc::borrow::Cow", [inner]) => self.borrowed(inner, function, position),
            _ => match self.types.get(&canonical) {
                Some(Mapping::Handle) if position == Position::Input => {
                    Err(Reason::new(Category::Receiver, format!("takes the runtime handle `{canonical}` by value")))
                }
                Some(_) => Ok(()),
                None => self
                    .mirrored(&canonical, function, position)
                    .unwrap_or_else(|| Err(self.unmapped(&canonical, &self.renderer.ty(&Type::ResolvedPath(path.clone()))))),
            },
        }
    }

    /// A shared borrow is fine when the target is copied across, or is the runtime handle as a
    /// result; a parameter borrowing the handle waits until the emitter passes the service.
    fn borrowed(&self, inner: &Type, function: &Function, position: Position) -> Result<(), Reason> {
        match inner {
            Type::Primitive(name) if name == "str" || PRIMITIVES.contains(&name.as_str()) => Ok(()),
            Type::Slice(element) if is_byte(element) => Ok(()),
            Type::ResolvedPath(path) if self.types.contains_key(&self.renderer.path(path)) => match self.types.get(&self.renderer.path(path)) {
                Some(Mapping::Handle) if position == Position::Input => {
                    Err(Reason::new(Category::Receiver, "takes the runtime handle as a parameter"))
                }
                _ => Ok(()),
            },
            Type::ResolvedPath(path) if self.has_mirror_definition(path) => {
                self.mirrored(&self.renderer.path(path), function, position).unwrap_or(Ok(()))
            }
            Type::Generic(name) if name == "Self" => self.owner_value(function),
            _ => Err(self.blocked(inner, Category::Borrowed)),
        }
    }

    /// Whether `path` names a public enum, or a struct whose named fields are all public.
    fn has_mirror_definition(&self, path: &Path) -> bool {
        let canonical = self.renderer.path(path);
        self.implementations.enum_def(&canonical).is_some() || self.implementations.struct_def(&canonical).is_some()
    }

    /// Whether an enum or struct crosses as its mirror; `None` when `canonical` is neither. An
    /// enum needs a mirror name, no type parameters, at least one variant, no hidden fields and no
    /// recursion, and every payload must cross as a result.
    fn mirrored(&self, canonical: &str, function: &Function, position: Position) -> Option<Result<(), Reason>> {
        if let Some(def) = self.implementations.struct_def(canonical) {
            return Some(self.recorded(canonical, def, function, position));
        }
        let def = self.implementations.enum_def(canonical)?;
        let blocked = |category: Category, detail: &str| Some(Err(Reason::new(category, format!("{canonical}: {detail}")).about(Some(canonical))));
        if def.generic {
            return blocked(Category::Generic, "a generic enum");
        }
        if self.implementations.mirror(canonical).is_none() {
            return blocked(Category::Unmapped, UNNAMED);
        }
        if def.variants.is_empty() {
            return blocked(Category::Unsupported, "an enum without variants");
        }
        if def.variants.iter().any(|variant| variant.fields == VariantFields::Hidden) {
            return blocked(Category::Unsupported, "hidden fields");
        }
        if self.visiting.borrow().iter().any(|path| path == canonical) || def.variants.iter().flat_map(|variant| variant.fields.types()).any(mentions_self) {
            return blocked(Category::Unsupported, "a recursive enum");
        }
        self.visiting.borrow_mut().push(canonical.to_owned());
        let checked = def.variants.iter().flat_map(|variant| variant.fields.types()).try_for_each(|ty| {
            self.value(ty, function, Position::Output).map_err(|reason| Reason { detail: format!("{canonical}: {}", reason.detail), ..reason })
        });
        self.visiting.borrow_mut().pop();
        Some(checked)
    }

    /// Whether a struct crosses as its mirror: it needs a mirror name, no type parameters, at least
    /// one field, no field holding the runtime handle and no recursion, and every field must cross
    /// in `position`; as a parameter it must also be owned and buildable.
    fn recorded(&self, canonical: &str, def: &StructDef, function: &Function, position: Position) -> Result<(), Reason> {
        let blocked = |category: Category, detail: &str| Err(Reason::new(category, format!("{canonical}: {detail}")).about(Some(canonical)));
        if def.generic {
            return blocked(Category::Generic, "a generic struct");
        }
        if self.implementations.mirror(canonical).is_none() {
            return blocked(Category::Unmapped, UNNAMED);
        }
        if def.fields.is_empty() {
            return blocked(Category::Unsupported, "a struct without fields");
        }
        if let Some((field, _)) = def.fields.iter().find(|(_, ty)| self.holds_handle(ty)) {
            return blocked(Category::Unmapped, &format!("field `{field}` holds the runtime handle"));
        }
        if position == Position::Input && def.borrows {
            return blocked(Category::Unmapped, "a borrowed view, which crosses as a result only");
        }
        if position == Position::Input && !def.constructible {
            return blocked(Category::Unmapped, "`#[non_exhaustive]`, so its mirror cannot build it");
        }
        if self.visiting.borrow().iter().any(|path| path == canonical) || def.fields.iter().any(|(_, ty)| mentions_self(ty)) {
            return blocked(Category::Unsupported, "a recursive struct");
        }
        self.visiting.borrow_mut().push(canonical.to_owned());
        let checked = def.fields.iter().try_for_each(|(field, ty)| {
            self.value(ty, function, position).map_err(|reason| Reason { detail: format!("{canonical}: field `{field}`: {}", reason.detail), ..reason })
        });
        self.visiting.borrow_mut().pop();
        checked
    }

    /// Whether `ty`, through references, wrappers and type arguments, mentions a type mapped to
    /// the runtime handle.
    fn holds_handle(&self, ty: &Type) -> bool {
        match ty {
            Type::BorrowedRef { type_, .. } | Type::RawPointer { type_, .. } | Type::Slice(type_) | Type::Array { type_, .. } => {
                self.holds_handle(type_)
            }
            Type::Tuple(types) => types.iter().any(|ty| self.holds_handle(ty)),
            Type::ResolvedPath(path) => {
                self.types.get(&self.renderer.path(path)) == Some(&Mapping::Handle) || type_args(path).iter().any(|arg| self.holds_handle(arg))
            }
            _ => false,
        }
    }

    fn owner_value(&self, function: &Function) -> Result<(), Reason> {
        self.owner_mapping(function).map(|_| ()).ok_or_else(|| {
            Reason::new(Category::Unmapped, format!("Self ({})", function.owner.as_deref().unwrap_or("no owner"))).about(function.owner.as_deref())
        })
    }

    /// Whether some bound can be satisfied from a mapped value.
    fn convertible(&self, bounds: &[GenericBound], label: &str, function: &Function) -> Result<(), Reason> {
        if self.conversion(bounds).is_some() {
            return Ok(());
        }
        for bound in bounds {
            let GenericBound::TraitBound { trait_, .. } = bound else { continue };
            if self.renderer.path(trait_) == "core::iter::traits::collect::IntoIterator"
                && iterator_item(trait_).is_some_and(|item| self.input(item, function).is_ok())
            {
                // The emitter has no slice parameters yet, so the item converting is not enough.
                return Err(Reason::new(Category::Generic, format!("{label}: an iterable of convertible items waits for slice parameters")));
            }
        }
        let rendered = self.renderer.bounds(bounds);
        Err(Reason::new(self.bounds_category(bounds, Category::Generic), format!("{label}: {rendered}")))
    }

    /// The first specific category among the bounds, otherwise `fallback`.
    fn bounds_category(&self, bounds: &[GenericBound], fallback: Category) -> Category {
        bounds
            .iter()
            .find_map(|bound| match bound {
                GenericBound::TraitBound { trait_, .. } => trait_category(&self.renderer.path(trait_)),
                _ => None,
            })
            .unwrap_or(fallback)
    }

    /// The reason for a type that cannot cross as written, with the most specific category.
    fn blocked(&self, ty: &Type, fallback: Category) -> Reason {
        let rendered = self.renderer.ty(ty);
        let (category, about) = match ty {
            Type::DynTrait(dyn_trait) => {
                (dyn_trait.traits.iter().find_map(|poly| trait_category(&self.renderer.path(&poly.trait_))).unwrap_or(fallback), None)
            }
            Type::ResolvedPath(path) => {
                let canonical = self.renderer.path(path);
                let category = self.unmapped(&canonical, &rendered).category;
                (if category == Category::Unmapped { fallback } else { category }, Some(canonical))
            }
            _ => (fallback, None),
        };
        Reason::new(category, rendered).about(about.as_deref())
    }

    /// An unmapped type, categorized by what it implements rather than by its name.
    fn unmapped(&self, canonical: &str, rendered: &str) -> Reason {
        let category = if canonical == "core::sync::atomic::AtomicBool" {
            Category::Cancellation
        } else if self.implementations.implements(canonical, ITERATOR) {
            Category::Iterator
        } else {
            Category::Unmapped
        };
        Reason::new(category, rendered).about(Some(canonical))
    }
}

/// A rendered type without borrow or generic arguments: `&demo::Name<u8>` becomes `demo::Name`.
fn base(rendered: &str) -> &str {
    let unborrowed = rendered.trim_start_matches('&').trim_start_matches("mut ");
    unborrowed.split('<').next().unwrap_or(unborrowed)
}

/// The enum definition of `item`, whose public path in its own crate is `public`.
fn enum_def(krate: &Crate, renderer: &Renderer, item: &Item, enumeration: &Enum, public: &str) -> EnumDef {
    let field_type = |id: &Id| match krate.index.get(id).map(|field| &field.inner) {
        Some(ItemEnum::StructField(ty)) => Some(portable(ty, renderer)),
        _ => None,
    };
    let variants = enumeration
        .variants
        .iter()
        .filter_map(|id| krate.index.get(id))
        .filter_map(|variant_item| {
            let ItemEnum::Variant(variant) = &variant_item.inner else { return None };
            let fields = match &variant.kind {
                VariantKind::Plain => VariantFields::Unit,
                VariantKind::Tuple(ids) => ids
                    .iter()
                    .map(|id| id.as_ref().and_then(field_type))
                    .collect::<Option<Vec<_>>>()
                    .map_or(VariantFields::Hidden, VariantFields::Tuple),
                VariantKind::Struct { has_stripped_fields: true, .. } => VariantFields::Hidden,
                VariantKind::Struct { fields, .. } => fields
                    .iter()
                    .map(|id| Some((krate.index.get(id)?.name.clone()?, field_type(id)?)))
                    .collect::<Option<Vec<_>>>()
                    .map_or(VariantFields::Hidden, VariantFields::Named),
            };
            Some(VariantDef {
                name: variant_item.name.clone()?,
                discriminant: variant.discriminant.as_ref().map(|discriminant| discriminant.value.clone()),
                fields,
                constructible: !non_exhaustive(&variant_item.attrs),
            })
        })
        .collect();
    EnumDef {
        public: public.to_owned(),
        variants,
        open: enumeration.has_stripped_variants || non_exhaustive(&item.attrs),
        generic: enumeration.generics.params.iter().any(|param| !matches!(param.kind, GenericParamDefKind::Lifetime { .. })),
    }
}

/// The definition of `item`, a struct whose public path in its own crate is `public`; `None`
/// unless its fields are named and all public.
fn struct_def(krate: &Crate, renderer: &Renderer, item: &Item, structure: &Struct, public: &str) -> Option<StructDef> {
    let StructKind::Plain { fields, has_stripped_fields: false } = &structure.kind else { return None };
    let fields = fields
        .iter()
        .map(|id| {
            let field = krate.index.get(id)?;
            let ItemEnum::StructField(ty) = &field.inner else { return None };
            Some((field.name.clone()?, portable(ty, renderer)))
        })
        .collect::<Option<Vec<_>>>()?;
    let params = &structure.generics.params;
    Some(StructDef {
        public: public.to_owned(),
        fields,
        borrows: params.iter().any(|param| matches!(param.kind, GenericParamDefKind::Lifetime { .. })),
        constructible: !non_exhaustive(&item.attrs),
        generic: params.iter().any(|param| !matches!(param.kind, GenericParamDefKind::Lifetime { .. })),
    })
}

fn non_exhaustive(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attribute| matches!(attribute, Attribute::NonExhaustive))
}

/// `ty` with every path written canonically under an id no renderer knows.
fn portable(ty: &Type, renderer: &Renderer) -> Type {
    match ty {
        Type::ResolvedPath(path) => Type::ResolvedPath(Path { path: renderer.path(path), id: UNKNOWN_ID, ..map_args(path, &|ty| portable(ty, renderer)) }),
        other => map_children(other, &|ty| portable(ty, renderer)),
    }
}

/// `ty` with `f` applied to the types it holds through a reference, pointer, slice, array or
/// tuple; other types come back unchanged.
fn map_children(ty: &Type, f: &dyn Fn(&Type) -> Type) -> Type {
    match ty {
        Type::BorrowedRef { lifetime, is_mutable, type_ } => Type::BorrowedRef { lifetime: lifetime.clone(), is_mutable: *is_mutable, type_: Box::new(f(type_)) },
        Type::RawPointer { is_mutable, type_ } => Type::RawPointer { is_mutable: *is_mutable, type_: Box::new(f(type_)) },
        Type::Slice(inner) => Type::Slice(Box::new(f(inner))),
        Type::Array { type_, len } => Type::Array { type_: Box::new(f(type_)), len: len.clone() },
        Type::Tuple(types) => Type::Tuple(types.iter().map(f).collect()),
        other => other.clone(),
    }
}

/// `path` with `f` applied to its type arguments.
fn map_args(path: &Path, f: &dyn Fn(&Type) -> Type) -> Path {
    let args = path.args.as_deref().map(|args| {
        Box::new(match args {
            GenericArgs::AngleBracketed { args, constraints } => GenericArgs::AngleBracketed {
                args: args
                    .iter()
                    .map(|arg| match arg {
                        GenericArg::Type(ty) => GenericArg::Type(f(ty)),
                        other => other.clone(),
                    })
                    .collect(),
                constraints: constraints.clone(),
            },
            other => other.clone(),
        })
    });
    Path { path: path.path.clone(), id: path.id, args }
}

/// `ty` with the type parameters in `bindings` replaced.
fn substitute(ty: &Type, bindings: &BTreeMap<String, Type>) -> Type {
    match ty {
        Type::Generic(name) => bindings.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::ResolvedPath(path) => Type::ResolvedPath(map_args(path, &|arg| substitute(arg, bindings))),
        other => map_children(other, &|ty| substitute(ty, bindings)),
    }
}

/// std's `io::Result<T>`, an alias of `Result<T, io::Error>`; std has no rustdoc JSON to read it
/// from.
fn io_result(canonical: &str) -> Option<Alias> {
    IO_RESULT.contains(&canonical).then(|| {
        let written = |path: &str, args: Vec<GenericArg>| {
            let args = (!args.is_empty()).then(|| Box::new(GenericArgs::AngleBracketed { args, constraints: Vec::new() }));
            Type::ResolvedPath(Path { path: path.to_owned(), id: UNKNOWN_ID, args })
        };
        let ok = GenericArg::Type(Type::Generic("T".to_owned()));
        let error = GenericArg::Type(written("core::io::error::Error", Vec::new()));
        Alias { params: vec![("T".to_owned(), None)], target: written("core::result::Result", vec![ok, error]) }
    })
}

/// The mirror name of a public path through the root crate: `demo::remote::Direction` becomes
/// `RemoteDirection`.
fn mirror_name(path: &str) -> Option<String> {
    let mut segments = path.split("::");
    segments.next()?;
    let name: String = segments.map(pascal).collect();
    (!name.is_empty()).then_some(name)
}

/// `tree_diff` becomes `TreeDiff`; names already in Pascal case stay.
fn pascal(segment: &str) -> String {
    segment
        .trim_start_matches("r#")
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
        })
        .collect()
}

/// Whether `ty` mentions `Self`, as a recursive enum's payload does.
fn mentions_self(ty: &Type) -> bool {
    match ty {
        Type::Generic(name) => name == "Self",
        Type::BorrowedRef { type_, .. } | Type::RawPointer { type_, .. } | Type::Slice(type_) | Type::Array { type_, .. } => mentions_self(type_),
        Type::Tuple(types) => types.iter().any(mentions_self),
        Type::ResolvedPath(path) => type_args(path).iter().any(|arg| mentions_self(arg)),
        _ => false,
    }
}
/// The specific category of a std trait, if it has one.
fn trait_category(canonical: &str) -> Option<Category> {
    match canonical {
        "core::ops::function::Fn" | "core::ops::function::FnMut" | "core::ops::function::FnOnce" => Some(Category::Callback),
        "std::io::Read" | "std::io::Write" | "std::io::BufRead" => Some(Category::Io),
        ITERATOR | "core::iter::traits::double_ended::DoubleEndedIterator" | "core::iter::traits::exact_size::ExactSizeIterator" => {
            Some(Category::Iterator)
        }
        _ => None,
    }
}

/// The `Item` type of an `IntoIterator<Item = ...>` bound.
fn iterator_item(trait_: &Path) -> Option<&Type> {
    let Some(GenericArgs::AngleBracketed { constraints, .. }) = trait_.args.as_deref() else { return None };
    constraints.iter().find_map(|constraint| match &constraint.binding {
        AssocItemConstraintKind::Equality(Term::Type(item)) if constraint.name == "Item" => Some(item),
        _ => None,
    })
}

fn is_byte(ty: &Type) -> bool {
    matches!(ty, Type::Primitive(name) if name == "u8")
}

/// Whether `path` is `prefix` or lies below it.
fn is_under(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix).is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::tests::{bound, resolved, shared};
    use rustdoc_types::{AssocItemConstraint, GenericArg};

    const REPO: &str = "demo::inner::Repo";

    fn renderer() -> Renderer {
        Renderer::from_paths([
            (1, "alloc::string::String"),
            (2, "bstr::bstr::BStr"),
            (3, "core::convert::AsRef"),
            (4, "std::path::Path"),
            (5, "core::result::Result"),
            (6, "core::option::Option"),
            (7, "demo::Error"),
            (8, "demo::Iter"),
            (9, "core::ops::function::FnMut"),
            (10, "core::iter::traits::collect::IntoIterator"),
            (11, "core::sync::atomic::AtomicBool"),
            (12, "demo::AsBytes"),
            (13, "demo::Platform"),
            (14, "demo::Repo"),
            (15, "demo::Mode"),
            (16, "demo::Event"),
            (17, "demo::List"),
            (18, "demo::Pair"),
            (19, "demo::Stats"),
            (20, "demo::Label"),
            (21, "demo::Holder"),
            (22, "demo::Open"),
            (23, "demo::Count"),
            (24, "demo::Res"),
            (25, "core::io::error::Result"),
            (26, "demo::Counted"),
        ])
    }

    fn method(receiver: Receiver, params: Vec<(&str, Type)>, output: Option<Type>) -> Function {
        Function {
            path: "demo::Repo::method".to_owned(),
            name: "method".to_owned(),
            owner: Some(REPO.to_owned()),
            owner_public: Some("demo::Repo".to_owned()),
            receiver,
            params: params.into_iter().map(|(name, ty)| (name.to_owned(), ty)).collect(),
            output,
            generics: BTreeMap::new(),
            deprecated: false,
            is_async: false,
            is_unsafe: false,
        }
    }

    fn implementations() -> Implementations {
        Implementations::from_pairs([(ITERATOR, "demo::Iter"), ("demo::AsBytes", "[u8]")])
    }

    fn classify(function: &Function) -> Verdict {
        let renderer = renderer();
        let overrides = BTreeMap::from([("demo::Repo".to_owned(), Mapping::Handle), ("bstr::bstr::BStr".to_owned(), Mapping::Bytes)]);
        let excluded = vec!["demo::schema".to_owned()];
        let implementations = implementations();
        Classifier::new(&renderer, &overrides, &excluded, &implementations).classify(function)
    }

    fn category(function: &Function) -> Option<Category> {
        classify(function).reasons.first().map(|reason| reason.category)
    }

    fn unit(name: &str) -> VariantDef {
        VariantDef { name: name.to_owned(), discriminant: None, fields: VariantFields::Unit, constructible: true }
    }

    fn enum_def(public: &str, variants: Vec<VariantDef>) -> EnumDef {
        EnumDef { public: public.to_owned(), variants, open: false, generic: false }
    }

    /// A fieldless enum, a data enum, a recursive one and a generic one, all under `demo`.
    fn enums() -> Implementations {
        let renamed = VariantDef { fields: VariantFields::Named(vec![("to".to_owned(), resolved(1, Vec::new()))]), ..unit("Renamed") };
        let next = VariantDef { fields: VariantFields::Tuple(vec![resolved(17, Vec::new())]), ..unit("Next") };
        Implementations::from_pairs([(ITERATOR, "demo::Iter")])
            .with_root("demo")
            .with_enum("demo::Mode", enum_def("demo::Mode", vec![unit("Read"), unit("Write")]))
            .with_enum("demo::Event", EnumDef { open: true, ..enum_def("demo::Event", vec![renamed, unit("Closed")]) })
            .with_enum("demo::List", enum_def("demo::List", vec![next, unit("End")]))
            .with_enum("demo::Pair", EnumDef { generic: true, ..enum_def("demo::Pair", vec![unit("Left")]) })
    }

    #[test]
    fn enums_cross_as_mirrors_unless_recursive_generic_or_unnamed() {
        let renderer = renderer();
        let implementations = enums();
        let overrides = BTreeMap::from([("demo::Repo".to_owned(), Mapping::Handle)]);
        let classifier = Classifier::new(&renderer, &overrides, &[], &implementations);
        let category = |output: Type| classifier.classify(&method(Receiver::Shared, Vec::new(), Some(output))).reasons.first().map(|reason| reason.category);
        assert_eq!(category(resolved(15, Vec::new())), None, "a fieldless enum");
        assert_eq!(category(resolved(16, Vec::new())), None, "a data enum whose payload maps");
        assert_eq!(category(shared(resolved(16, Vec::new()))), None, "a borrowed data enum");
        assert_eq!(category(resolved(17, Vec::new())), Some(Category::Unsupported), "a recursive enum");
        assert_eq!(category(resolved(18, Vec::new())), Some(Category::Generic), "a generic enum");
        let parameter = classifier.classify(&method(Receiver::Shared, vec![("mode", resolved(15, Vec::new()))], None));
        assert_eq!(parameter.class, Class::Automatic);
        assert_eq!(implementations.mirror("demo::Mode"), Some("Mode"));
        assert_eq!(implementations.nameable("demo::Mode").as_deref(), Some("demo::Mode"));
    }

    #[test]
    fn mirror_names_follow_the_public_path_and_drop_clashes() {
        assert_eq!(mirror_name("demo::remote::Direction").as_deref(), Some("RemoteDirection"));
        assert_eq!(mirror_name("demo::tree_diff::r#Change").as_deref(), Some("TreeDiffChange"));
        let clash = Implementations::default()
            .with_root("demo")
            .with_enum("demo::a::B", enum_def("demo::a::B", vec![unit("X")]))
            .with_enum("demo::inner::AB", enum_def("demo::AB", vec![unit("Y")]));
        assert_eq!((clash.mirror("demo::a::B"), clash.mirror("demo::inner::AB")), (None, None));
    }

    #[test]
    fn mirror_names_follow_reexports_through_globs_and_other_crates() {
        let implementations = Implementations::default()
            .with_root("demo")
            .with_reexport("demo", "demo_diff", "demo::diff")
            .with_reexport("demo_diff", "demo_text_diff", "demo_diff::blob")
            .with_reexport("demo", "demo_hash", "demo::hash")
            .with_enum("demo_text_diff::Algorithm", enum_def("demo_text_diff::Algorithm", vec![unit("Myers")]))
            .with_enum("demo_hash::Kind", enum_def("demo_hash::Kind", vec![unit("Sha1")]));
        assert_eq!(implementations.nameable("demo_text_diff::Algorithm").as_deref(), Some("demo::diff::blob::Algorithm"));
        assert_eq!(implementations.mirror("demo_text_diff::Algorithm"), Some("DiffBlobAlgorithm"));
        assert_eq!(implementations.mirror("demo_hash::Kind"), Some("HashKind"));
    }

    /// A record, a borrowed view, a struct holding the runtime handle and a `#[non_exhaustive]` one.
    fn records() -> Implementations {
        let record = |public: &str, fields: Vec<(&str, Type)>| StructDef {
            public: public.to_owned(),
            fields: fields.into_iter().map(|(name, ty)| (name.to_owned(), ty)).collect(),
            borrows: false,
            constructible: true,
            generic: false,
        };
        let stats = record("demo::Stats", vec![("count", Type::Primitive("u32".to_owned())), ("mode", resolved(15, Vec::new()))]);
        let label = StructDef { borrows: true, ..record("demo::Label", vec![("text", shared(Type::Primitive("str".to_owned())))]) };
        let holder = record("demo::Holder", vec![("repo", shared(resolved(14, Vec::new())))]);
        let open = StructDef { constructible: false, ..record("demo::Open", vec![("flag", Type::Primitive("bool".to_owned()))]) };
        enums().with_struct("demo::Stats", stats).with_struct("demo::Label", label).with_struct("demo::Holder", holder).with_struct("demo::Open", open)
    }

    #[test]
    fn structs_cross_as_mirrors_unless_they_hold_the_handle_or_cannot_be_built() {
        let renderer = renderer();
        let implementations = records();
        let overrides = BTreeMap::from([("demo::Repo".to_owned(), Mapping::Handle)]);
        let classifier = Classifier::new(&renderer, &overrides, &[], &implementations);
        let result = |id: u32| classifier.classify(&method(Receiver::Shared, Vec::new(), Some(resolved(id, Vec::new())))).reasons;
        let parameter = |id: u32| classifier.classify(&method(Receiver::Shared, vec![("value", resolved(id, Vec::new()))], None)).reasons;
        assert!(result(19).is_empty() && parameter(19).is_empty(), "a record crosses both ways");
        assert!(result(20).is_empty(), "a borrowed view crosses as a result");
        assert!(parameter(20)[0].detail.ends_with("demo::Label: a borrowed view, which crosses as a result only"), "{:?}", parameter(20));
        assert!(result(22).is_empty(), "a `#[non_exhaustive]` struct crosses as a result");
        assert!(parameter(22)[0].detail.ends_with("demo::Open: `#[non_exhaustive]`, so its mirror cannot build it"), "{:?}", parameter(22));
        let holder = result(21);
        assert_eq!((holder[0].category, holder[0].detail.as_str()), (Category::Unmapped, "demo::Holder: field `repo` holds the runtime handle"));
        assert_eq!(implementations.mirror("demo::Stats"), Some("Stats"));
    }

    #[test]
    fn aliases_are_transparent() {
        let renderer = renderer();
        let written = |path: &str| Type::ResolvedPath(Path { path: path.to_owned(), id: UNKNOWN_ID, args: None });
        let counted = StructDef {
            public: "demo::Counted".to_owned(),
            fields: vec![("count".to_owned(), written("demo::Count"))],
            borrows: false,
            constructible: true,
            generic: false,
        };
        let result = resolved(5, vec![GenericArg::Type(Type::Generic("T".to_owned())), GenericArg::Type(Type::Generic("E".to_owned()))]);
        let implementations = records()
            .with_alias("demo::Count", Vec::new(), Type::Primitive("u32".to_owned()))
            .with_alias("demo::Res", vec![("T", None), ("E", Some(resolved(7, Vec::new())))], result)
            .with_struct("demo::Counted", counted);
        let overrides = BTreeMap::from([("demo::Repo".to_owned(), Mapping::Handle)]);
        let classifier = Classifier::new(&renderer, &overrides, &[], &implementations);
        let reasons = |output: Type| classifier.classify(&method(Receiver::Shared, Vec::new(), Some(output))).reasons;
        let count = || resolved(23, Vec::new());
        assert!(reasons(count()).is_empty(), "an alias of a primitive");
        assert!(reasons(resolved(26, Vec::new())).is_empty(), "a record with an aliased field");
        assert!(reasons(resolved(24, vec![GenericArg::Type(count())])).is_empty(), "a generic alias with a default argument");
        assert!(reasons(resolved(25, vec![GenericArg::Type(count())])).is_empty(), "std's io::Result");
        assert_eq!(implementations.struct_def("demo::Counted").map(|def| def.fields[0].1.clone()), Some(Type::Primitive("u32".to_owned())));
    }

    fn as_ref(target: Type) -> Type {
        Type::ImplTrait(vec![bound(3, vec![GenericArg::Type(target)])])
    }

    #[test]
    fn mapped_methods_on_the_runtime_handle_are_automatic() {
        let result = resolved(5, vec![GenericArg::Type(resolved(6, vec![GenericArg::Type(shared(resolved(2, Vec::new())))])), GenericArg::Type(resolved(7, Vec::new()))]);
        let function = method(Receiver::Shared, vec![("path", as_ref(resolved(4, Vec::new())))], Some(result));
        assert_eq!(classify(&function), Verdict { class: Class::Automatic, reasons: Vec::new() });
    }

    #[test]
    fn receivers_need_the_runtime_handle_without_mut() {
        assert_eq!(category(&method(Receiver::Exclusive, Vec::new(), None)), Some(Category::MutHandle));
        let mut other_owner = method(Receiver::Shared, Vec::new(), None);
        other_owner.owner = Some("demo::Commit".to_owned());
        other_owner.owner_public = None;
        assert_eq!(category(&other_owner), Some(Category::Receiver));
    }

    #[test]
    fn every_blocker_is_reported_in_order() {
        let function = method(Receiver::Exclusive, vec![("error", resolved(7, Vec::new()))], Some(resolved(8, Vec::new())));
        let categories: Vec<Category> = classify(&function).reasons.iter().map(|reason| reason.category).collect();
        assert_eq!(categories, [Category::MutHandle, Category::Unmapped, Category::Iterator]);
    }

    #[test]
    fn blockers_name_the_type_they_are_about() {
        let function = method(Receiver::Exclusive, vec![("error", resolved(7, Vec::new()))], None);
        let verdict = classify(&function);
        let types: Vec<Option<&str>> = verdict.reasons.iter().map(|reason| reason.ty.as_deref()).collect();
        assert_eq!(types, [None, Some("demo::Error")]);
        let mut elsewhere = method(Receiver::Shared, Vec::new(), None);
        elsewhere.owner = Some("demo::Other".to_owned());
        elsewhere.owner_public = None;
        assert_eq!(classify(&elsewhere).reasons[0].ty.as_deref(), Some("demo::Other"));
    }

    #[test]
    fn references_keep_their_arguments_and_borrowed_implementers_count() {
        let implementations = Implementations::from_pairs([("core::convert::AsRef<[u8]>", "demo::Blob"), ("core::convert::From<&[u8]>", "&demo::Name")]);
        assert!(implementations.implements_reference("demo::Blob", "core::convert::AsRef<[u8]>"));
        assert!(!implementations.implements_reference("demo::Blob", "core::convert::AsRef<str>"));
        assert!(implementations.implements_reference("demo::Name", "core::convert::From<&[u8]>"));
        assert!(implementations.implements("demo::Blob", "core::convert::AsRef"));
    }

    fn keyed_classifier<'a>(renderer: &'a Renderer, implementations: &'a Implementations, rebuild: &str) -> Classifier<'a> {
        let keyed = Keyed {
            handle: "demo::Repo".to_owned(),
            key: "u64".to_owned(),
            rebuild: rebuild.to_owned(),
            fallible: true,
            key_of: "{value}.id".to_owned(),
            name: "demo::Commit".to_owned(),
        };
        let overrides = BTreeMap::from([("demo::Repo".to_owned(), Mapping::Handle), ("demo::Commit".to_owned(), Mapping::Keyed(keyed))]);
        Classifier::new(renderer, &overrides, &[], implementations)
    }

    #[test]
    fn keyed_types_take_self_by_reference_or_value_but_not_mut() {
        let renderer = renderer();
        let implementations = implementations();
        let classifier = keyed_classifier(&renderer, &implementations, "{handle}.find({key})");
        let on_commit = |receiver: Receiver, output: Option<Type>| {
            let mut function = method(receiver, Vec::new(), output);
            function.owner = Some("demo::Commit".to_owned());
            function.owner_public = None;
            classifier.classify(&function).reasons.first().map(|reason| reason.category)
        };
        assert_eq!(on_commit(Receiver::Shared, None), None);
        assert_eq!(on_commit(Receiver::Value, Some(Type::Primitive("u64".to_owned()))), None);
        assert_eq!(on_commit(Receiver::Exclusive, None), Some(Category::MutHandle));
        assert_eq!(on_commit(Receiver::Value, Some(Type::Generic("Self".to_owned()))), Some(Category::Builder));
        assert_eq!(on_commit(Receiver::Shared, Some(Type::Generic("Self".to_owned()))), None);
    }

    #[test]
    fn parameters_borrowing_the_handle_wait_for_the_emitter() {
        let repo = shared(resolved(14, Vec::new()));
        assert_eq!(category(&method(Receiver::None, vec![("repo", repo.clone())], None)), Some(Category::Receiver));
        assert_eq!(category(&method(Receiver::Shared, Vec::new(), Some(repo))), None, "results may borrow the handle");
    }

    #[test]
    fn keyed_payloads_are_checked() {
        let renderer = renderer();
        let implementations = implementations();
        assert_eq!(keyed_classifier(&renderer, &implementations, "{handle}.find({key})").validate(), Ok(()));
        let missing = keyed_classifier(&renderer, &implementations, "{handle}.find(7)").validate();
        assert!(missing.is_err_and(|why| why.contains("`rebuild` needs")));
        let mut overrides = BTreeMap::from([("demo::Commit".to_owned(), keyed_classifier(&renderer, &implementations, "{handle}.find({key})").mapping("demo::Commit").expect("keyed"))]);
        overrides.insert("demo::Repo".to_owned(), Mapping::Bytes);
        let wrong_repo = Classifier::new(&renderer, &overrides, &[], &implementations).validate();
        assert!(wrong_repo.is_err_and(|why| why.contains("not mapped as `handle`")));
    }

    #[test]
    fn parameters_are_categorized() {
        let callback = Type::ImplTrait(vec![bound(9, Vec::new())]);
        assert_eq!(category(&method(Receiver::None, vec![("each", callback)], None)), Some(Category::Callback));
        let flag = shared(resolved(11, Vec::new()));
        assert_eq!(category(&method(Receiver::None, vec![("interrupt", flag)], None)), Some(Category::Cancellation));
        let names = Type::ImplTrait(vec![GenericBound::TraitBound {
            trait_: rustdoc_types::Path {
                path: "IntoIterator".to_owned(),
                id: rustdoc_types::Id(10),
                args: Some(Box::new(GenericArgs::AngleBracketed {
                    args: Vec::new(),
                    constraints: vec![AssocItemConstraint {
                        name: "Item".to_owned(),
                        args: None,
                        binding: AssocItemConstraintKind::Equality(Term::Type(as_ref(Type::Primitive("str".to_owned())))),
                    }],
                })),
            },
            generic_params: Vec::new(),
            modifier: rustdoc_types::TraitBoundModifier::None,
        }]);
        let verdict = classify(&method(Receiver::None, vec![("names", names)], None));
        assert_eq!(verdict.reasons.first().map(|reason| reason.category), Some(Category::Generic));
        assert!(verdict.reasons[0].detail.ends_with("waits for slice parameters"), "{:?}", verdict.reasons);
    }

    #[test]
    fn bounds_follow_the_type_map_and_trait_implementations() {
        let text = as_ref(Type::Primitive("str".to_owned()));
        assert_eq!(category(&method(Receiver::None, vec![("name", text)], None)), None);
        let unmapped = as_ref(resolved(7, Vec::new()));
        assert_eq!(category(&method(Receiver::None, vec![("error", unmapped)], None)), Some(Category::Generic));
        let bytes_like = vec![bound(12, Vec::new())];
        let function = method(Receiver::None, vec![("data", Type::ImplTrait(bytes_like.clone()))], None);
        assert_eq!(category(&function), None, "a trait implemented for [u8] is satisfied by bytes");
        let renderer = renderer();
        let implementations = implementations();
        let classifier = Classifier::new(&renderer, &BTreeMap::new(), &[], &implementations);
        assert_eq!(classifier.conversion(&bytes_like), Some(Conversion { mapping: Mapping::Bytes, borrowed: true }));
    }

    #[test]
    fn results_are_categorized_by_their_traits_not_their_names() {
        assert_eq!(category(&method(Receiver::None, Vec::new(), Some(resolved(8, Vec::new())))), Some(Category::Iterator));
        assert_eq!(category(&method(Receiver::None, Vec::new(), Some(resolved(13, Vec::new())))), Some(Category::Unmapped));
        assert_eq!(category(&method(Receiver::None, Vec::new(), Some(resolved(7, Vec::new())))), Some(Category::Unmapped));
        assert_eq!(category(&method(Receiver::None, Vec::new(), Some(resolved(1, Vec::new())))), None);
    }

    #[test]
    fn deprecated_and_configured_paths_are_excluded() {
        let mut deprecated = method(Receiver::None, Vec::new(), None);
        deprecated.deprecated = true;
        assert_eq!(classify(&deprecated).class, Class::Excluded);
        let mut schema = method(Receiver::None, Vec::new(), None);
        schema.path = "demo::schema::Key::name".to_owned();
        assert_eq!(category(&schema), Some(Category::ExcludedPath));
        schema.path = "demo::schemas::name".to_owned();
        assert_ne!(category(&schema), Some(Category::ExcludedPath), "prefixes match whole path segments");
    }

    #[test]
    fn custom_mappings_read_from_the_manifest() {
        let json = r#"{"demo::Oid": {"custom": {"ffi": "string", "into_rust": "object_id", "into_ffi": "object_id_out"}}, "demo::Repo": "handle"}"#;
        let map: BTreeMap<String, Mapping> = serde_json::from_str(json).expect("the mappings parse");
        let custom = Custom { ffi: Carrier::String, into_rust: "object_id".to_owned(), into_ffi: "object_id_out".to_owned() };
        assert_eq!(map["demo::Oid"], Mapping::Custom(custom));
        assert_eq!(map["demo::Repo"], Mapping::Handle);
    }
}