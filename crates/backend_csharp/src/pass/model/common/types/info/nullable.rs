//! Decides what a conversion site must do when a field's value can be `null`.
//!
//! Three answers, not two. The obvious shape is a boolean — "is this a reference type?" — and
//! that is what this pass used to answer, with the consumers mapping `true` onto
//! `?{suffix} ?? default`. That works for a class delegate, where substituting a zeroed
//! `Unmanaged` is harmless, and it is actively wrong for a class-backed union, where a zeroed
//! `Unmanaged` means discriminant 0: a **fabricated variant crossing FFI**, read by Rust as a
//! real one. That is precisely what item 3a's private parameterless constructor exists to
//! prevent, and it is worse than the exception it would replace.
//!
//! See `docs/csharp-unions.md` Open items 1 for the decision and the two rejected alternatives.

use crate::lang::TypeId;
use crate::lang::types::kind::{DelegateKind, TypeKind, TypePattern};
use crate::pass::Outcome::Unchanged;
use crate::pass::{ModelResult, PassInfo, model};
use std::collections::HashMap;

/// What a conversion site must emit for a field of this type.
///
/// Named for the *decision* rather than the type, because the consumers act on the policy and
/// should not have to re-derive it from a type quiz.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NullPolicy {
    /// A value type. `null` is not representable, so the conversion is unguarded.
    NotNullable,

    /// A reference type whose zeroed `Unmanaged` is a legitimate value. Emits
    /// `?{suffix} ?? default`.
    SubstituteDefault,

    /// A reference type whose zeroed `Unmanaged` would be a **fabricated variant**. Emits
    /// `?{suffix} ?? throw new InvalidOperationException(..)`.
    ///
    /// Class-backed unions only. A struct-backed union cannot be null — its empty state is
    /// `_hasValue == false`, guarded separately by item 4 at `ToUnmanaged`/`AsUnmanaged`.
    Throw,
}

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    policy: HashMap<TypeId, NullPolicy>,
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, policy: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        types: &model::common::types::all::Pass,
        struct_class: &model::common::types::info::struct_class::Pass,
        projection: &model::common::types::info::projection::Pass,
    ) -> ModelResult {
        let mut outcome = Unchanged;

        for (type_id, ty) in types.iter() {
            if self.policy.contains_key(type_id) {
                continue;
            }

            // Not-ready guard, and it is load-bearing rather than defensive.
            //
            // This pass is write-once — the `contains_key` above never revisits — and the two
            // pipelines disagree about when it runs: `rust/library.rs` runs it *before*
            // `struct_class` and `projection`, `dotnet/library.rs` *after*. Without this, the
            // rust pipeline would classify every union on round one, when `projection` has no
            // answer yet, and cache it forever.
            //
            // `struct_class` cannot serve as the signal: `is_class` is `!is_struct` and
            // `is_struct` is `unwrap_or(false)`, so an unanswered type reports `is_class == true`
            // — a wrong answer rather than a not-ready one. `projection` returns `None` for
            // anything that is not an enum, so the guard is keyed on the kind first.
            let is_enum_kind = matches!(
                &ty.kind,
                TypeKind::DataEnum(_) | TypeKind::TypePattern(TypePattern::Result(_, _, _) | TypePattern::Option(_, _))
            );

            if is_enum_kind && projection.projection(*type_id).is_none() {
                continue;
            }

            let policy = if is_enum_kind && projection.is_union(*type_id) && struct_class.is_class(*type_id) {
                NullPolicy::Throw
            } else if matches!(&ty.kind, TypeKind::Delegate(d) if d.kind == DelegateKind::Class) {
                NullPolicy::SubstituteDefault
            } else {
                // Deliberately narrow. Class-backed *composites* are reference types too and are
                // mis-classified here today, which is `Issues.md` `b4e07f12`. That ticket also
                // questions whether `?? default` is right even for the types it already covers,
                // so widening is its call and not this one's.
                NullPolicy::NotNullable
            };

            self.policy.insert(*type_id, policy);
            outcome.changed();
        }

        Ok(outcome)
    }

    #[must_use]
    pub fn null_policy(&self, ty: TypeId) -> Option<NullPolicy> {
        self.policy.get(&ty).copied()
    }
}
