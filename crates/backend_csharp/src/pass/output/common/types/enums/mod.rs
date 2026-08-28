pub mod all;
pub mod body;
pub mod body_as_unmanaged;
pub mod body_case_types;
pub mod body_ctors;
pub mod body_exception_for_variant;
pub mod body_from_call;
pub mod body_to_unmanaged;
pub mod body_tostring;
pub mod body_union_members;
pub mod body_unmanaged;
pub mod body_unmanaged_variant;
pub mod definition;

use crate::lang::TypeId;
use crate::lang::types::kind::TypeKind;
use crate::pass::model::common::types::info::nullable::NullPolicy;
use crate::pass::{OperationMode, model};

/// If `ty` is a pointer to a service type, return the service `TypeId` so that
/// enum rendering uses the managed service class and `Service.Unmanaged` instead
/// of raw `IntPtr`. Only applicable for reverse interop (`Plugin` mode) where
/// services are managed C# objects; in forward interop (`Rust` mode) services
/// are opaque Rust pointers and should remain as `IntPtr`.
fn resolve_service_variant(ty: TypeId, types: &model::common::types::all::Pass, mode: OperationMode) -> TypeId {
    if mode == OperationMode::Rust {
        return ty;
    }
    let Some(t) = types.get(ty) else { return ty };
    let TypeKind::Pointer(p) = &t.kind else { return ty };
    let Some(target) = types.get(p.target) else { return ty };
    if matches!(&target.kind, TypeKind::Service) { p.target } else { ty }
}

/// Wraps a variant payload's conversion suffix according to the payload's [`NullPolicy`].
///
/// A union's payload is reached through a bare suffix — `_Ok.ToUnmanaged()` — so a payload that
/// is itself a **class-backed union** dereferences on null exactly as a composite field does.
/// That is position (c) of `docs/csharp-unions.md` Open items 1, measured in `c70efeb`, and the
/// decision there is `InvalidOperationException` rather than `?? default`: a zeroed `Unmanaged`
/// would be discriminant 0, a fabricated variant crossing FFI.
///
/// An empty suffix means the payload converts as-is, so there is nothing to guard — and `?` on a
/// value type is `CS0023`, which is how the first attempt at this failed.
fn guard_null_payload(
    suffix: &str,
    nullable: &model::common::types::info::nullable::Pass,
    payload: TypeId,
    union_name: &str,
    variant_stem: &str,
) -> String {
    if suffix.is_empty() {
        return suffix.to_string();
    }

    match nullable.null_policy(payload).unwrap_or(NullPolicy::NotNullable) {
        NullPolicy::NotNullable => suffix.to_string(),
        NullPolicy::SubstituteDefault => format!("?{suffix} ?? default"),
        NullPolicy::Throw => format!(
            "?{suffix} ?? throw new InvalidOperationException(\"Cannot marshal {union_name}.{variant_stem}: its payload is null and corresponds to no Rust variant. Construct it through a case constructor or factory.\")"
        ),
    }
}
