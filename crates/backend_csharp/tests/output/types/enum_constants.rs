use interoptopus::inventory::{RustInventory, TypeId};
use interoptopus::lang::meta::{Docs, Emission, FileEmission, Visibility};
use interoptopus::lang::types::{Enum, Layout, Primitive, Repr, Type, TypeInfo, TypeKind, Variant, VariantKind};
use interoptopus::extra_type;
use interoptopus_csharp::RustLibrary;
use interoptopus_csharp::config::HeaderConfig;

fn generate(base: Primitive, name: &str, unit_count: usize) -> String {
    let mut inventory = RustInventory::new();
    let _ = inventory.register(extra_type!(u32));
    let mut variants = vec![Variant::new("First", 2, VariantKind::Unit)];
    if unit_count == 2 {
        variants.push(Variant::new("Last", 9, VariantKind::Unit));
    }
    variants.extend([
        Variant::new("Payload", 11, VariantKind::Tuple(vec![u32::id()])),
        Variant::new("EmptyTuple", 12, VariantKind::Tuple(vec![])),
        Variant::new("EmptyNamed", 13, VariantKind::Struct(vec![])),
    ]);
    inventory.register_type(TypeId::new(0xEEC0_0001), Type {
        name: name.to_string(),
        visibility: Visibility::Public,
        docs: Docs::default(),
        emission: Emission::FileEmission(FileEmission::Default),
        kind: TypeKind::Enum(Enum { variants, repr: Repr { layout: Layout::Primitive(base), alignment: None } }),
    });
    RustLibrary::builder(inventory.validate())
        .headers(HeaderConfig { emit_version: false })
        .build().process().unwrap().buffer("Interop.cs").unwrap().to_string()
}

#[test]
fn all_legal_enum_bases_group_constants_without_changing_the_native_tag_type() {
    for (base, cs) in [
        (Primitive::U8, "byte"), (Primitive::I8, "sbyte"),
        (Primitive::U16, "ushort"), (Primitive::I16, "short"),
        (Primitive::U32, "uint"), (Primitive::I32, "int"),
        (Primitive::U64, "ulong"), (Primitive::I64, "long"),
    ] {
        let code = generate(base, "Grouped", 2);
        assert!(code.contains(&format!("public enum Constants : {cs}")));
        assert!(code.contains(&format!("internal {cs} _variant;")));
        assert!(code.contains("First = 2,"));
        assert!(code.contains("Last = 9,"));
        assert!(!code.contains("FirstCase"));
        assert!(!code.contains("LastCase"));
        assert!(code.contains("record struct EmptyTupleCase()"));
        assert!(code.contains("record struct EmptyNamedCase()"));
    }
}

#[test]
fn pointer_sized_discriminants_keep_separate_unit_cases() {
    for base in [Primitive::Usize, Primitive::Isize] {
        let code = generate(base, "NativeWidth", 2);
        assert!(!code.contains("public enum Constants"));
        assert!(code.contains("record struct FirstCase()"));
        assert!(code.contains("record struct LastCase()"));
    }
}

#[test]
fn empty_payload_declarations_do_not_count_toward_the_group_threshold() {
    let code = generate(Primitive::U8, "SingleUnit", 1);
    assert!(!code.contains("public enum Constants"));
    for case in ["FirstCase", "EmptyTupleCase", "EmptyNamedCase"] {
        assert!(code.contains(&format!("record struct {case}()")));
    }
}

#[test]
fn the_group_name_cannot_equal_its_enclosing_type() {
    let code = generate(Primitive::U8, "Constants", 2);
    assert!(code.contains("public enum Constants2 : byte"));
    assert!(code.contains("public Constants(Constants2 value)"));
}
