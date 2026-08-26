//! Item 3c: `HasValue`, `Value` and `TryGetValue`, and the `_hasValue` writes they read.
//!
//! Reads and writes could not be split. `_hasValue` was emitted by `definition.cs` and touched
//! nowhere else in the generator — CS0169, "never used", nineteen times in full-scale output
//! (`Issues.md` `8f4c1e2a`). Adding writes alone would have turned that into CS0414, "assigned
//! but never used"; only both together clear it.
//!
//! The struct/class split is the substance, not a detail:
//!
//! - **Struct-backed.** `default(E)` is a legal C# value corresponding to no Rust variant, so
//!   `HasValue` and `Value` must consult `_hasValue`. Item 4c makes this a soundness obligation:
//!   the compiler assumes *"for struct unions, `default` produces a `Value` of null"* and reasons
//!   about exhaustiveness on it.
//! - **Class-backed.** There is no empty class instance — `default(E)` is a null reference and
//!   every non-null instance is valid — so `HasValue` is constant `true`, `Value` is never null,
//!   and there is no field to consult.
//!
//! Scoping is the same per-enum eligibility rule as 3b, and it now also governs the *field*: a
//! unit-only enum has no union machinery, so it needs no empty-state flag. Six of the nineteen
//! CS0169 were exactly that case.

use interoptopus::lang::meta::FileEmission;
use interoptopus::wire::Wire;
use interoptopus::{extra_type, ffi, function};
use interoptopus_csharp::config::HeaderConfig;
use interoptopus_csharp::dispatch::Dispatch;
use interoptopus_csharp::output::Target;

/// Struct-backed: blittable payload, so `struct_class` keeps it a struct.
#[ffi]
pub enum Meter {
    Nothing,
    Count(u32),
}

/// Unit-only: receives no union machinery at all, and therefore no `_hasValue`.
#[ffi]
pub enum Flag {
    Off,
    On,
}

/// Class-backed: carries a `String`, so its managed conversion is `Into`.
#[ffi]
pub enum Label {
    Blank,
    Text(String),
}

#[ffi]
pub struct LabelHolder {
    pub label: Option<Label>,
    pub note: String,
}

/// `Result<(), ()>` — declares payload-carrying `Ok(T)`/`Err(E)` positions whose type arguments
/// are both `()`. The only shape where "can carry a payload" and "does carry a payload" disagree.
#[ffi(export = unique)]
pub fn union_members_void_result(x: ffi::Result<(), ()>) -> ffi::Result<(), ()> {
    x
}

#[ffi(export = unique)]
pub fn union_members_wire(mut holder: Wire<LabelHolder>) -> bool {
    holder.unwire().note.is_empty()
}

fn generated_interop() -> String {
    let mut inventory = interoptopus::inventory::RustInventory::new();
    let _ = inventory.register(function!(union_members_wire));
    let _ = inventory.register(function!(union_members_void_result));
    let _ = inventory.register(extra_type!(Meter));
    let _ = inventory.register(extra_type!(Flag));
    let inventory = inventory.validate();

    let multibuf = interoptopus_csharp::RustLibrary::builder(inventory)
        .dispatch(Dispatch::custom(|x, _| match x.emission {
            FileEmission::Common => Target::new("Interop.Common.cs", "My.Company.Common"),
            FileEmission::Default | FileEmission::CustomModule(_) => Target::new("Interop.cs", "My.Company"),
        }))
        .headers(HeaderConfig { emit_version: false })
        .build()
        .process()
        .unwrap();

    multibuf.buffer("Interop.cs").unwrap().to_string()
}

/// `Result<(), ()>` is a *common* type, so it is dispatched to `Interop.Common.cs` rather than
/// `Interop.cs`. Reading the wrong buffer is why the premise guard exists.
fn generated_common() -> String {
    let mut inventory = interoptopus::inventory::RustInventory::new();
    let _ = inventory.register(function!(union_members_void_result));
    let inventory = inventory.validate();

    let multibuf = interoptopus_csharp::RustLibrary::builder(inventory)
        .dispatch(Dispatch::custom(|x, _| match x.emission {
            FileEmission::Common => Target::new("Interop.Common.cs", "My.Company.Common"),
            FileEmission::Default | FileEmission::CustomModule(_) => Target::new("Interop.cs", "My.Company"),
        }))
        .headers(HeaderConfig { emit_version: false })
        .build()
        .process()
        .unwrap();

    multibuf.buffer("Interop.Common.cs").unwrap().to_string()
}

/// Guards the premise. Every assertion below is about *which* form a type gets, so all three
/// fixtures must reach the output and keep their struct/class backing, or the assertions pass
/// for the wrong reason.
#[test]
fn the_fixtures_have_the_backing_the_assertions_assume() {
    let cs = generated_interop();
    assert!(cs.contains("struct Meter"), "Meter must stay struct-backed");
    assert!(cs.contains("struct Flag"), "Flag must stay struct-backed");
    assert!(cs.contains("class Label"), "Label must stay class-backed");
}

#[test]
fn a_struct_backed_union_consults_has_value() {
    let cs = generated_interop();

    assert!(cs.contains("public bool HasValue => _hasValue;"), "struct-backed HasValue must read the flag");
    assert!(
        cs.contains("public object? Value => !_hasValue ? null : _variant switch"),
        "struct-backed Value must return null for default(E); item 4c makes this a soundness \
         obligation, not a preference"
    );
    assert!(
        cs.contains("if (_hasValue && _variant == 1) { value = new CountCase(_Count); return true; }"),
        "struct-backed TryGetValue must consult the flag before materialising a case"
    );
}

#[test]
fn a_class_backed_union_has_no_flag_to_consult() {
    let cs = generated_interop();

    assert!(cs.contains("public bool HasValue => true;"), "class-backed HasValue is a constant: every non-null instance is valid");

    let label = cs.split("class Label").nth(1).expect("Label is emitted");
    let label = label.split("public partial").next().expect("Label body is bounded");
    assert!(!label.contains("_hasValue"), "class-backed enums have no `_hasValue` field to read: {label}");
}

#[test]
fn the_flag_is_written_where_a_well_formed_value_is_constructed() {
    let cs = generated_interop();

    assert!(
        cs.contains("public static Meter Count(uint value) => new() { _variant = 1, _Count = value, _hasValue = true };"),
        "a factory produces a well-formed value, so it must set the flag"
    );
    assert!(
        cs.contains("_managed._hasValue = true;"),
        "a value arriving from native is well-formed too; without this write ToManaged would \
         produce values whose Value reports null"
    );
}

/// Eligibility asks whether a variant **can** carry a payload, not whether one **does**.
///
/// `Result<(), ()>` declares `Ok(T)` and `Err(E)` — payload-carrying positions — but
/// `fallback.rs::resolve_payload` maps a `()` payload to `None`, so both read as payloadless
/// downstream and become indistinguishable from a true unit variant. A check written as
/// `v.ty.is_some()` therefore classified `ResultVoidVoid` as unit-only and stripped its union
/// projection, while `Result<u32, Error>` kept it — the same shape decided by a type argument.
///
/// The predicate now reads `v.can_carry_payload`, set from `VariantKind::Tuple` in
/// `enum_variants.rs` and from the payload-carrying position in `fallback.rs`.
///
/// **This is the only fixture that separates the two predicates.** Every other type is classified
/// identically either way, so reverting to `ty.is_some()` would pass the rest of the suite.
#[test]
fn eligibility_asks_can_carry_not_does_carry() {
    let cs = generated_common();

    assert!(cs.contains("struct ResultVoidVoid"), "ResultVoidVoid must reach the output or this proves nothing");

    let decl = cs.split("struct ResultVoidVoid").nth(1).expect("ResultVoidVoid is emitted");
    let decl = decl.split("[NativeMarshalling").next().expect("its plain declaration is bounded");

    assert!(
        decl.contains("bool _hasValue;"),
        "Result<(), ()> declares payload-carrying variants, so it is union-projected and needs the \
         empty-state flag; without it `default(ResultVoidVoid)` cannot be told from `Ok`, which \
         item 4c forbids: {decl}"
    );

    assert!(
        cs.contains("public readonly record struct OkCase();"),
        "Result<(), ()> must still get case types; a `()` type argument must not decide whether a \
         type is a union"
    );
}

/// The eligibility rule now governs the field, not only the members. This is what cleared six
/// of the nineteen CS0169: nothing would ever have read their flag.
#[test]
fn a_unit_only_enum_gets_neither_the_flag_nor_the_members() {
    let cs = generated_interop();

    let flag = cs.split("struct Flag").nth(1).expect("Flag is emitted");
    let flag = flag.split("public partial").next().expect("Flag body is bounded");

    assert!(!flag.contains("_hasValue"), "unit-only enum must not declare an empty-state flag: {flag}");
    assert!(!cs.contains("OffCase"), "unit-only enum must receive no union machinery");
}