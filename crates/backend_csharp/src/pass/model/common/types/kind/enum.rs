//! Creates `DataEnum` types from computed enum variants.

use crate::lang::types::kind::{DataEnum, Primitive, TypeKind};
use crate::pass::Outcome::Unchanged;
use crate::pass::{ModelResult, PassInfo, model};
use crate::try_resolve;
use interoptopus::lang;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
}

/// Map a core `Primitive` (from `Enum.repr`) to the C# backend `Primitive`.
fn cs_primitive(p: lang::types::Primitive) -> Primitive {
    match p {
        lang::types::Primitive::Void => Primitive::Void,
        lang::types::Primitive::Bool => Primitive::Bool,
        lang::types::Primitive::U8 => Primitive::Byte,
        lang::types::Primitive::U16 => Primitive::UShort,
        lang::types::Primitive::U32 => Primitive::UInt,
        lang::types::Primitive::U64 => Primitive::ULong,
        lang::types::Primitive::Usize => Primitive::NUInt,
        lang::types::Primitive::I8 => Primitive::SByte,
        lang::types::Primitive::I16 => Primitive::Short,
        lang::types::Primitive::I32 => Primitive::Int,
        lang::types::Primitive::I64 => Primitive::Long,
        lang::types::Primitive::Isize => Primitive::NInt,
        lang::types::Primitive::F32 => Primitive::Float,
        lang::types::Primitive::F64 => Primitive::Double,
    }
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() } }
    }

    pub fn process(
        &mut self,
        pass_meta: &mut crate::pass::PassMeta,
        id_map: &model::common::id_map::Pass,
        kinds: &mut model::common::types::kind::Pass,
        variants_pass: &model::common::types::kind::enum_variants::Pass,
        rs_types: &interoptopus::inventory::Types,
    ) -> ModelResult {
        let mut outcome = Unchanged;

        for (rust_id, ty) in rs_types {
            let lang::types::TypeKind::Enum(rust_enum) = &ty.kind else {
                continue;
            };

            let cs_id = try_resolve!(id_map.ty(*rust_id), pass_meta, self.info, crate::pass::MissingItem::RustType(*rust_id));
            let variants = try_resolve!(variants_pass.get(cs_id), pass_meta, self.info, crate::pass::MissingItem::CsType(cs_id));

            // Check if we've already processed this type
            if kinds.contains(&cs_id) {
                continue;
            }

            // Determine discriminant type from Rust repr
            // The width itself is decided by `Repr::discriminant_primitive` so that every
            // backend answers the same thing; this only maps that answer into the C#
            // primitive set. This pass and the wire pass previously each carried their own
            // fallback and disagreed, answering `int` here and `uint` there.
            let discriminant_type = cs_primitive(rust_enum.repr.discriminant_primitive());

            // Create the data enum
            let data_enum = DataEnum { variants: variants.clone(), discriminant_type };

            kinds.set(cs_id, TypeKind::DataEnum(data_enum));
            outcome.changed();
        }

        Ok(outcome)
    }
}
