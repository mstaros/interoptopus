//! Sorts each surface function into automatic, needs-a-decision or excluded (design sections
//! "Classification (v1)" and "Type map"). The engine knows only std types: everything else comes
//! from the manifest's type map, and generic bounds are matched against it and against the trait
//! implementations rustdoc reports.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rustdoc_types::{
    AssocItemConstraintKind, Crate, Enum, GenericArgs, GenericBound, GenericParamDefKind, Impl, ItemEnum, Path, Struct, Term, Type, Union,
};
use serde::{Deserialize, Serialize};

use crate::render::{Renderer, type_args};
use crate::surface::{Function, Receiver};

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
            Self::Unmapped => "unmapped",
            Self::Borrowed => "borrowed",
            Self::Generic => "generic",
            Self::Iterator => "iterator",
            Self::Callback => "callback",
            Self::Cancellation => "cancellation",
            Self::Io => "io",
            Self::Unsupported => "unsupported",
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
}

impl Implementations {
    /// Adds the trait implementations and lifetime-generic types one crate's JSON lists.
    pub fn add(&mut self, krate: &Crate) {
        let renderer = Renderer::new(krate);
        for item in krate.index.values() {
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
}

/// Classifies surface functions against a type map and exclusion rules.
pub struct Classifier<'a> {
    renderer: &'a Renderer,
    types: BTreeMap<String, Mapping>,
    exclude_paths: &'a [String],
    implementations: &'a Implementations,
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
        Self { renderer, types, exclude_paths, implementations }
    }

    /// This classifier with one more type-map entry, for measuring a proposal.
    #[must_use]
    pub fn with(&self, canonical: &str, mapping: Mapping) -> Self {
        let mut types = self.types.clone();
        types.insert(canonical.to_owned(), mapping);
        Self { renderer: self.renderer, types, exclude_paths: self.exclude_paths, implementations: self.implementations }
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
        let reasons = self.blockers(function);
        let class = if reasons.is_empty() { Class::Automatic } else { Class::Decision };
        Verdict { class, reasons }
    }

    /// The mapping of a canonical type path, if any.
    #[must_use]
    pub fn mapping(&self, canonical: &str) -> Option<Mapping> {
        self.types.get(canonical).cloned()
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
                    _ => self.types.get(&canonical).filter(|mapping| **mapping != Mapping::Handle).cloned(),
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
                .filter(|mapping| **mapping != Mapping::Handle)
                .map(|mapping| (2, Conversion { mapping: mapping.clone(), borrowed: false })),
        }
    }

    /// Every blocker: the receiver, then each parameter in order, then the result.
    fn blockers(&self, function: &Function) -> Vec<Reason> {
        if function.is_async || function.is_unsafe {
            return vec![Reason::new(Category::Unsupported, if function.is_async { "async" } else { "unsafe" })];
        }
        let receiver = self.receiver(function).err();
        let parameters =
            function.params.iter().filter_map(|(name, ty)| self.input(ty, function).err().map(|reason| reason.in_parameter(name)));
        let result = function.output.as_ref().and_then(|output| self.value(output, function, Position::Output).err());
        receiver.into_iter().chain(parameters).chain(result).collect()
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
            Type::BorrowedRef { is_mutable: false, type_, .. } => self.borrowed(type_, function),
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
            ("alloc::borrow::Cow", [inner]) => self.borrowed(inner, function),
            _ => match self.types.get(&canonical) {
                Some(Mapping::Handle) if position == Position::Input => {
                    Err(Reason::new(Category::Receiver, format!("takes the runtime handle `{canonical}` by value")))
                }
                Some(_) => Ok(()),
                None => Err(self.unmapped(&canonical, &self.renderer.ty(&Type::ResolvedPath(path.clone())))),
            },
        }
    }

    /// A shared borrow is fine when the target is copied across or is the runtime handle.
    fn borrowed(&self, inner: &Type, function: &Function) -> Result<(), Reason> {
        match inner {
            Type::Primitive(name) if name == "str" || PRIMITIVES.contains(&name.as_str()) => Ok(()),
            Type::Slice(element) if is_byte(element) => Ok(()),
            Type::ResolvedPath(path) if self.types.contains_key(&self.renderer.path(path)) => Ok(()),
            Type::Generic(name) if name == "Self" => self.owner_value(function),
            _ => Err(self.blocked(inner, Category::Borrowed)),
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
                return Ok(());
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
        assert_eq!(category(&method(Receiver::None, vec![("names", names)], None)), None, "iterables of convertible items are mapped");
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