//! Renders the union member trio: `HasValue`, `Value` and `TryGetValue` (item 3c).
//!
//! Scoped by the same per-enum eligibility rule as 3b: a `DataEnum` with no payload-carrying
//! variant receives no union machinery, so it gets none of these. Emitting them there would
//! declare `Value` on a type with no cases behind it.
//!
//! **Struct-backed and class-backed differ, and the difference is not cosmetic.**
//! `docs/csharp-unions.md` Step 3 "Class-backed": there is no empty class instance —
//! `default(E)` is a null reference and every non-null instance is valid — so `HasValue` is a
//! constant `true` and `Value` is never null. A struct always has `default(E)`, which is a
//! legal C# value that corresponds to no Rust variant, so both must consult `_hasValue`.
//!
//! That consultation is a soundness obligation rather than a nicety (item 4c): the compiler
//! assumes *"for struct unions, `default` produces a `Value` of null"* and reasons about
//! exhaustiveness on it. A `Value` that materialised a case for `default(E)` would make the
//! compiler's exhaustiveness reasoning wrong.
//!
//! `Value` boxes a `readonly record struct` on each access, so reference identity is **not**
//! stable across accesses — value-stable, not reference-stable. That is deliberate. An eager
//! `_boxed` field would allocate inside `ToManaged`, on every boundary crossing, which is the
//! cost this representation exists to avoid. Pattern matching goes through `TryGetValue` and
//! does not allocate.

use crate::lang::TypeId;
use crate::lang::types::kind::{TypeKind, TypePattern};
use crate::pass::{OutputResult, PassInfo, model, output};
use interoptopus_backends::template::{Context, Value};
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    union_members: HashMap<TypeId, String>,
}

impl Pass {
    #[must_use]
    pub fn new(_config: Config) -> Self {
        Self { info: PassInfo { name: file!() }, union_members: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        types: &model::common::types::all::Pass,
        struct_class: &model::common::types::info::struct_class::Pass,
        projection: &model::common::types::info::projection::Pass,
    ) -> OutputResult {
        let templates = output_master.templates();

        for (type_id, ty) in types.iter() {
            let type_kind = &ty.kind;
            let Some(data_enum) = model::common::types::union_names::data_enum(type_kind) else { continue };

            if !projection.is_union(*type_id) {
                continue;
            }

            let is_struct = struct_class.is_struct(*type_id);

            let variants: Vec<HashMap<&str, Value>> = data_enum
                .variants
                .iter()
                .map(|v| {
                    let mut m = HashMap::new();
                    m.insert("stem", Value::normal_string(&v.stem));
                    m.insert("case_type", Value::normal_string(&v.case_type));
                    m.insert("tag", Value::from(v.tag));
                    let payloads: Vec<_> = v.payloads().map(|_| HashMap::from([("name", v.stem.clone())])).collect();
                    m.insert("payloads", Value::from(payloads));
                    m
                })
                .collect();

            let mut context = Context::new();
            context.insert("is_struct", &is_struct);
            context.insert("variants", &variants);

            let rendered = templates.render("common/types/enums/body_union_members.cs", &context)?;
            self.union_members.insert(*type_id, rendered);
        }

        Ok(())
    }

    #[must_use]
    pub fn get(&self, type_id: TypeId) -> Option<&String> {
        self.union_members.get(&type_id)
    }
}