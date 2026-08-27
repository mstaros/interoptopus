//! Decides how a type is projected into C#, independently of what kind it is.
//!
//! Today this answers two related questions, both previously computed inline in
//! `output/common/types/enums/body.rs` at render time and shared with nobody — which is why the
//! four unmanaged output passes cannot consult them; see `Issues.md` `5d1ae4c7`.
//!
//! **They are two questions, not one.** `crosses_ffi` is false for a `DataEnum` with a `WireOnly`
//! variant payload *and* for a `Result`/`Option` whose `Ok` side is a `Service`. But only the
//! first also forces the type non-disposable — a wire-only payload is GC-managed and holds no
//! native resource, whereas a Service-backed `Result` still owns one. Collapsing the two would
//! silently make Service-backed results non-disposable.
//!
//! **Why this recomputes rather than writing once.** `struct_class` and `disposable` may skip a
//! type they have already answered, because their input is `managed_conversion`, which returns
//! `None` while a type is still being resolved. This pass reads raw `TypeKind`s instead — a
//! `WireOnly` variant payload, a `Service` on a `Result`'s `Ok` side — and a kind is *always*
//! something. There is no not-ready signal to defer on, so an answer cached early would go stale
//! when a later kind pass reclassifies a payload. Recomputing each round and reporting `changed`
//! only on difference converges correctly under the same fixed-point contract.

use crate::lang::TypeId;
use crate::lang::types::kind::{TypeKind, TypePattern};
use crate::pass::Outcome::Unchanged;
use crate::pass::{ModelResult, PassInfo, model};
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    crosses_ffi: HashMap<TypeId, bool>,
    has_wire_only_payload: HashMap<TypeId, bool>,
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, crosses_ffi: HashMap::default(), has_wire_only_payload: HashMap::default() }
    }

    pub fn process(&mut self, _pass_meta: &mut crate::pass::PassMeta, types: &model::common::types::all::Pass) -> ModelResult {
        let mut outcome = Unchanged;

        for (type_id, ty) in types.iter() {
            let kind = &ty.kind;

            // A `DataEnum` carrying a `WireOnly` variant payload (e.g. `S(String)`) has no
            // FFI-safe `Unmanaged` form — it only flows through `Wire<T>`.
            let wire_only = match kind {
                TypeKind::DataEnum(de) => de
                    .variants
                    .iter()
                    .any(|v| v.ty.is_some_and(|t| matches!(types.get(t).map(|x| &x.kind), Some(TypeKind::WireOnly(_))))),
                _ => false,
            };

            // A `Result`/`Option` whose `Ok` side is a `Service` is managed-only for a different
            // reason: there is nothing to mirror, but it still owns a native resource.
            let ok_is_service = match kind {
                TypeKind::TypePattern(TypePattern::Result(ok_ty, _, _) | TypePattern::Option(ok_ty, _)) => {
                    types.get(*ok_ty).is_some_and(|t| matches!(&t.kind, TypeKind::Service))
                }
                _ => false,
            };

            let crosses_ffi = !(wire_only || ok_is_service);

            // Recompute-and-compare, not write-once. See the module comment.
            if self.crosses_ffi.get(type_id) != Some(&crosses_ffi) {
                self.crosses_ffi.insert(*type_id, crosses_ffi);
                outcome.changed();
            }
            if self.has_wire_only_payload.get(type_id) != Some(&wire_only) {
                self.has_wire_only_payload.insert(*type_id, wire_only);
                outcome.changed();
            }
        }

        Ok(outcome)
    }

    /// Whether the type needs an `Unmanaged` mirror and a marshaller.
    ///
    /// `None` means the pass has not answered for this type yet, and is deliberately not the same
    /// as `Some(false)`: this value decides whether machinery is emitted at all, so collapsing
    /// absent into "no" would silently drop a type's marshalling.
    #[must_use]
    pub fn crosses_ffi(&self, ty: TypeId) -> Option<bool> {
        self.crosses_ffi.get(&ty).copied()
    }

    /// Whether the type is a `DataEnum` carrying a `WireOnly` variant payload.
    ///
    /// Narrower than `!crosses_ffi`: it excludes the Service-backed `Result` case, which is also
    /// managed-only but *is* still disposable.
    #[must_use]
    pub fn has_wire_only_payload(&self, ty: TypeId) -> Option<bool> {
        self.has_wire_only_payload.get(&ty).copied()
    }
}
