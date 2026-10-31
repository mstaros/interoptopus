//! The public function surface of a crate: the public inherent methods of its types and the public
//! free functions of its public modules, each under its public path.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use rustdoc_types::{Crate, GenericBound, GenericParamDefKind, Id, Item, ItemEnum, ItemKind, Type, Visibility, WherePredicate};

use crate::render::Renderer;

/// The rustdoc `crate_id` of the documented crate itself.
const LOCAL_CRATE: u32 = 0;

/// How a method takes `self`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// An associated or free function.
    None,
    /// `&self`.
    Shared,
    /// `&mut self`.
    Exclusive,
    /// `self` by value.
    Value,
    /// Any other receiver, such as `self: Arc<Self>`, as canonical text.
    Other(String),
}

/// One function of the public surface.
#[derive(Clone, Debug)]
pub struct Function {
    /// Public path, such as `demo::Repo::head_id`; duplicates carry a `#2`, `#3` suffix.
    pub path: String,
    /// The function name.
    pub name: String,
    /// Canonical path of the type that owns a method.
    pub owner: Option<String>,
    /// Public path of the type that owns a method.
    pub owner_public: Option<String>,
    /// How the method takes `self`.
    pub receiver: Receiver,
    /// Parameters after the receiver.
    pub params: Vec<(String, Type)>,
    /// The return type, if any.
    pub output: Option<Type>,
    /// Bounds of each declared generic type parameter, from its declaration and `where` clauses.
    pub generics: BTreeMap<String, Vec<GenericBound>>,
    /// Whether the item is deprecated.
    pub deprecated: bool,
    /// Whether the function is `async`.
    pub is_async: bool,
    /// Whether the function is `unsafe`.
    pub is_unsafe: bool,
}

impl Function {
    /// The canonical, lifetime-free signature, such as `fn head_id(&self) -> ...`.
    #[must_use]
    pub fn signature(&self, renderer: &Renderer) -> String {
        let generics = if self.generics.is_empty() {
            String::new()
        } else {
            let params: Vec<String> = self
                .generics
                .iter()
                .map(|(name, bounds)| match renderer.bounds(bounds) {
                    text if text.is_empty() => name.clone(),
                    text => format!("{name}: {text}"),
                })
                .collect();
            format!("<{}>", params.join(", "))
        };
        let receiver = match &self.receiver {
            Receiver::None => None,
            Receiver::Shared => Some("&self".to_owned()),
            Receiver::Exclusive => Some("&mut self".to_owned()),
            Receiver::Value => Some("self".to_owned()),
            Receiver::Other(ty) => Some(format!("self: {ty}")),
        };
        let params: Vec<String> =
            receiver.into_iter().chain(self.params.iter().map(|(name, ty)| format!("{name}: {}", renderer.ty(ty)))).collect();
        let output = self.output.as_ref().map_or_else(String::new, |ty| format!(" -> {}", renderer.ty(ty)));
        let qualifiers = match (self.is_async, self.is_unsafe) {
            (true, true) => "async unsafe ",
            (true, false) => "async ",
            (false, true) => "unsafe ",
            (false, false) => "",
        };
        format!("{qualifiers}fn {}{generics}({}){output}", self.name, params.join(", "))
    }
}

/// Collects the surface, sorted by path.
#[must_use]
pub fn collect(krate: &Crate, renderer: &Renderer) -> Vec<Function> {
    let public = public_paths(krate);
    let mut functions = Vec::new();
    for item in krate.index.values().filter(|item| item.crate_id == LOCAL_CRATE) {
        let ItemEnum::Impl(imp) = &item.inner else { continue };
        if imp.trait_.is_some() || imp.is_synthetic || imp.blanket_impl.is_some() {
            continue;
        }
        let Type::ResolvedPath(owner) = &imp.for_ else { continue };
        if !krate.paths.contains_key(&owner.id) {
            continue;
        }
        let canonical = renderer.path(owner);
        let public_owner = public.get(&owner.id).cloned().unwrap_or_else(|| canonical.clone());
        for method in imp.items.iter().filter_map(|id| krate.index.get(id)) {
            if let Some(mut function) = function(method, &public_owner, renderer) {
                function.owner = Some(canonical.clone());
                function.owner_public = Some(public_owner.clone());
                functions.push(function);
            }
        }
    }
    let crate_name = krate.index.get(&krate.root).and_then(|root| root.name.clone()).unwrap_or_default();
    free_functions(krate, krate.root, &crate_name, renderer, &mut functions);
    disambiguate(&mut functions, renderer);
    functions
}

/// Canonical paths of local items keyed by their public paths, so a manifest may name a type
/// either way.
#[must_use]
pub fn public_to_canonical(krate: &Crate) -> HashMap<String, String> {
    public_paths(krate)
        .into_iter()
        .filter_map(|(id, public)| krate.paths.get(&id).map(|summary| (public, summary.path.join("::"))))
        .collect()
}

/// Public paths of local items, shortest first, following public modules and re-exports.
pub fn public_paths(krate: &Crate) -> HashMap<Id, String> {
    let mut paths = HashMap::new();
    let root_name = krate.index.get(&krate.root).and_then(|root| root.name.clone()).unwrap_or_default();
    let mut visited = HashSet::from([krate.root]);
    let mut pending = VecDeque::from([(krate.root, root_name)]);
    while let Some((module_id, prefix)) = pending.pop_front() {
        let Some(ItemEnum::Module(module)) = krate.index.get(&module_id).map(|item| &item.inner) else { continue };
        for id in &module.items {
            let Some(item) = krate.index.get(id).filter(|item| matches!(item.visibility, Visibility::Public)) else {
                continue;
            };
            let (target, path) = match &item.inner {
                ItemEnum::Use(import) => match import.id {
                    Some(target) if import.is_glob => (target, prefix.clone()),
                    Some(target) => (target, format!("{prefix}::{}", import.name)),
                    None => continue,
                },
                _ => match &item.name {
                    Some(name) => (*id, format!("{prefix}::{name}")),
                    None => continue,
                },
            };
            let is_module = matches!(krate.index.get(&target).map(|target| &target.inner), Some(ItemEnum::Module(_)));
            // A glob of a local module adds that module's items under this prefix (followed below); a
            // glob of another crate's module stands for that module re-exported here.
            let glob = matches!(&item.inner, ItemEnum::Use(import) if import.is_glob);
            let external_module = !krate.index.contains_key(&target) && krate.paths.get(&target).is_some_and(|summary| summary.kind == ItemKind::Module);
            if !glob || external_module {
                paths.entry(target).or_insert_with(|| path.clone());
            }
            if is_module && visited.insert(target) {
                pending.push_back((target, path));
            }
        }
    }
    paths
}

/// Public free functions of public modules; re-exports are not followed.
fn free_functions(krate: &Crate, module_id: Id, prefix: &str, renderer: &Renderer, out: &mut Vec<Function>) {
    let Some(ItemEnum::Module(module)) = krate.index.get(&module_id).map(|item| &item.inner) else { return };
    for id in &module.items {
        let Some(item) = krate.index.get(id).filter(|item| matches!(item.visibility, Visibility::Public)) else { continue };
        match (&item.inner, &item.name) {
            (ItemEnum::Module(_), Some(name)) => free_functions(krate, *id, &format!("{prefix}::{name}"), renderer, out),
            (ItemEnum::Function(_), _) => out.extend(function(item, prefix, renderer)),
            _ => {}
        }
    }
}

/// The surface function for a public function item under `prefix`.
fn function(item: &Item, prefix: &str, renderer: &Renderer) -> Option<Function> {
    let ItemEnum::Function(function) = &item.inner else { return None };
    if !matches!(item.visibility, Visibility::Public) {
        return None;
    }
    let name = item.name.clone()?;
    let (receiver, params) = match function.sig.inputs.split_first() {
        Some(((first, ty), rest)) if first == "self" => (receiver(ty, renderer), rest),
        _ => (Receiver::None, function.sig.inputs.as_slice()),
    };
    Some(Function {
        path: format!("{prefix}::{name}"),
        name,
        owner: None,
        owner_public: None,
        receiver,
        params: params.to_vec(),
        output: function.sig.output.clone(),
        generics: generic_bounds(function),
        deprecated: item.deprecation.is_some(),
        is_async: function.header.is_async,
        is_unsafe: function.header.is_unsafe,
    })
}

fn receiver(ty: &Type, renderer: &Renderer) -> Receiver {
    let is_self = |ty: &Type| matches!(ty, Type::Generic(name) if name == "Self");
    match ty {
        _ if is_self(ty) => Receiver::Value,
        Type::BorrowedRef { is_mutable, type_, .. } if is_self(type_) => {
            if *is_mutable { Receiver::Exclusive } else { Receiver::Shared }
        }
        other => Receiver::Other(renderer.ty(other)),
    }
}

/// Bounds per declared (not synthetic) generic type parameter, including `where` clauses.
fn generic_bounds(function: &rustdoc_types::Function) -> BTreeMap<String, Vec<GenericBound>> {
    let mut generics: BTreeMap<String, Vec<GenericBound>> = function
        .generics
        .params
        .iter()
        .filter_map(|param| match &param.kind {
            GenericParamDefKind::Type { bounds, is_synthetic: false, .. } => Some((param.name.clone(), bounds.clone())),
            _ => None,
        })
        .collect();
    for predicate in &function.generics.where_predicates {
        if let WherePredicate::BoundPredicate { type_: Type::Generic(name), bounds, .. } = predicate
            && let Some(existing) = generics.get_mut(name)
        {
            existing.extend(bounds.iter().cloned());
        }
    }
    generics
}

/// Sorts by path and suffixes duplicate paths with `#2`, `#3`, ordered by signature.
fn disambiguate(functions: &mut [Function], renderer: &Renderer) {
    functions.sort_by_cached_key(|function| (function.path.clone(), function.signature(renderer)));
    let mut index = 0;
    while index < functions.len() {
        let end = functions[index..].iter().position(|function| function.path != functions[index].path).map_or(functions.len(), |offset| index + offset);
        for (ordinal, function) in functions[index..end].iter_mut().enumerate().skip(1) {
            function.path = format!("{}#{}", function.path, ordinal + 1);
        }
        index = end;
    }
}