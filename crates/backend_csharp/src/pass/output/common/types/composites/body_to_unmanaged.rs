//! Renders the `ToUnmanaged`/`IntoUnmanaged` method for each composite using the
//! `body_to_unmanaged.cs` template.
//!
//! Shared between the Rust and .NET pipelines.

use crate::lang::TypeId;
use crate::lang::types::kind::TypeKind;
use crate::pass::model::common::types::info::nullable::NullPolicy;
use crate::pass::{OutputResult, PassInfo, model, output};
use interoptopus_backends::template::Context;
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    body_to_unmanaged: HashMap<TypeId, String>,
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, body_to_unmanaged: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        types: &model::common::types::all::Pass,
        managed: &output::common::conversion::unmanaged_conversion::Pass,
        field_conversions: &output::common::conversion::fields::Pass,
        nullable: &model::common::types::info::nullable::Pass,
    ) -> OutputResult {
        let templates = output_master.templates();

        for (type_id, ty) in types.iter() {
            let TypeKind::Composite(composite) = &ty.kind else { continue };

            let name = &ty.name;
            let to_unmanaged = managed.to_unmanaged_name(*type_id);

            let fields: Vec<HashMap<&str, String>> = composite
                .fields
                .iter()
                .map(|f| {
                    let suffix = managed.to_unmanaged_suffix(f.ty);

                    // An empty suffix means the field converts as-is, so there is nothing to
                    // guard: `?` on a value type does not compile.
                    let to_unmanaged = if suffix.is_empty() {
                        suffix.to_string()
                    } else {
                        match nullable.null_policy(f.ty).unwrap_or(NullPolicy::NotNullable) {
                            NullPolicy::NotNullable => suffix.to_string(),
                            NullPolicy::SubstituteDefault => format!("?{suffix} ?? default"),

                            // A zeroed `Unmanaged` for a union is discriminant 0 — a fabricated
                            // variant crossing FFI. `docs/csharp-unions.md` Open items 1.
                            NullPolicy::Throw => format!(
                                "?{suffix} ?? throw new InvalidOperationException(\"Cannot marshal {name}.{field}: it is null and corresponds to no Rust variant. Construct it through a case constructor or factory.\")",
                                field = f.name
                            ),
                        }
                    };

                    let mut m = HashMap::new();
                    m.insert("name", f.name.clone());
                    m.insert("to_unmanaged", to_unmanaged);
                    if let Some(custom) = field_conversions.custom_to_unmanaged(*type_id, &f.name) {
                        m.insert("custom_to_unmanaged", custom.to_string());
                    }
                    m
                })
                .collect();

            let mut context = Context::new();
            context.insert("name", name);
            context.insert("to_unmanaged", to_unmanaged);
            context.insert("fields", &fields);

            let rendered = templates.render("common/types/composite/body_to_unmanaged.cs", &context)?;
            self.body_to_unmanaged.insert(*type_id, rendered);
        }

        Ok(())
    }

    #[must_use]
    pub fn get(&self, type_id: TypeId) -> Option<&String> {
        self.body_to_unmanaged.get(&type_id)
    }
}
