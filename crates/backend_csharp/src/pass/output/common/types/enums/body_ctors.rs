//! Renders constructors, variant checks, and conversion methods for each enum
//! using the `body_ctors.cs` template.

use crate::lang::TypeId;
use crate::lang::types::kind::{TypeKind, TypePattern};
use crate::pass::{OutputResult, PassInfo, format_docs, model, output};
use interoptopus_backends::template::{Context, Value};
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    body_ctors: HashMap<TypeId, String>,
}

impl Pass {
    #[must_use]
    pub fn new(_config: Config) -> Self {
        Self { info: PassInfo { name: file!() }, body_ctors: HashMap::default() }
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

            let name = &ty.name;

            let variants: Vec<HashMap<&str, Value>> = data_enum
                .variants
                .iter()
                .map(|v| {
                    let payloads: Vec<HashMap<&str, String>> = v
                        .payloads()
                        .map(|payload| {
                            let ty = super::resolve_service_variant(payload.ty, types, mode);
                            let type_name = types.get(ty).map(|t| t.name.clone()).unwrap_or_default();
                            HashMap::from([("name", v.stem.clone()), ("type", type_name)])
                        })
                        .collect();
                    // Preserve the public single-payload AsX() return type during Steps 2–4.
                    // Step 5 must choose the multi-field return shape before widening storage.
                    let type_name = match payloads.as_slice() {
                        [] => String::new(),
                        [payload] => payload["type"].clone(),
                        _ => panic!("multi-field AsX() return contract must be selected before lifting the enum payload limit"),
                    };
                    let has_payload = !payloads.is_empty();

                    let mut m = HashMap::new();
                    m.insert("name", Value::normal_string(&v.stem));
                    m.insert("id", Value::from(v.tag as i64));
                    m.insert("has_payload", Value::from(has_payload));
                    m.insert("type", Value::normal_string(&type_name));
                    m.insert("payloads", Value::from(payloads));
                    m.insert("case_type", Value::normal_string(&v.case_type));
                    m.insert("docs", Value::normal_string(&format_docs(&v.docs.lines)));
                    m
                })
                .collect();

            // `_hasValue` exists only on struct-backed, union-projected enums; see
            // `definition.cs`. A factory produces a well-formed value, so it is the primary
            // place the flag is set. Without this write the flag stays `false` and `Value`
            // would report `null` for every value the consumer constructs.
            let writes_has_value = struct_class.is_struct(*type_id) && projection.is_union(*type_id);

            let mut context = Context::new();
            context.insert("writes_has_value", &writes_has_value);

            // Case constructors are the union *creation members*: the compiler establishes the
            // case types from public single-parameter constructors, so without them `[Union]`
            // is CS9385 "a union type must have at least one union creation member". They are
            // gated on projection alone, not on `writes_has_value`, because a class-backed
            // union needs them just as much and simply has no flag to set.
            let is_union_projected = projection.is_union(*type_id);
            context.insert("is_union_projected", &is_union_projected);
            context.insert("name", name);
            context.insert("variants", &variants);

            let rendered = templates.render("common/types/enums/body_ctors.cs", &context)?;
            self.body_ctors.insert(*type_id, rendered);
        }

        Ok(())
    }

    #[must_use]
    pub fn get(&self, type_id: TypeId) -> Option<&String> {
        self.body_ctors.get(&type_id)
    }
}
