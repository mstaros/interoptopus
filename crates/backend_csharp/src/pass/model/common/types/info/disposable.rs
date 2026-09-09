//! Determines whether a type should implement `IDisposable` in C#.
//!
//! Native-owning leaves and containers of those leaves require disposal,
//! including managed-only wire payloads. Ownership propagates to a fixed point.

use crate::lang::TypeId;
use crate::lang::types::ManagedConversion;
use crate::pass::Outcome::Unchanged;
use crate::pass::{ModelResult, PassInfo, model};
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    disposable: HashMap<TypeId, bool>,
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, disposable: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        managed_conversion: &model::common::types::info::managed_conversion::Pass,
        types: &model::common::types::all::Pass,
    ) -> ModelResult {
        use crate::lang::types::kind::{TypeKind, Variant};
        use crate::lang::types::kind::wire::WireOnly;

        let mut outcome = Unchanged;

        // Recompute: wire-only containers can be classified after their native leaves.
        // Managed collections need recursive cleanup, but string itself owns no native data.
        for (type_id, ty) in types.iter() {
            let Some(mc) = managed_conversion.managed_conversion(*type_id) else { continue };
            let child_owns = |id| self.is_disposable(id).unwrap_or(false);
            let is_disposable = match &ty.kind {
                TypeKind::WireOnly(WireOnly::String) => false,
                TypeKind::WireOnly(WireOnly::Vec(inner) | WireOnly::Nullable(inner)) => child_owns(*inner),
                TypeKind::WireOnly(WireOnly::Map(key, value)) => child_owns(*key) || child_owns(*value),
                TypeKind::Array(array) => child_owns(array.ty),
                TypeKind::Composite(composite) | TypeKind::WireOnly(WireOnly::Composite(composite)) => {
                    composite.fields.iter().any(|field| child_owns(field.ty))
                }
                TypeKind::DataEnum(data_enum) => data_enum.variants.iter().flat_map(Variant::payloads).any(|field| child_owns(field.ty)),
                _ => matches!(mc, ManagedConversion::Into),
            };
            if self.disposable.get(type_id) != Some(&is_disposable) {
                self.disposable.insert(*type_id, is_disposable);
                outcome.changed();
            }
        }

        Ok(outcome)
    }

    #[must_use]
    pub fn is_disposable(&self, ty: TypeId) -> Option<bool> {
        self.disposable.get(&ty).copied()
    }
}
