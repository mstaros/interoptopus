//! Renders the `AsUnmanaged` method for each enum using the `body_as_unmanaged.cs` template.

use crate::lang::TypeId;
use crate::lang::types::kind::{TypeKind, TypePattern};
use crate::pass::{OutputResult, PassInfo, model, output};
use interoptopus_backends::template::{Context, Value};
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    body_as_unmanaged: HashMap<TypeId, String>,
}

impl Pass {
    #[must_use]
    pub fn new(_config: Config) -> Self {
        Self { info: PassInfo { name: file!() }, body_as_unmanaged: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        types: &model::common::types::all::Pass,
        managed: &output::common::conversion::unmanaged_conversion::Pass,
        struct_class: &model::common::types::info::struct_class::Pass,
        projection: &model::common::types::info::projection::Pass,
        nullable: &model::common::types::info::nullable::Pass,
        mode: crate::pass::OperationMode,
    ) -> OutputResult {
        let templates = output_master.templates();

        for (type_id, ty) in types.iter() {
            let type_kind = &ty.kind;
            let Some(data_enum) = model::common::types::union_names::data_enum(type_kind) else { continue };

            let name = &ty.name;

            let variants: Vec<HashMap<&str, Value>> = data_enum
                .variants
                .iter()
                .filter_map(|v| {
                    let payloads: Vec<HashMap<&str, String>> = model::common::types::union_names::payload_fields(v)
                        .map(|(payload_ty, field)| {
                            let variant_ty = super::resolve_service_variant(payload_ty, types, mode);
                            let suffix = managed.as_unmanaged_suffix(variant_ty);
                            let as_unmanaged = super::guard_null_payload(suffix, nullable, variant_ty, name, &v.stem);
                            HashMap::from([("field", field), ("as_unmanaged", as_unmanaged)])
                        })
                        .collect();
                    if payloads.is_empty() {
                        return None;
                    }
                    Some(HashMap::from([
                        ("name", Value::normal_string(&v.stem)),
                        ("id", Value::from(v.tag)),
                        ("payloads", Value::from(payloads)),
                    ]))
                })
                .collect();

            let has_empty_state = struct_class.is_struct(*type_id) && projection.is_union(*type_id);

            let mut context = Context::new();
            context.insert("has_empty_state", &has_empty_state);
            context.insert("name", name);
            context.insert("variants", &variants);

            let rendered = templates.render("common/types/enums/body_as_unmanaged.cs", &context)?;
            self.body_as_unmanaged.insert(*type_id, rendered);
        }

        Ok(())
    }

    #[must_use]
    pub fn get(&self, type_id: TypeId) -> Option<&String> {
        self.body_as_unmanaged.get(&type_id)
    }
}
