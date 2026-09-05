//! Renders the nested case types that union projection introduces (item 3b).
//!
//! One `readonly record struct` per variant, nested inside the union type. The names
//! come from `Variant::case_type`, already allocated and collision-resolved by
//! `union_names`; this pass only emits them.
//!
//! `public` is stated explicitly rather than inherited. A nested type in C# defaults
//! to `private`, which is not merely suboptimal but unusable: the case type could not
//! be named outside the union, so pattern matching could not mention it and
//! exhaustiveness checking would go with it. `internal` fails the same way one scope
//! out. That is item 3f, settled in `docs/csharp-unions.md` Open items #3.
//!
//! The positional record parameter carries the payload. It is what gives both the
//! single-parameter constructor the compiler-provided union conversion needs (item 3e)
//! and the deconstruction in `E.BCase(var x) => ...`.

use crate::lang::TypeId;
use crate::lang::types::kind::{TypeKind, TypePattern};
use crate::pass::{OutputResult, PassInfo, model, output};
use interoptopus_backends::template::Context;
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    case_types: HashMap<TypeId, Vec<String>>,
}

impl Pass {
    #[must_use]
    pub fn new(_config: Config) -> Self {
        Self { info: PassInfo { name: file!() }, case_types: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        types: &model::common::types::all::Pass,
        projection: &model::common::types::info::projection::Pass,
        mode: crate::pass::OperationMode,
    ) -> OutputResult {
        let templates = output_master.templates();

        for (type_id, ty) in types.iter() {
            let type_kind = &ty.kind;
            let Some(data_enum) = model::common::types::union_names::data_enum(type_kind) else { continue };

            // Eligibility rule from `docs/csharp-unions.md` "Two layers, two rules": a
            // `DataEnum` with no payload-carrying variant receives no union machinery.
            //
            // The test is per *enum*, not per variant. An enum that is projected as a
            // union gives every variant a case type, unit variants included — an empty
            // case type is what keeps a mixed enum exhaustive in the compiler-checked
            // layer, and folding unit variants together was considered and dropped.
            if !projection.is_union(*type_id) {
                continue;
            }

            let mut rendered_case_types = Vec::new();

            for variant in &data_enum.variants {
                let payloads: Vec<HashMap<&str, String>> = variant
                    .payloads()
                    .enumerate()
                    .filter_map(|(index, payload)| {
                        let variant_ty = super::resolve_service_variant(payload.ty, types, mode);
                        let ty_name = types.get(variant_ty).map(|t| t.name.clone())?;
                        Some(HashMap::from([("type", ty_name), ("case_field", variant.case_fields[index].clone())]))
                    })
                    .collect();

                let mut context = Context::new();
                context.insert("case_type", &variant.case_type);
                context.insert("payloads", &payloads);

                let rendered = templates.render("common/types/enums/body_case_types.cs", &context)?;
                rendered_case_types.push(rendered);
            }

            self.case_types.insert(*type_id, rendered_case_types);
        }

        Ok(())
    }

    #[must_use]
    pub fn get(&self, type_id: TypeId) -> Option<&[String]> {
        self.case_types.get(&type_id).map(std::vec::Vec::as_slice)
    }
}