//! ...

use crate::lang::TypeId;
use crate::lang::types::kind::Variant;
use crate::pass::Outcome::Unchanged;
use crate::pass::{ModelResult, PassInfo, model};
use crate::try_extract_kind;
use interoptopus::lang;
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
                let (tag, cs_variant_type_id) = match &rust_variant.kind {
                    lang::types::VariantKind::Unit => (rust_variant.tag, None),
                    lang::types::VariantKind::Tuple(rust_type_id) => {
                        // Tag comes from the variant, never its position. See `Issues.md` `09b82d44`.
                        let Some(cs_type_id) = id_map.ty(*rust_type_id) else {
                            // Variant type not yet mapped, skip this enum for now
                            pass_meta.lost_found.missing(self.info, crate::pass::MissingItem::RustType(*rust_type_id));
                            all_variants_available = false;
                            break;
                        };
                        (rust_variant.tag, Some(cs_type_id))
                    }
                };

                // Asks whether the *declaration* has a payload slot, not whether it resolved to one.
                // A `Tuple(())` variant is payload-capable even though its C# payload is absent.
                let can_carry_payload = matches!(rust_variant.kind, lang::types::VariantKind::Tuple(_));

                cs_variants.push(Variant {
                    name: rust_variant.name.clone(),
                    docs: rust_variant.docs.clone(),
                    tag,
                    ty: cs_variant_type_id,
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
