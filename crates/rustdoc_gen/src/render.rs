//! Canonical, lifetime-free text for rustdoc types. Paths are the defining paths from rustdoc's
//! `paths` table, so a type renders the same wherever it is re-exported.

use std::collections::HashMap;

use rustdoc_types::{
    AssocItemConstraintKind, Crate, GenericArg, GenericArgs, GenericBound, Id, Path, Term, TraitBoundModifier, Type,
};

/// Renders rustdoc types using canonical item paths. The default renderer knows no ids, so every
/// path renders as written.
#[derive(Default)]
pub struct Renderer {
    paths: HashMap<Id, String>,
}

impl Renderer {
    /// A renderer for the items of `krate`.
    #[must_use]
    pub fn new(krate: &Crate) -> Self {
        Self { paths: krate.paths.iter().map(|(id, summary)| (*id, summary.path.join("::"))).collect() }
    }

    /// A renderer with an explicit id-to-path table.
    #[cfg(test)]
    pub fn from_paths(paths: impl IntoIterator<Item = (u32, &'static str)>) -> Self {
        Self { paths: paths.into_iter().map(|(id, path)| (Id(id), path.to_owned())).collect() }
    }

    /// The canonical path of a resolved path, or its written path when rustdoc has no entry.
    #[must_use]
    pub fn path(&self, path: &Path) -> String {
        self.paths.get(&path.id).cloned().unwrap_or_else(|| path.path.clone())
    }

    /// A type as canonical text.
    #[must_use]
    pub fn ty(&self, ty: &Type) -> String {
        match ty {
            Type::ResolvedPath(path) => format!("{}{}", self.path(path), self.args(path.args.as_deref())),
            Type::DynTrait(dyn_trait) => {
                let traits: Vec<String> = dyn_trait
                    .traits
                    .iter()
                    .map(|poly| format!("{}{}", self.path(&poly.trait_), self.args(poly.trait_.args.as_deref())))
                    .collect();
                format!("dyn {}", traits.join(" + "))
            }
            Type::Generic(name) | Type::Primitive(name) => name.clone(),
            Type::FunctionPointer(pointer) => {
                let inputs: Vec<&Type> = pointer.sig.inputs.iter().map(|(_, input)| input).collect();
                format!("fn({}){}", self.list(inputs), self.output(pointer.sig.output.as_ref()))
            }
            Type::Tuple(types) => match types.as_slice() {
                [single] => format!("({},)", self.ty(single)),
                _ => format!("({})", self.list(types)),
            },
            Type::Slice(inner) => format!("[{}]", self.ty(inner)),
            Type::Array { type_, len } => format!("[{}; {len}]", self.ty(type_)),
            Type::Pat { type_, .. } => self.ty(type_),
            Type::ImplTrait(bounds) => format!("impl {}", self.bounds(bounds)),
            Type::Infer => "_".to_owned(),
            Type::RawPointer { is_mutable, type_ } => {
                format!("*{} {}", if *is_mutable { "mut" } else { "const" }, self.ty(type_))
            }
            Type::BorrowedRef { is_mutable, type_, .. } => {
                format!("&{}{}", if *is_mutable { "mut " } else { "" }, self.ty(type_))
            }
            Type::QualifiedPath { name, args, self_type, trait_ } => {
                let qualified = trait_.as_ref().map_or_else(
                    || format!("<{}>", self.ty(self_type)),
                    |trait_| format!("<{} as {}>", self.ty(self_type), self.path(trait_)),
                );
                format!("{qualified}::{name}{}", self.args(args.as_deref()))
            }
        }
    }

    /// Trait bounds joined with `+`; lifetime and precise-capturing bounds are left out.
    #[must_use]
    pub fn bounds(&self, bounds: &[GenericBound]) -> String {
        bounds.iter().filter_map(|bound| self.bound(bound)).collect::<Vec<_>>().join(" + ")
    }

    fn bound(&self, bound: &GenericBound) -> Option<String> {
        match bound {
            GenericBound::TraitBound { trait_, modifier, .. } => {
                let prefix = match modifier {
                    TraitBoundModifier::None => "",
                    TraitBoundModifier::Maybe => "?",
                    TraitBoundModifier::MaybeConst => "~const ",
                };
                Some(format!("{prefix}{}{}", self.path(trait_), self.args(trait_.args.as_deref())))
            }
            GenericBound::Outlives(_) | GenericBound::Use(_) => None,
        }
    }

    fn list<'t>(&self, types: impl IntoIterator<Item = &'t Type>) -> String {
        types.into_iter().map(|ty| self.ty(ty)).collect::<Vec<_>>().join(", ")
    }

    fn output(&self, output: Option<&Type>) -> String {
        output.map_or_else(String::new, |ty| format!(" -> {}", self.ty(ty)))
    }

    fn args(&self, args: Option<&GenericArgs>) -> String {
        match args {
            Some(GenericArgs::AngleBracketed { args, constraints }) => {
                let parts: Vec<String> = args
                    .iter()
                    .filter_map(|arg| match arg {
                        GenericArg::Lifetime(_) => None,
                        GenericArg::Type(ty) => Some(self.ty(ty)),
                        GenericArg::Const(constant) => Some(constant.expr.clone()),
                        GenericArg::Infer => Some("_".to_owned()),
                    })
                    .chain(constraints.iter().map(|constraint| match &constraint.binding {
                        AssocItemConstraintKind::Equality(Term::Type(ty)) => format!("{} = {}", constraint.name, self.ty(ty)),
                        AssocItemConstraintKind::Equality(Term::Constant(constant)) => {
                            format!("{} = {}", constraint.name, constant.expr)
                        }
                        AssocItemConstraintKind::Constraint(bounds) => format!("{}: {}", constraint.name, self.bounds(bounds)),
                    }))
                    .collect();
                if parts.is_empty() { String::new() } else { format!("<{}>", parts.join(", ")) }
            }
            Some(GenericArgs::Parenthesized { inputs, output }) => format!("({}){}", self.list(inputs), self.output(output.as_ref())),
            Some(GenericArgs::ReturnTypeNotation) => "(..)".to_owned(),
            None => String::new(),
        }
    }
}

/// The type arguments of a path, leaving out lifetimes and constants.
#[must_use]
pub fn type_args(path: &Path) -> Vec<&Type> {
    match path.args.as_deref() {
        Some(GenericArgs::AngleBracketed { args, .. }) => args
            .iter()
            .filter_map(|arg| match arg {
                GenericArg::Type(ty) => Some(ty),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The last segment of a `::`-separated path.
#[must_use]
pub fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A resolved path with optional angle-bracketed arguments.
    pub fn resolved(id: u32, args: Vec<GenericArg>) -> Type {
        Type::ResolvedPath(path(id, args))
    }

    /// A path with optional angle-bracketed arguments.
    pub fn path(id: u32, args: Vec<GenericArg>) -> Path {
        let args = (!args.is_empty()).then(|| Box::new(GenericArgs::AngleBracketed { args, constraints: Vec::new() }));
        Path { path: format!("written{id}"), id: Id(id), args }
    }

    /// A shared reference with a lifetime, which rendering must drop.
    pub fn shared(ty: Type) -> Type {
        Type::BorrowedRef { lifetime: Some("'a".to_owned()), is_mutable: false, type_: Box::new(ty) }
    }

    /// A trait bound on the trait with the given id.
    pub fn bound(id: u32, args: Vec<GenericArg>) -> GenericBound {
        GenericBound::TraitBound { trait_: path(id, args), generic_params: Vec::new(), modifier: TraitBoundModifier::None }
    }

    fn renderer() -> Renderer {
        Renderer::from_paths([(1, "core::option::Option"), (2, "bstr::bstr::BStr"), (3, "core::convert::AsRef"), (4, "std::path::Path")])
    }

    #[test]
    fn renders_canonical_paths_without_lifetimes() {
        let ty = resolved(1, vec![GenericArg::Lifetime("'a".to_owned()), GenericArg::Type(shared(resolved(2, Vec::new())))]);
        assert_eq!(renderer().ty(&ty), "core::option::Option<&bstr::bstr::BStr>");
    }

    #[test]
    fn renders_impl_trait_bounds_and_unit() {
        let ty = Type::ImplTrait(vec![bound(3, vec![GenericArg::Type(resolved(4, Vec::new()))]), GenericBound::Outlives("'a".to_owned())]);
        assert_eq!(renderer().ty(&ty), "impl core::convert::AsRef<std::path::Path>");
        assert_eq!(renderer().ty(&Type::Tuple(Vec::new())), "()");
    }

    #[test]
    fn unknown_ids_fall_back_to_the_written_path() {
        assert_eq!(renderer().ty(&resolved(99, Vec::new())), "written99");
    }
}