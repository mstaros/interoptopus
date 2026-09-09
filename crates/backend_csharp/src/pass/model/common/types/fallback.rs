//! Computes the C-level fallback `TypeKind` for each pattern type.
//!
//! For each Rust `TypePattern`, this stores the equivalent C-level `TypeKind`
//! (the "unrolled" representation). Struct-based patterns like Slice become
//! Composite with ptr/len fields; enum-based patterns like Option/Result
//! become `DataEnum` with their variants.
//!
//! All inner type references are resolved through the `id_map` from Rust `TypeIds`
//! to C# `TypeIds`, relying on the convergence loop to retry when dependencies
//! aren't mapped yet.

use crate::lang::TypeId;
use crate::lang::meta::Visibility;
use crate::lang::types::kind::{Composite, DataEnum, Field, IntPtrHint, Pointer, PointerKind, Primitive, TypeKind, Variant};
use crate::pass::Outcome::Unchanged;
use crate::pass::{ModelResult, PassInfo, model};
use crate::try_extract_kind;
use interoptopus::lang;
use interoptopus::lang::meta::Docs;
use interoptopus::lang::types::{Repr, TypeInfo, type_id_ptr, type_id_ptr_mut};
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    fallbacks: HashMap<TypeId, TypeKind>,
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, fallbacks: HashMap::default() }
    }

    pub fn process(&mut self, _pass_meta: &mut crate::pass::PassMeta, id_map: &model::common::id_map::Pass, rs_types: &interoptopus::inventory::Types) -> ModelResult {
        let mut outcome = Unchanged;

        // Static Rust TypeIds for commonly needed types and pointers.

        for (rust_id, ty) in rs_types {
            let rust_pattern = try_extract_kind!(ty, TypePattern);
            let Some(cs_id) = id_map.ty(*rust_id) else { continue };

            if self.fallbacks.contains_key(&cs_id) {
                continue;
            }

            let fallback = match rust_pattern {
                lang::types::TypePattern::CStrPointer => {
                    // *const c_char
                    let Some(cs_ptr) = id_map.ty(<*const std::ffi::c_char>::id()) else { continue };
                    TypeKind::Pointer(Pointer { kind: PointerKind::IntPtr(IntPtrHint::Read), target: cs_ptr })
                }
                lang::types::TypePattern::Utf8String => {
                    // { *mut u8, u64, u64 }
                    let Some(cs_ptr) = id_map.ty(<*mut u8>::id()) else { continue };
                    let Some(cs_u64) = id_map.ty(u64::id()) else { continue };
                    TypeKind::Composite(Composite { fields: vec![field("ptr", cs_ptr), field("len", cs_u64), field("capacity", cs_u64)], repr: Repr::c() })
                }
                lang::types::TypePattern::Version => TypeKind::Primitive(Primitive::ULong),
                lang::types::TypePattern::Slice(rust_ty) => {
                    // { *const T, u64 }
                    let Some(cs_ptr) = id_map.ty(type_id_ptr(*rust_ty)) else { continue };
                    let Some(cs_u64) = id_map.ty(u64::id()) else { continue };
                    TypeKind::Composite(Composite { fields: vec![field("ptr", cs_ptr), field("len", cs_u64)], repr: Repr::c() })
                }
                lang::types::TypePattern::SliceMut(rust_ty) => {
                    // { *mut T, u64 }
                    let Some(cs_ptr) = id_map.ty(type_id_ptr_mut(*rust_ty)) else { continue };
                    let Some(cs_u64) = id_map.ty(u64::id()) else { continue };
                    TypeKind::Composite(Composite { fields: vec![field("ptr", cs_ptr), field("len", cs_u64)], repr: Repr::c() })
                }
                lang::types::TypePattern::Vec(rust_ty) => {
                    // { *mut T, u64, u64 }
                    let Some(cs_ptr) = id_map.ty(type_id_ptr_mut(*rust_ty)) else { continue };
                    let Some(cs_u64) = id_map.ty(u64::id()) else { continue };
                    TypeKind::Composite(Composite { fields: vec![field("ptr", cs_ptr), field("len", cs_u64), field("capacity", cs_u64)], repr: Repr::c() })
                }
                lang::types::TypePattern::Option(rust_ty) => {
                    let Some(payload) = resolve_payload(*rust_ty, id_map) else { continue };
                    TypeKind::DataEnum(DataEnum { variants: vec![payload_variant("Some", 0, payload), unit_variant("None", 1)], discriminant_type: Primitive::UInt })
                }
                lang::types::TypePattern::Result(rust_ok, rust_err) => {
                    let Some(ok_payload) = resolve_payload(*rust_ok, id_map) else { continue };
                    let Some(err_payload) = resolve_payload(*rust_err, id_map) else { continue };
                    TypeKind::DataEnum(DataEnum {
                        variants: vec![
                            payload_variant("Ok", 0, ok_payload),
                            payload_variant("Err", 1, err_payload),
                            unit_variant("Panic", 2),
                            unit_variant("Null", 3),
                        ],
                        discriminant_type: Primitive::UInt,
                    })
                }
                lang::types::TypePattern::Bool => TypeKind::Primitive(Primitive::Byte),
                lang::types::TypePattern::CChar => TypeKind::Primitive(Primitive::SByte),
                lang::types::TypePattern::CVoid => TypeKind::Primitive(Primitive::Void),
                lang::types::TypePattern::Wire(_) => {
                    // { *mut u8, i32, i32 }
                    let Some(cs_ptr) = id_map.ty(<*mut u8>::id()) else { continue };
                    let Some(cs_i32) = id_map.ty(i32::id()) else { continue };
                    TypeKind::Composite(Composite { fields: vec![field("data", cs_ptr), field("len", cs_i32), field("capacity", cs_i32)], repr: Repr::c() })
                }
                lang::types::TypePattern::NamedCallback(_) | lang::types::TypePattern::AsyncCallback(_) => {
                    // { *mut c_void, *mut c_void }
                    let Some(cs_void_ptr) = id_map.ty(<*mut std::ffi::c_void>::id()) else { continue };
                    TypeKind::Composite(Composite { fields: vec![field("fnptr", cs_void_ptr), field("data", cs_void_ptr)], repr: Repr::c() })
                }
                lang::types::TypePattern::AsyncIterator(_) => {
                    let Some(cs_ptr) = id_map.ty(<*mut std::ffi::c_void>::id()) else { continue };
                    TypeKind::Composite(Composite {
                        fields: ["data", "next_fn", "drop_fn"].into_iter().map(|name| field(name, cs_ptr)).collect(),
                        repr: Repr::c(),
                    })
                }
                lang::types::TypePattern::Iterator(_) => {
                    let Some(cs_ptr) = id_map.ty(<*mut std::ffi::c_void>::id()) else { continue };
                    TypeKind::Composite(Composite {
                        fields: ["data", "where_fn", "take_fn", "any_fn", "drop_fn", "next_fn"].into_iter().map(|name| field(name, cs_ptr)).collect(),
                        repr: Repr::c(),
                    })
                }
                lang::types::TypePattern::TaskHandle => {
                    // { *mut c_void, *mut c_void, *mut c_void }
                    let Some(cs_void_ptr) = id_map.ty(<*mut std::ffi::c_void>::id()) else { continue };
                    TypeKind::Composite(Composite {
                        fields: vec![field("data", cs_void_ptr), field("abort_fn", cs_void_ptr), field("drop_fn", cs_void_ptr)],
                        repr: Repr::c(),
                    })
                }
            };

            self.fallbacks.insert(cs_id, fallback);
            outcome.changed();
        }

        Ok(outcome)
    }

    #[must_use]
    pub fn get(&self, id: TypeId) -> Option<&TypeKind> {
        self.fallbacks.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&TypeId, &TypeKind)> {
        self.fallbacks.iter()
    }
}

fn field(name: &str, ty: TypeId) -> Field {
    Field { name: name.to_string(), docs: Docs::default(), visibility: Visibility::Public, ty }
}

/// A synthesised variant whose declaration has a payload slot.
///
/// `can_carry_payload` is `true` even when `ty` is `None`, because `resolve_payload` maps a `()`
/// payload to `None`. `Result<(), ()>` still declares `Ok(T)`/`Err(E)`; erasing that would make
/// it indistinguishable from a unit-only enum and strip its union projection.
fn payload_variant(name: &str, tag: isize, ty: Option<TypeId>) -> Variant {
    Variant {
        name: name.to_string(), docs: Docs::default(), tag,
        fields: ty.map(|ty| crate::lang::types::kind::Field {
            name: String::new(), docs: Docs::default(), visibility: crate::lang::meta::Visibility::Public, ty,
        }).into_iter().collect(),
        field_names: Vec::new(), case_fields: Vec::new(), can_carry_payload: true,
        stem: String::new(), case_type: String::new(),
    }
}

/// A synthesised variant with no payload slot at all — `None`, `Panic`, `Null`.
fn unit_variant(name: &str, tag: isize) -> Variant {
    Variant { name: name.to_string(), docs: Docs::default(), tag, fields: Vec::new(), field_names: Vec::new(), case_fields: Vec::new(), can_carry_payload: false, stem: String::new(), case_type: String::new() }
}

/// Resolves a Rust type to an optional C# variant payload.
/// Void types (`()`) become `Some(None)` (no payload), non-void types become
/// `Some(Some(cs_id))`, and not-yet-mapped types return `None`.
#[allow(clippy::option_option)]
fn resolve_payload(rust_ty: interoptopus::inventory::TypeId, id_map: &model::common::id_map::Pass) -> Option<Option<TypeId>> {
    if rust_ty == <()>::id() { Some(None) } else { id_map.ty(rust_ty).map(Some) }
}
