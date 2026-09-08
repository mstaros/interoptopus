//! Creates `ByRef` and `ByOut` sibling types for each existing `IntPtr` pointer type.
//!
//! For every `Pointer::IntPtr(pointee, _)` that is fully resolved in the `all` pass,
//! this pass creates two new types — `Pointer::ByRef(pointee)` and `Pointer::ByOut(pointee)` —
//! with fresh `TypeIds` derived from the original. It registers them in the kind, name,
//! and all passes, and registers the family in the overload all pass. Read-only pointers
//! to types requiring custom marshalling use C# `in`; all other by-reference pointers
//! retain `ref`.

use crate::lang::TypeId;
use crate::lang::meta::{Emission, Visibility};
use crate::lang::types::kind::{DelegateKind, IntPtrHint, Pointer, PointerKind, TypeKind, TypePattern, Util};
use crate::lang::types::{Decorators, ManagedConversion, OverloadFamily, ParamDecorator, PointerFamily, Type};
use crate::pass::Outcome::Unchanged;
use crate::pass::{ModelResult, PassInfo, model};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    /// Tracks which `IntPtr` types we've already processed.
    processed: HashSet<TypeId>,
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, processed: HashSet::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        kinds: &mut model::common::types::kind::Pass,
        names: &mut model::common::types::names::Pass,
        types: &mut model::common::types::all::Pass,
        managed_conversion: &model::common::types::info::managed_conversion::Pass,
        overloads: &mut model::rust::types::overload::all::Pass,
    ) -> ModelResult {
        let mut outcome = Unchanged;

        // Collect IntPtr types that are fully resolved in the map pass
        let intptr_types: Vec<(TypeId, TypeId, IntPtrHint)> = kinds
            .iter()
            .filter_map(|(&type_id, kind)| match kind {
                TypeKind::Pointer(Pointer { kind: PointerKind::IntPtr(hint), target }) => Some((type_id, *target, *hint)),
                _ => None,
            })
            .collect();

        for (intptr_id, pointee_id, hint) in intptr_types {
            if self.processed.contains(&intptr_id) {
                continue;
            }

            // Wait until the IntPtr type is fully resolved in the map pass
            if types.get(intptr_id).is_none() {
                continue;
            }

            let Some(pointee_type) = types.get(pointee_id) else {
                continue;
            };

            let Some(conversion) = managed_conversion.managed_conversion(pointee_id) else {
                continue;
            };

            // Also need the pointee to be named
            let Some(pointee_name) = names.get(pointee_id) else {
                continue;
            };

            let pointee_name = pointee_name.clone();
            let by_ref_decorator =
                if hint == IntPtrHint::Read && conversion != ManagedConversion::AsIs && supports_in_marshaller(&pointee_type.kind) {
                    ParamDecorator::In { marshaller: format!("{pointee_name}.InMarshallerMeta") }
            } else {
                ParamDecorator::Ref
            };

            // Derive new TypeIds for ByRef and ByOut variants
            let by_ref_id = TypeId::from_id(intptr_id.id().derive(0x_6279_7265_665F_7369)); // "byref_si"
            let by_out_id = TypeId::from_id(intptr_id.id().derive(0x_6279_6F75_745F_7369)); // "byout_si"

            // Register kinds
            kinds.set(by_ref_id, TypeKind::Pointer(Pointer { kind: PointerKind::ByRef, target: pointee_id }));
            kinds.set(by_out_id, TypeKind::Pointer(Pointer { kind: PointerKind::ByOut, target: pointee_id }));

            // Register names (base pointee name, without ref/out prefix)
            names.set(by_ref_id, pointee_name.clone());
            names.set(by_out_id, pointee_name.clone());

            // Register in the all pass so they're fully resolved
            types.set(
                by_ref_id,
                Type {
                    emission: Emission::Builtin,
                    name: pointee_name.clone(),
                    visibility: Visibility::Public,
                    docs: Vec::new(),
                    kind: TypeKind::Pointer(Pointer { kind: PointerKind::ByRef, target: pointee_id }),
                    decorators: Decorators { param: Some(by_ref_decorator), ..Default::default() },
                },
            );
            types.set(
                by_out_id,
                Type {
                    emission: Emission::Builtin,
                    name: pointee_name.clone(),
                    visibility: Visibility::Public,
                    docs: Vec::new(),
                    kind: TypeKind::Pointer(Pointer { kind: PointerKind::ByOut, target: pointee_id }),
                    decorators: Decorators { param: Some(ParamDecorator::Out), ..Default::default() },
                },
            );

            // Register family in the overload all pass
            let family = Arc::new(OverloadFamily::Pointer(PointerFamily { intptr: intptr_id, by_ref: by_ref_id, by_out: by_out_id }));

            overloads.register(intptr_id, Arc::clone(&family));
            overloads.register(by_ref_id, Arc::clone(&family));
            overloads.register(by_out_id, family);

            self.processed.insert(intptr_id);
            outcome.changed();
        }

        Ok(outcome)
    }
}

fn supports_in_marshaller(kind: &TypeKind) -> bool {
    match kind {
        TypeKind::Composite(_) | TypeKind::DataEnum(_) => true,
        TypeKind::Delegate(delegate) => matches!(delegate.kind, DelegateKind::Class),
        TypeKind::TypePattern(pattern) => matches!(
            pattern,
            TypePattern::Utf8String
                | TypePattern::Slice(_)
                | TypePattern::SliceMut(_)
                | TypePattern::Vec(_)
                | TypePattern::Iterator(_)
                | TypePattern::Option(_, _)
                | TypePattern::Result(_, _, _)
                | TypePattern::Wire(_)
        ),
        TypeKind::Util(Util::WireBuffer) => true,
        _ => false,
    }
}
