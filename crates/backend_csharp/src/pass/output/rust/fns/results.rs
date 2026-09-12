//! Rejects results whose pointer ownership cannot be represented by a typed C# value.
//!
//! `SafeHandle` protects a registered service constructor's owned handle. It cannot
//! infer the owner, allocator, or lifetime of an arbitrary pointer returned by an API.

use crate::lang::TypeId;
use crate::lang::types::kind::{TypeKind, TypePattern};
use crate::output::FileType;
use crate::pass::{OutputResult, model, output};
use std::collections::HashSet;

pub fn validate(
    master: &output::common::master::Pass,
    fns: &model::common::fns::all::Pass,
    types: &model::common::types::all::Pass,
    services: &model::common::service::all::Pass,
    wire_helpers: &model::common::wire::helpers::Pass,
    ids: &model::common::id_map::Pass,
) -> OutputResult {
    // This allocator is private implementation plumbing for an owning WireBuffer.
    // Match the discovered helper identity, never exempt arbitrary interoptopus_* APIs.
    let wire_create = wire_helpers.helpers().and_then(|helpers| ids.fns(helpers.create_fn));
    for file in master.outputs_of(FileType::Csharp) {
        for (&id, function) in fns.originals() {
            if !master.fn_belongs_to(id, file) { continue; }
            let owner = services.iter().find_map(|(_, service)| service.sources.ctors.contains(&id).then_some(service.ty));
            if Some(id) != wire_create {
                check(function.signature.rval, types, &function.name, "return", owner, &mut HashSet::new())?;
            }
            // Async exports return task plumbing at the ABI level; their actual result
            // arrives through the completion callback, including async constructors.
            for argument in &function.signature.arguments {
                if let Some(TypeKind::TypePattern(TypePattern::AsyncCallback(inner))) = types.get(argument.ty).map(|ty| &ty.kind) {
                    check(*inner, types, &function.name, "async result", owner, &mut HashSet::new())?;
                }
            }
        }
        // Returned native callbacks expose Call/CallRaw too, and standalone callback
        // types can be registered without a function. Validate their result contracts.
        for (&id, ty) in types.iter() {
            if master.type_belongs_to(id, file)
                && let TypeKind::Delegate(delegate) = &ty.kind
            {
                check(delegate.signature.rval, types, &ty.name, "callback return", None, &mut HashSet::new())?;
            }
        }
    }
    Ok(())
}

fn check(
    id: TypeId,
    types: &model::common::types::all::Pass,
    api: &str,
    path: &str,
    constructor_owner: Option<TypeId>,
    visiting: &mut HashSet<TypeId>,
) -> OutputResult {
    if !visiting.insert(id) { return Ok(()); }
    let ty = types.get(id).ok_or_else(|| crate::Error::from(format!("Unresolved result type in API `{api}` at `{path}`")))?;
    match &ty.kind {
        // Only the constructor's own service pointer is adopted by its SafeHandle.
        // The exemption does not extend to fields, error payloads, or ordinary methods.
        TypeKind::Pointer(pointer) if constructor_owner == Some(pointer.target) => {}
        TypeKind::Pointer(_) => return Err(crate::Error::from(format!(
            "Cannot generate C# API `{api}`: raw pointer result at `{path}` (type `{}`) has no managed ownership or lifetime contract. SafeHandle only owns registered service constructor handles; it cannot protect arbitrary returned pointers. Return an owned typed value (for example ffi::String or ffi::Vec<T>) or a registered service constructor result. Rust references returned as pointers are rejected too.",
            ty.name
        ))),
        TypeKind::Composite(composite) => {
            for field in &composite.fields {
                check(field.ty, types, api, &format!("{path}.{}", field.name), None, visiting)?;
            }
        }
        TypeKind::Array(array) => check(array.ty, types, api, &format!("{path}[]"), None, visiting)?,
        TypeKind::TypePattern(TypePattern::Result(ok, error, _)) => {
            check(*ok, types, api, &format!("{path}.Ok"), constructor_owner, visiting)?;
            check(*error, types, api, &format!("{path}.Err"), None, visiting)?;
        }
        TypeKind::DataEnum(enumeration) | TypeKind::TypePattern(TypePattern::Option(_, enumeration)) => {
            for variant in &enumeration.variants {
                for (index, field) in variant.fields.iter().enumerate() {
                    let name = if field.name.is_empty() { index.to_string() } else { field.name.clone() };
                    check(field.ty, types, api, &format!("{path}.{}.{name}", variant.name), None, visiting)?;
                }
            }
        }
        TypeKind::TypePattern(TypePattern::Vec(inner) | TypePattern::Slice(inner) | TypePattern::SliceMut(inner)
            | TypePattern::Iterator(inner) | TypePattern::AsyncIterator(inner) | TypePattern::AsyncCallback(inner)) => {
            check(*inner, types, api, &format!("{path}.element"), None, visiting)?;
        }
        TypeKind::Delegate(delegate) => check(delegate.signature.rval, types, api, &format!("{path}.callback return"), None, visiting)?,
        TypeKind::Task(task) => {
            if let Some(inner) = task.inner { check(inner, types, api, &format!("{path}.result"), None, visiting)?; }
        }
        // These patterns project to typed values with their own marshalling contract.
        // Their private ABI storage (e.g. a string's data pointer) is not a public result.
        _ => {}
    }
    visiting.remove(&id);
    Ok(())
}