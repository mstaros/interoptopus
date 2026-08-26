//! Renders enum type definitions using the `definition.cs` template.
//!
//! Shared between the Rust and .NET pipelines. Each data-enum (including
//! `Result` / `Option` pattern types) gets a `partial struct` (or `partial class`)
//! declaration listing its variant tag and typed variant fields.

use crate::lang::TypeId;
use crate::lang::types::kind::{TypeKind, TypePattern};
use crate::pass::{OutputResult, PassInfo, format_docs, model, output};
use interoptopus_backends::template::Context;
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    enum_ty: HashMap<TypeId, String>,
}

impl Pass {
    #[must_use]
    pub fn new(_config: Config) -> Self {
        Self { info: PassInfo { name: file!() }, enum_ty: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        types: &model::common::types::all::Pass,
        struct_class: &model::common::types::info::struct_class::Pass,
        disposable: &model::common::types::info::disposable::Pass,
        mode: crate::pass::OperationMode,
    ) -> OutputResult {
        let templates = output_master.templates();

        for (type_id, ty) in types.iter() {
            let type_kind = &ty.kind;
            let data_enum = match type_kind {
                TypeKind::DataEnum(e) => e,
                TypeKind::TypePattern(TypePattern::Result(_, _, e)) => e,
                TypeKind::TypePattern(TypePattern::Option(_, e)) => e,
                _ => continue,
            };

            let name = &ty.name;
            let docs = format_docs(&ty.docs);
            let visibility = ty.visibility.to_string();
            let is_disposable = disposable.is_disposable(*type_id).unwrap_or(false);

            let ty = *type_id;
            let is_struct = struct_class.is_struct(ty);

            // Same eligibility rule as 3b (`docs/csharp-unions.md` "Two layers, two rules"):
            // a `DataEnum` with no payload-carrying variant receives no union machinery, and
            // `_hasValue` is union machinery. Without this the field would be emitted on
            // unit-only enums where nothing reads it, which is CS0169 — see `Issues.md`
            // `8f4c1e2a`, where six of the nineteen were exactly that.
            let is_union_projected = data_enum.variants.iter().any(|v| v.can_carry_payload);
            let struct_or_class = if is_struct { "struct" } else { "class" };

            let variants: Vec<HashMap<&str, String>> = data_enum
                .variants
                .iter()
                .filter_map(|v| {
                    let ty = super::resolve_service_variant(v.ty?, types, mode);
                    let ty_name = types.get(ty).map(|t| &t.name)?;
                    let mut m = HashMap::new();
                    m.insert("name", v.stem.clone());
                    m.insert("type", ty_name.clone());
                    Some(m)
                })
                .collect();

            let mut context = Context::new();
            context.insert("name", name);
            context.insert("struct_or_class", struct_or_class);
            context.insert("variants", &variants);
            context.insert("docs", &docs);
            context.insert("is_disposable", &is_disposable);
            context.insert("visibility", &visibility);
            context.insert("is_struct", &is_struct);
            context.insert("is_union_projected", &is_union_projected);
            context.insert("discriminant_type", data_enum.discriminant_type.cs_name());

            let rendered = templates.render("common/types/enums/definition.cs", &context)?;
            self.enum_ty.insert(*type_id, rendered);
        }

        Ok(())
    }

    #[must_use]
    pub fn get(&self, type_id: TypeId) -> Option<&String> {
        self.enum_ty.get(&type_id)
    }
}
