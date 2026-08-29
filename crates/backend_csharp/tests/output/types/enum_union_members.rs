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
//!
//! **Since step three that population is not a struct at all.** A `DataEnum` with no
//! payload-capable variant *and* a discriminant C# accepts as an enum base is emitted as a plain
//! C# `enum`, which cannot carry a flag or union members by construction. The assertions below
//! therefore check the stronger fact rather than the absence of members on a struct.

use interoptopus::inventory::TypeId;
use interoptopus::lang::meta::{Docs, Emission, FileEmission, Visibility};
use interoptopus::lang::types::{Enum, Layout, Primitive, Repr, Type, TypeKind, Variant, VariantKind};
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

/// Unit-only: no variant can carry a payload, so it is emitted as a plain C# `enum`.
#[ffi]
pub enum Flag {
    Off,
    On,
}

/// Models the metadata a hand-written `TypeInfo` implementation may register.
///
/// `#[ffi]` intentionally normalises enum reprs to fixed widths, so a test-only inventory type
/// is the narrowest way to exercise the pointer-sized discriminant route without adding a native
/// reference-project API.
fn pointer_sized_flag_type() -> (TypeId, Type) {
    let id = TypeId::new(0xD15C_21A1_0000_0000_0000_0000_0000_0001);
    let kind = TypeKind::Enum(Enum {
        variants: vec![
            Variant::new("NativeWidthZero", 0, VariantKind::Unit),
            Variant::new("NativeWidthOne", 1, VariantKind::Unit),
        ],
        repr: Repr { layout: Layout::Primitive(Primitive::Isize), alignment: None },
    });
    let ty = Type {
        name: "PointerSizedFlag".to_string(),
        visibility: Visibility::Public,
        docs: Docs::default(),
        emission: Emission::FileEmission(FileEmission::Default),
        kind,
    };
    (id, ty)
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
    let (id, ty) = pointer_sized_flag_type();
    inventory.register_type(id, ty);
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
/// fixtures must reach the output in the form the assertions assume, or they pass for the wrong
/// reason.
///
/// `Flag`'s expected form changed with step three: it was struct-backed, and is now a plain C#
/// `enum`. That is the assertion, not an incidental detail — if it regressed to a struct the
/// plain-enum test below would still pass vacuously on the absence of union members.
#[test]
fn the_fixtures_have_the_backing_the_assertions_assume() {
    let cs = generated_interop();
    assert!(cs.contains("struct Meter"), "Meter must stay struct-backed");
    assert!(cs.contains("enum Flag : "), "Flag must be emitted as a plain C# enum");
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
fn a_well_formed_value_gets_the_flag_however_it_is_constructed() {
    let cs = generated_interop();

    assert!(
        cs.contains("public static Meter Count(uint value) => new() { _variant = 1, _Count = value, _hasValue = true };"),
        "a factory produces a well-formed value, so it must set the flag"
    );
    assert!(!cs.contains("_managed._hasValue = true;"), "item 4a deleted 3c's explicit write; if it is back, ToManaged has regressed to mutating a default instance"); assert!(cs.contains("new Meter(new CountCase("), "a value arriving from native is well-formed too, and ToManaged now establishes that by building through the case constructor, which sets the flag itself"); assert!(cs.contains("_ => throw new InteropException("), "and an unrecognised native tag must throw rather than yield a value carrying that tag with no payload");
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
///
/// Step three raised the stakes: a type misclassified as unit-only is no longer merely stripped
/// of its members, it is emitted as a plain C# `enum` and loses its payload entirely.
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

/// A unit-only enum is emitted as a plain C# `enum`, not a struct.
///
/// Both halves of the rule are required. No variant can carry a payload — which is what makes it
/// ineligible for union projection — *and* its discriminant is a type C# accepts as an enum base.
/// A unit-only enum failing the second half keeps the struct, which is the one exclusion that
/// stops `Projection` collapsing to a boolean.
///
/// This supersedes the older assertion that such a type merely lacked `_hasValue` and union
/// members. It cannot have either now: a C# `enum` has no fields and no methods. The absence
/// checks are kept because they would still catch a regression to the struct form emitting
/// machinery nothing reads — six of the nineteen CS0169 in `Issues.md` `8f4c1e2a` were exactly
/// that.
#[test]
fn a_unit_only_enum_becomes_a_plain_csharp_enum() {
    let cs = generated_interop();

    assert!(cs.contains("enum Flag : "), "a unit-only enum with an integral discriminant is emitted as a plain C# enum");
    assert!(!cs.contains("struct Flag"), "and therefore not as a struct carrying a discriminant field");

    let flag = cs.split("enum Flag : ").nth(1).expect("Flag is emitted");
    let flag = flag.split('}').next().expect("Flag body is bounded");

    assert!(flag.contains("Off = 0,"), "its variants become enum members carrying their discriminants: {flag}");
    assert!(flag.contains("On = 1,"), "its variants become enum members carrying their discriminants: {flag}");
    assert!(!flag.contains("_hasValue"), "a C# enum has no empty-state flag to declare: {flag}");
    assert!(!cs.contains("OffCase"), "unit-only enum must receive no union machinery");
}


/// A unit-only enum with a pointer-sized discriminant cannot become a plain C# enum.
///
/// `nint` is a valid managed representation of Rust `isize`, but C# does not permit it as an
/// enum base. That is the third `Projection` value: no payload warrants union machinery, yet the
/// discriminant prevents the plain-enum projection, so the legacy struct, mirror and marshaller
/// must remain.
#[test]
fn a_unit_only_pointer_sized_enum_keeps_the_discriminant_struct_projection() {
    let cs = generated_interop();

    assert!(
        !cs.contains("enum PointerSizedFlag :"),
        "C# forbids nint as an enum base, so this type must not take the plain-enum route"
    );
    assert!(cs.contains("partial struct PointerSizedFlag"), "the discriminant projection must keep the managed struct");
    assert!(
        cs.contains("internal nint _variant;"),
        "the unmanaged mirror must preserve the pointer-sized discriminant instead of collapsing it to int"
    );
    assert!(
        cs.contains("[CustomMarshaller(typeof(PointerSizedFlag), MarshalMode.Default, typeof(Marshaller))]"),
        "the discriminant projection keeps its marshaller"
    );
    assert!(
        !cs.contains("partial struct PointerSizedFlag : IUnion"),
        "a unit-only enum has no payload and must not receive union machinery"
    );
    assert!(
        !cs.contains("public readonly record struct NativeWidthZeroCase()"),
        "the discriminant projection must not emit case types"
    );
}

/// Item 4c: a default struct-backed union must not read as its variant-zero case.
///
/// `Ok` is tag 0 and `default` is all-zero, so with tag preservation the discriminant cannot
/// distinguish "empty" from "variant zero". 3c made the C# 15 surface handle that — `HasValue`
/// reads `_hasValue` and `Value` returns null when it is false — but the older accessor surface
/// did not, so one value simultaneously reported `HasValue == false` and `IsOk == true`, and
/// `AsOk()` returned a **fabricated zero** out of uninitialised memory rather than throwing. A
/// wrong value reaching a consumer, not merely a wrong flag.
///
/// Scoped by `writes_has_value`, so class-backed unions are untouched: they have no `_hasValue`
/// and their `default` is a null reference, which cannot reach these members at all.
///
/// `ExceptionForVariant()` is deliberately retained, and item 4b now makes it return
/// `InvalidOperationException` for the empty state before consulting `_variant`.
///
/// **This test is also why the change is testable.** The template-only version (`9e3383f`,
/// reverted in `a4e39ad`) caused Guarded to select no tests and pass a gate it never executed: a
/// `.cs` template is an input to the generator, but the impact analyser does not model it as one.
#[test]
fn a_default_struct_backed_union_does_not_read_as_variant_zero() {
    let cs = generated_interop();

    assert!(
        cs.contains("public bool IsNothing => _hasValue && _variant == 0;"),
        "a struct-backed check must consult the empty-state flag, or `default(Meter).IsNothing` is \
         true while `default(Meter).HasValue` is false"
    );
    assert!(
        cs.contains("public uint AsCount() { if (!_hasValue || _variant != 1)"),
        "a struct-backed accessor must reject the empty state before returning a payload read out \
         of uninitialised memory"
    );

    assert!(
        cs.contains(
            "if (!_hasValue) return new InvalidOperationException(\"A default Meter is empty and corresponds to no Rust variant."
        ),
        "item 4b must classify the empty state before variant zero can become EnumException"
    );

    assert!(cs.contains("public bool IsBlank => _variant == 0;"), "class-backed unions keep the plain check: there is no _hasValue to read and default is a null reference, so the empty state is unreachable");
}
