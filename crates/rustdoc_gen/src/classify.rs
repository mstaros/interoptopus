//! Sorts each surface function into automatic, needs-a-decision or excluded (design sections
//! "Classification (v1)" and "Default type map").

use std::collections::BTreeMap;
use std::fmt;

use rustdoc_types::{AssocItemConstraintKind, GenericArgs, GenericBound, Path, Term, Type};
use serde::{Deserialize, Serialize};

use crate::render::{Renderer, last_segment, type_args};
use crate::surface::{Function, Receiver};

/// How a mapped Rust type crosses the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mapping {
    /// UTF-8 text.
    String,
    /// Bytes.
    Bytes,
    /// An object id as hexadecimal text.
    ObjectId,
    /// A filesystem path as bytes.
    Path,
    /// The repository handle held by the hand-written runtime.
    Repository,
}

/// The default type map by canonical path; manifest entries extend and override it.
const DEFAULT_TYPES: [(&str, Mapping); 7] = [
    ("alloc::string::String", Mapping::String),
    ("bstr::bstr::BStr", Mapping::Bytes),
    ("bstr::bstring::BString", Mapping::Bytes),
    ("gix_hash::object_id::ObjectId", Mapping::ObjectId),
    ("gix_hash::borrowed::oid", Mapping::ObjectId),
    ("std::path::Path", Mapping::Path),
    ("std::path::PathBuf", Mapping::Path),
];

/// Primitives that cross the boundary unchanged.
const PRIMITIVES: [&str; 13] = ["bool", "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64"];

/// Generic parameter bounds the facade satisfies from a mapped value: trait name and its canonical
/// type argument, empty when the trait has none.
const CONVERSIONS: [(&str, &str); 11] = [
    ("AsRef", "str"),
    ("Into", "alloc::string::String"),
    ("AsRef", "bstr::bstr::BStr"),
    ("Into", "bstr::bstring::BString"),
    ("AsBStr", ""),
    ("AsRef", "[u8]"),
    ("AsRef", "std::path::Path"),
    ("Into", "std::path::PathBuf"),
    ("AsRef", "std::ffi::os_str::OsStr"),
    ("Into", "gix_hash::object_id::ObjectId"),
    ("AsRef", "gix_hash::borrowed::oid"),
];

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
    /// `&mut self` on the runtime repository needs the persistent repository runtime.
    MutRepository,
    /// A type is not in the type map.
    Unmapped,
    /// A borrowed type that cannot be copied across.
    Borrowed,
    /// A generic parameter or result without a conversion.
    Generic,
    /// An iterator or platform result, until a cursor template exists.
    Iterator,
    /// A callback parameter.
    Callback,
    /// A progress parameter.
    Progress,
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
            Self::MutRepository => "mut_repository",
            Self::Unmapped => "unmapped",
            Self::Borrowed => "borrowed",
            Self::Generic => "generic",
            Self::Iterator => "iterator",
            Self::Callback => "callback",
            Self::Progress => "progress",
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
}

impl Reason {
    fn new(category: Category, detail: impl Into<String>) -> Self {
        Self { category, detail: detail.into() }
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
    /// Why the function is not automatic.
    pub reason: Option<Reason>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    Input,
    Output,
}

/// Classifies surface functions against a type map and exclusion rules.
pub struct Classifier<'a> {
    renderer: &'a Renderer,
    types: BTreeMap<String, Mapping>,
    exclude_paths: &'a [String],
}

impl<'a> Classifier<'a> {
    /// A classifier using the default type map extended by `overrides`.
    #[must_use]
    pub fn new(renderer: &'a Renderer, overrides: &BTreeMap<String, Mapping>, exclude_paths: &'a [String]) -> Self {
        let mut types: BTreeMap<String, Mapping> = DEFAULT_TYPES.iter().map(|(path, mapping)| ((*path).to_owned(), *mapping)).collect();
        types.extend(overrides.iter().map(|(path, mapping)| (path.clone(), *mapping)));
        Self { renderer, types, exclude_paths }
    }

    /// The verdict for one function.
    #[must_use]
    pub fn classify(&self, function: &Function) -> Verdict {
        if function.deprecated {
            return Verdict { class: Class::Excluded, reason: Some(Reason::new(Category::Deprecated, "deprecated")) };
        }
        if let Some(prefix) = self.exclude_paths.iter().find(|prefix| is_under(&function.path, prefix)) {
            return Verdict { class: Class::Excluded, reason: Some(Reason::new(Category::ExcludedPath, prefix.clone())) };
        }
        match self.check(function) {
            Ok(()) => Verdict { class: Class::Automatic, reason: None },
            Err(reason) => Verdict { class: Class::Decision, reason: Some(reason) },
        }
    }

    /// The mapping of a canonical type path, if any.
    #[must_use]
    pub fn mapping(&self, canonical: &str) -> Option<Mapping> {
        self.types.get(canonical).copied()
    }

    /// Canonical paths of the types mapped to the repository handle.
    pub fn handles(&self) -> impl Iterator<Item = &str> {
        self.types.iter().filter(|(_, mapping)| **mapping == Mapping::Repository).map(|(path, _)| path.as_str())
    }

    /// The conversion-table entry a generic parameter's bounds satisfy: the trait name and its
    /// canonical type argument.
    #[must_use]
    pub fn conversion(&self, bounds: &[GenericBound]) -> Option<(&'static str, &'static str)> {
        bounds.iter().find_map(|bound| {
            let GenericBound::TraitBound { trait_, .. } = bound else { return None };
            let canonical = self.renderer.path(trait_);
            let name = last_segment(&canonical);
            let argument = type_args(trait_).into_iter().map(|ty| self.renderer.ty(ty)).collect::<Vec<_>>().join(", ");
            CONVERSIONS.iter().find(|(trait_name, expected)| *trait_name == name && *expected == argument).copied()
        })
    }

    fn check(&self, function: &Function) -> Result<(), Reason> {
        if function.is_async || function.is_unsafe {
            return Err(Reason::new(Category::Unsupported, if function.is_async { "async" } else { "unsafe" }));
        }
        self.receiver(function)?;
        for (name, ty) in &function.params {
            self.input(ty, function).map_err(|reason| reason.in_parameter(name))?;
        }
        function.output.as_ref().map_or(Ok(()), |output| self.value(output, function, Position::Output))
    }

    fn owner_mapping(&self, function: &Function) -> Option<Mapping> {
        [&function.owner, &function.owner_public].into_iter().flatten().find_map(|path| self.types.get(path).copied())
    }

    fn receiver(&self, function: &Function) -> Result<(), Reason> {
        match (&function.receiver, self.owner_mapping(function)) {
            (Receiver::None, _) | (Receiver::Shared, Some(Mapping::Repository)) => Ok(()),
            (Receiver::Exclusive, Some(Mapping::Repository)) => {
                Err(Reason::new(Category::MutRepository, "&mut self needs the persistent repository runtime"))
            }
            (Receiver::Value | Receiver::Other(_), Some(Mapping::Repository)) => {
                Err(Reason::new(Category::Receiver, "takes the repository handle by value or wrapped"))
            }
            _ => Err(Reason::new(Category::Receiver, format!("{} is not a handle", function.owner.as_deref().unwrap_or("Self")))),
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
                Some(Mapping::Repository) if position == Position::Input => {
                    Err(Reason::new(Category::Receiver, format!("takes the repository handle `{canonical}` by value")))
                }
                Some(_) => Ok(()),
                None => Err(self.unmapped(&canonical, &self.renderer.ty(&Type::ResolvedPath(path.clone())))),
            },
        }
    }

    /// A shared borrow is fine when the target is copied across or is the runtime repository.
    fn borrowed(&self, inner: &Type, function: &Function) -> Result<(), Reason> {
        match inner {
            Type::Primitive(name) if name == "str" || PRIMITIVES.contains(&name.as_str()) => Ok(()),
            Type::Slice(element) if matches!(element.as_ref(), Type::Primitive(byte) if byte == "u8") => Ok(()),
            Type::ResolvedPath(path) if self.types.contains_key(&self.renderer.path(path)) => Ok(()),
            Type::Generic(name) if name == "Self" => self.owner_value(function),
            _ => Err(self.blocked(inner, Category::Borrowed)),
        }
    }

    fn owner_value(&self, function: &Function) -> Result<(), Reason> {
        self.owner_mapping(function).map(|_| ()).ok_or_else(|| {
            Reason::new(Category::Unmapped, format!("Self ({})", function.owner.as_deref().unwrap_or("no owner")))
        })
    }

    /// Whether some bound can be satisfied from a mapped value.
    fn convertible(&self, bounds: &[GenericBound], label: &str, function: &Function) -> Result<(), Reason> {
        if self.conversion(bounds).is_some() {
            return Ok(());
        }
        for bound in bounds {
            let GenericBound::TraitBound { trait_, .. } = bound else { continue };
            if last_segment(&self.renderer.path(trait_)) == "IntoIterator" && iterator_item(trait_).is_some_and(|item| self.input(item, function).is_ok()) {
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
        let category = match ty {
            Type::DynTrait(dyn_trait) => {
                dyn_trait.traits.iter().find_map(|poly| trait_category(&self.renderer.path(&poly.trait_))).unwrap_or(fallback)
            }
            Type::ResolvedPath(path) => {
                let category = self.unmapped(&self.renderer.path(path), &rendered).category;
                if category == Category::Unmapped { fallback } else { category }
            }
            _ => fallback,
        };
        Reason::new(category, rendered)
    }

    #[allow(clippy::unused_self, reason = "kept with the other categorizing helpers")]
    fn unmapped(&self, canonical: &str, rendered: &str) -> Reason {
        let name = last_segment(canonical);
        let category = if canonical == "core::sync::atomic::AtomicBool" {
            Category::Cancellation
        } else if name.contains("Iter") || name == "Platform" || canonical.contains("::iter::") {
            Category::Iterator
        } else {
            Category::Unmapped
        };
        Reason::new(category, rendered)
    }
}

/// The specific category of a trait, if it has one.
fn trait_category(canonical: &str) -> Option<Category> {
    match last_segment(canonical) {
        "Fn" | "FnMut" | "FnOnce" => Some(Category::Callback),
        "Read" | "Write" | "BufRead" => Some(Category::Io),
        "Progress" | "DynNestedProgress" | "NestedProgress" | "Count" | "RawProgress" => Some(Category::Progress),
        "Iterator" | "DoubleEndedIterator" | "ExactSizeIterator" => Some(Category::Iterator),
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

    fn classify(function: &Function) -> Verdict {
        let renderer = renderer();
        let overrides = BTreeMap::from([("demo::Repo".to_owned(), Mapping::Repository)]);
        let excluded = vec!["demo::schema".to_owned()];
        Classifier::new(&renderer, &overrides, &excluded).classify(function)
    }

    fn category(function: &Function) -> Option<Category> {
        classify(function).reason.map(|reason| reason.category)
    }

    fn as_ref(target: Type) -> Type {
        Type::ImplTrait(vec![bound(3, vec![GenericArg::Type(target)])])
    }

    #[test]
    fn mapped_methods_on_the_runtime_repository_are_automatic() {
        let result = resolved(5, vec![GenericArg::Type(resolved(6, vec![GenericArg::Type(shared(resolved(2, Vec::new())))])), GenericArg::Type(resolved(7, Vec::new()))]);
        let function = method(Receiver::Shared, vec![("path", as_ref(resolved(4, Vec::new())))], Some(result));
        assert_eq!(classify(&function), Verdict { class: Class::Automatic, reason: None });
    }

    #[test]
    fn receivers_need_the_runtime_repository_without_mut() {
        assert_eq!(category(&method(Receiver::Exclusive, Vec::new(), None)), Some(Category::MutRepository));
        let mut other_owner = method(Receiver::Shared, Vec::new(), None);
        other_owner.owner = Some("demo::Commit".to_owned());
        other_owner.owner_public = None;
        assert_eq!(category(&other_owner), Some(Category::Receiver));
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
    fn results_are_categorized() {
        assert_eq!(category(&method(Receiver::None, Vec::new(), Some(resolved(8, Vec::new())))), Some(Category::Iterator));
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
}