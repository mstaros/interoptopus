//! Renders the `Unmanaged` struct for each enum using the `enum_body_unmanaged.cs` template.

use crate::lang::TypeId;
use crate::lang::types::kind::{TypeKind, TypePattern};
use crate::pass::{OutputResult, PassInfo, model, output};
use interoptopus_backends::template::{Context, Value};
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    enum_body_unmanaged: HashMap<TypeId, String>,
}

impl Pass {
    #[must_use]
    pub fn new(_config: Config) -> Self {
        Self { info: PassInfo { name: file!() }, enum_body_unmanaged: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        types: &model::common::types::all::Pass,
        managed: &output::common::conversion::unmanaged_conversion::Pass,
        projection: &model::common::types::info::projection::Pass,
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
                            let to_managed = managed.to_managed_suffix(variant_ty).to_string();
                            HashMap::from([("field", field), ("to_managed", to_managed)])
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

            let to_managed_method = managed.to_managed_name(*type_id);

            // Item 4a: `ToManaged` builds the value through the case constructors `aa2d550d` added,
            // in a switch over every variant. That establishes `_hasValue` implicitly, since those
            // constructors already follow `writes_has_value` — which is why 3c's explicit write is
            // gone and `struct_class` is no longer a parameter here.
            //
            // The default arm is the point: previously an unrecognised tag fell through every `if`
            // and returned a value carrying that tag with no payload set. It now throws.
            let is_union_projected = projection.is_union(*type_id);

            let all_variants: Vec<HashMap<&str, Value>> = data_enum
                .variants
                .iter()
                .map(|v| {
                    let payloads: Vec<HashMap<&str, String>> = model::common::types::union_names::payload_fields(v)
                        .map(|(payload_ty, field)| {
                            let variant_ty = super::resolve_service_variant(payload_ty, types, mode);
                            let to_managed = managed.to_managed_suffix(variant_ty).to_string();
                            HashMap::from([("field", field), ("to_managed", to_managed)])
                        })
                        .collect();

                    let mut m = HashMap::new();
                    m.insert("name", Value::normal_string(&v.stem));
                    m.insert("id", Value::from(v.tag as i64));
                    m.insert("case_type", Value::normal_string(&v.case_type));
                    let constant = data_enum.constant_member(v);
                    m.insert("is_constant", Value::from(constant.is_some()));
                    m.insert("constant_member", Value::normal_string(&constant.unwrap_or_default()));
                    m.insert("has_payload", Value::from(!payloads.is_empty()));
                    m.insert("payloads", Value::from(payloads));
                    m
                })
                .collect();

            let mut context = Context::new();
            context.insert("all_variants", &all_variants);
            context.insert("is_union_projected", &is_union_projected);
            context.insert("name", name);
            context.insert("to_managed_method", to_managed_method);
            context.insert("variants", &variants);
            context.insert("discriminant_type", data_enum.discriminant_type.cs_name());

            let rendered = templates.render("common/types/enums/body_unmanaged.cs", &context)?;
            self.enum_body_unmanaged.insert(*type_id, rendered);
        }

        Ok(())
    }

    #[must_use]
    pub fn get(&self, type_id: TypeId) -> Option<&String> {
        self.enum_body_unmanaged.get(&type_id)
    }
}
