use interoptopus::lang::types::{TypeInfo, TypeKind};
use interoptopus::{extra_type, ffi};
use interoptopus_csharp::RustLibrary;
use reference_project::patterns::tuples::{TupleBorrowed, TupleId, TupleOwned, TuplePair};

#[ffi]
pub struct NamedPositions {
    pub field_0: u32,
    pub field_1: f32,
}

#[test]
fn inventory_distinguishes_positional_fields_from_similarly_named_fields() {
    let TypeKind::Struct(positional) = TuplePair::kind() else { panic!("expected struct") };
    let TypeKind::Struct(named) = NamedPositions::kind() else { panic!("expected struct") };
    assert!(positional.is_positional);
    assert!(!named.is_positional);
    assert_eq!(positional.fields.iter().map(|f| &f.name).collect::<Vec<_>>(),
        named.fields.iter().map(|f| &f.name).collect::<Vec<_>>());
}

#[test]
fn tuple_projection_is_based_on_shape_and_keeps_resource_identities() -> Result<(), Box<dyn std::error::Error>> {
    let mut inventory = reference_project::inventory();
    let _ = inventory.register(extra_type!(TuplePair));
    let _ = inventory.register(extra_type!(NamedPositions));
    let _ = inventory.register(extra_type!(TupleId));
    let _ = inventory.register(extra_type!(TupleOwned));
    let _ = inventory.register(extra_type!(TupleBorrowed));
    let output = RustLibrary::builder(inventory).build().process()?.to_string();
    assert!(output.contains("implicit operator (uint, float)(TuplePair value)"));
    assert!(!output.contains("implicit operator (uint, float)(NamedPositions"));
    assert!(output.contains("public NamedPositions(uint field_0, float field_1)"));
    assert!(output.contains("Deconstruct(out uint value1, out float value2)"));
    for name in ["TupleId", "TupleOwned", "TupleBorrowed"] {
        assert!(!output.contains(&format!("implicit operator {name}(")));
    }
    Ok(())
}
