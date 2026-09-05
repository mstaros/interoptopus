//! ...

use crate::lang::TypeId;
use crate::lang::types::kind::{Field, Variant};
use crate::lang::meta::Visibility;
use interoptopus::lang::meta::Docs;
use crate::pass::Outcome::Unchanged;
use crate::pass::{ModelResult, PassInfo, model};
use crate::try_extract_kind;
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    variants: HashMap<TypeId, Vec<Variant>>,
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, variants: HashMap::default() }
    }

    pub fn process(&mut self, pass_meta: &mut crate::pass::PassMeta, id_map: &model::common::id_map::Pass, rs_types: &interoptopus::inventory::Types) -> ModelResult {
        let mut outcome = Unchanged;

        for (rust_id, ty) in rs_types {
            let rust_enum = try_extract_kind!(ty, Enum);

            // Resolve C# TypeId for the enum
            let Some(cs_id) = id_map.ty(*rust_id) else { continue };

            // Skip if we've already processed this enum
            if self.variants.contains_key(&cs_id) {
                continue;
            }

            // Try to convert all variants
            let mut cs_variants = Vec::new();
            let mut all_variants_available = true;

            for rust_variant in &rust_enum.variants {
                let mut fields = Vec::new();
                for payload in rust_variant.payloads() {
                    let Some(cs_type_id) = id_map.ty(payload.ty) else {
                        pass_meta.lost_found.missing(self.info, crate::pass::MissingItem::RustType(payload.ty));
                        all_variants_available = false;
                        break;
                    };
                    fields.push(Field {
                        name: payload.name.unwrap_or_default().to_string(),
                        docs: Docs::default(),
                        visibility: Visibility::Public,
                        ty: cs_type_id,
                    });
                }
                if !all_variants_available {
                    break;
                }

                // Eligibility follows declaration shape, including a payload that resolves to void.
                let can_carry_payload = rust_variant.has_payload();

                cs_variants.push(Variant {
                    name: rust_variant.name.clone(),
                    docs: rust_variant.docs.clone(),
                    tag: rust_variant.tag,
                    fields,
                    field_names: Vec::new(),
                    case_fields: Vec::new(),
                    can_carry_payload,
                    stem: String::new(),
                    case_type: String::new(),
                });
            }

            if !all_variants_available {
                continue;
            }

            // All variants available, register the enum
            self.variants.insert(cs_id, cs_variants);
            outcome.changed();
        }

        Ok(outcome)
    }

    #[must_use]
    pub fn get(&self, ty: TypeId) -> Option<&Vec<Variant>> {
        self.variants.get(&ty)
    }
}
