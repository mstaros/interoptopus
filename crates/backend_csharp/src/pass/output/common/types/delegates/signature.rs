//! Renders bare function pointer delegates through the `signature.cs` template, grouped per output file.
//!
//! These are simple `[UnmanagedFunctionPointer(CallingConvention.Cdecl)]` delegate declarations
//! for Rust `extern "C" fn(...)` types, as opposed to the full wrapper classes produced by
//! the `class` pass for named callbacks.

use crate::lang::types::kind::{DelegateKind, TypeKind, TypePattern, Variant};
use crate::output::{FileType, Output};
use crate::pass::{OutputResult, PassInfo, model, output};
use interoptopus_backends::template::Context;
use std::collections::HashMap;


/// Runtime delegate marshalling does not use `LibraryImport`'s custom marshallers.
/// A composite containing projected tuple fields must use its native mirror here.
fn abi_name(id: crate::lang::TypeId, types: &model::common::types::all::Pass) -> Option<String> {
    fn contains_tuple(id: crate::lang::TypeId, types: &model::common::types::all::Pass) -> bool {
        if types.tuple_name(id).is_some() { return true; }
        match types.get(id).map(|ty| &ty.kind) {
            Some(TypeKind::Composite(c)) => c.fields.iter().any(|f| contains_tuple(f.ty, types)),
            Some(TypeKind::Array(a)) => contains_tuple(a.ty, types),
            Some(TypeKind::DataEnum(e) | TypeKind::TypePattern(TypePattern::Option(_, e) | TypePattern::Result(_, _, e))) =>
                e.variants.iter().flat_map(Variant::payloads).any(|p| contains_tuple(p.ty, types)),
            _ => false,
        }
    }
    let ty = types.get(id)?;
    if matches!(ty.kind, TypeKind::Composite(_) | TypeKind::DataEnum(_) | TypeKind::TypePattern(TypePattern::Option(_, _) | TypePattern::Result(_, _, _))) && contains_tuple(id, types) {
        Some(format!("{}.Unmanaged", ty.name))
    } else {
        Some(ty.name.clone())
    }
}

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    delegates: HashMap<Output, Vec<String>>,
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, delegates: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        types: &model::common::types::all::Pass,
    ) -> OutputResult {
        let templates = output_master.templates();

        for file in output_master.outputs_of(FileType::Csharp) {
            let mut rendered = Vec::new();

            for (type_id, ty) in types.iter() {
                if !output_master.type_belongs_to(*type_id, file) {
                    continue;
                }

                let delegate = match &ty.kind {
                    TypeKind::Delegate(d) if d.kind == DelegateKind::Signature => d,
                    _ => continue,
                };
                let signature = &delegate.signature;
                let name = &ty.name;

                let rval_managed = abi_name(signature.rval, types).unwrap_or_else(|| "void".to_string());

                let mut args: Vec<HashMap<String, String>> = Vec::new();
                for arg in &signature.arguments {
                    let Some(arg_managed) = abi_name(arg.ty, types) else {
                        continue;
                    };
                    let mut m = HashMap::new();
                    m.insert("name".to_string(), arg.name.clone());
                    m.insert("managed_type".to_string(), arg_managed.clone());
                    args.push(m);
                }

                let mut context = Context::new();
                context.insert("name", name);
                context.insert("visibility", &ty.visibility.to_string());
                context.insert("rval_managed", &rval_managed);
                context.insert("args", &args);

                let r = templates.render("common/types/delegate/signature.cs", &context)?;
                rendered.push(r);
            }

            rendered.sort();
            self.delegates.insert(file.clone(), rendered);
        }

        Ok(())
    }

    #[must_use]
    pub fn delegates_for(&self, output: &Output) -> Option<&[String]> {
        self.delegates.get(output).map(std::vec::Vec::as_slice)
    }
}
