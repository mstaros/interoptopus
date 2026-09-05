//! Renders `ExceptionForVariant()` method for each enum using the
//! `body_exception_for_variant.cs` template.

use crate::lang::TypeId;
use crate::lang::types::kind::{TypeKind, TypePattern};
use crate::pass::{OutputResult, PassInfo, model, output};
use interoptopus_backends::template::{Context, Value};
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    body_exception_for_variant: HashMap<TypeId, String>,
}

impl Pass {
    #[must_use]
    pub fn new(_config: Config) -> Self {
        Self { info: PassInfo { name: file!() }, body_exception_for_variant: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        types: &model::common::types::all::Pass,
        struct_class: &model::common::types::info::struct_class::Pass,
        projection: &model::common::types::info::projection::Pass,
        mode: crate::pass::OperationMode,
    ) -> OutputResult {
        let templates = output_master.templates();

        for (type_id, ty) in types.iter() {
            let type_kind = &ty.kind;
            let Some(data_enum) = model::common::types::union_names::data_enum(type_kind) else { continue };

            let variants: Vec<HashMap<&str, Value>> = data_enum
                .variants
                .iter()
                .map(|v| {
                    let payloads: Vec<HashMap<&str, String>> = model::common::types::union_names::payload_fields(v)
                        .map(|(payload_ty, field)| {
                            let ty = super::resolve_service_variant(payload_ty, types, mode);
                            let type_name = types.get(ty).map(|t| t.name.clone()).unwrap_or_default();
                            HashMap::from([("field", field), ("type", type_name)])
                        })
                        .collect();
                    // Preserve EnumException<TPayload> for existing one-field cases.
                    let has_payload = !payloads.is_empty();
                    let is_multi = payloads.len() > 1;

                    let mut m = HashMap::new();
                    m.insert("name", Value::normal_string(&v.stem));
                    m.insert("id", Value::from(v.tag as i64));
                    m.insert("has_payload", Value::from(has_payload));
                    m.insert("is_multi", Value::from(is_multi));
                    m.insert("case_type", Value::normal_string(&v.case_type));
                    m.insert("payloads", Value::from(payloads));
                    m
                })
                .collect();

            let has_empty_state = struct_class.is_struct(*type_id) && projection.is_union(*type_id);

            let mut context = Context::new();
            context.insert("has_empty_state", &has_empty_state);
            context.insert("name", &ty.name);
            context.insert("variants", &variants);

            let rendered = templates.render("common/types/enums/body_exception_for_variant.cs", &context)?;
            self.body_exception_for_variant.insert(*type_id, rendered);
        }

        Ok(())
    }

    #[must_use]
    pub fn get(&self, type_id: TypeId) -> Option<&String> {
        self.body_exception_for_variant.get(&type_id)
    }
}
