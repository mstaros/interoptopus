//! Managed tuple projections retain the separately named native representation.
use crate::lang::TypeId;
use crate::lang::types::Type;
use crate::lang::types::kind::{Primitive, TypeKind, TypePattern};
use std::collections::HashSet;

/// Values with no borrowed pointers, ownership, or managed reference fields.
#[must_use]
pub fn is_value<'a>(id: TypeId, resolve: &dyn Fn(TypeId) -> Option<&'a Type>) -> bool {
    fn visit<'a>(id: TypeId, resolve: &dyn Fn(TypeId) -> Option<&'a Type>, active: &mut HashSet<TypeId>) -> bool {
        if !active.insert(id) { return false; }
        let valid = match resolve(id).map(|ty| &ty.kind) {
            Some(TypeKind::Primitive(p)) => *p != Primitive::Void,
            Some(TypeKind::TypePattern(TypePattern::Bool | TypePattern::CChar | TypePattern::Version)) => true,
            Some(TypeKind::DataEnum(e)) => !e.is_union_projected() && e.discriminant_type.is_csharp_enum_underlying(),
            Some(TypeKind::Composite(c)) => c.fields.iter().all(|f| visit(f.ty, resolve, active)),
            _ => false,
        };
        active.remove(&id);
        valid
    }
    visit(id, resolve, &mut HashSet::new())
}

/// A positional struct with at least two plain value fields becomes a tuple.
#[must_use]
pub fn name<'a>(id: TypeId, resolve: &dyn Fn(TypeId) -> Option<&'a Type>) -> Option<String> {
    let TypeKind::Composite(c) = &resolve(id)?.kind else { return None };
    if !c.is_positional || c.fields.len() < 2 || !is_value(id, resolve) { return None; }
    let fields = c.fields.iter().map(|f| {
        let ty = resolve(f.ty)?;
        if matches!(ty.kind, TypeKind::Primitive(Primitive::Bool) | TypeKind::TypePattern(TypePattern::Bool)) {
            Some("bool".to_string())
        } else {
            managed_name(f.ty, resolve)
        }
    }).collect::<Option<Vec<_>>>()?;
    Some(format!("({})", fields.join(", ")))
}

/// Public API spelling. Native identifiers continue to use `Type::name`.
#[must_use]
pub fn managed_name<'a>(id: TypeId, resolve: &dyn Fn(TypeId) -> Option<&'a Type>) -> Option<String> {
    if let Some(tuple) = name(id, resolve) { return Some(tuple); }
    let ty = resolve(id)?;
    match &ty.kind {
        TypeKind::Task(task) => {
            if let Some(inner) = task.inner.and_then(|inner| name(inner, resolve)) {
                return Some(format!("Task<{inner}>"));
            }
        }
        TypeKind::Delegate(d) if ty.name.starts_with("global::System.") => {
            return d.standard_name(resolve).or_else(|| Some(ty.name.clone()));
        }
        _ => {}
    }
    Some(ty.name.clone())
}

/// Convert a tuple expression back into its ABI-specific managed wrapper.
#[must_use]
pub fn native_value<'a>(id: TypeId, expression: &str, resolve: &dyn Fn(TypeId) -> Option<&'a Type>) -> String {
    if name(id, resolve).is_some() {
        format!("(({}){expression})", resolve(id).expect("resolved tuple").name)
    } else {
        expression.to_string()
    }
}
