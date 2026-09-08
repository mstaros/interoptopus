//! Renders class-based delegate type definitions through the `all.cs` template, grouped per output file.
//!
//! These are full wrapper classes for named callbacks (with data pointers), including
//! `Unmanaged` structs, marshallers, and trampoline methods.

use crate::lang::TypeId;
use crate::lang::types::kind::{DelegateKind, PointerKind, Primitive, TypeKind, TypePattern, Variant};
use crate::output::{FileType, Output};
use crate::pass::{OutputResult, PassInfo, model, output};
use interoptopus_backends::template::Context;
use std::collections::HashMap;

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
        unmanaged_names: &output::common::conversion::unmanaged_names::Pass,
        unmanaged_conversion: &output::common::conversion::unmanaged_conversion::Pass,
    ) -> OutputResult {
        let templates = output_master.templates();

        for file in output_master.outputs_of(FileType::Csharp) {
            let mut rendered_delegates = Vec::new();

            for (type_id, ty) in types.iter() {
                if !output_master.type_belongs_to(*type_id, file) {
                    continue;
                }

                let delegate = match &ty.kind {
                    TypeKind::Delegate(d) if d.kind == DelegateKind::Class => d,
                    _ => continue,
                };
                let signature = &delegate.signature;
                let name = &ty.name;

                // Determine return type info
                let rval_kind = types.get(signature.rval).map(|t| &t.kind);
                let rval_managed = types.get(signature.rval).map_or_else(|| "void".to_string(), |t| t.name.clone());
                let is_void = matches!(rval_kind, Some(TypeKind::Primitive(Primitive::Void)));

                let rval_unmanaged = if is_void {
                    "void".to_string()
                } else {
                    unmanaged_names.name(signature.rval).cloned().unwrap_or_else(|| rval_managed.clone())
                };

                let rval_to_unmanaged = if is_void {
                    String::new()
                } else {
                    unmanaged_conversion.to_unmanaged_suffix(signature.rval).to_string()
                };

                let rval_to_managed = if is_void {
                    String::new()
                } else {
                    unmanaged_conversion.to_managed_suffix(signature.rval).to_string()
                };

                // Build argument list (excluding callback_data which is always appended in the template)
                let mut args: Vec<HashMap<String, String>> = Vec::new();
                for arg in &signature.arguments {
                    let Some(arg_managed) = types.get(arg.ty).map(|t| &t.name) else {
                        continue;
                    };

                    let arg_unmanaged = unmanaged_names.name(arg.ty).cloned().unwrap_or_else(|| arg_managed.clone());
                    let to_managed = unmanaged_conversion.to_managed_suffix(arg.ty).to_string();
                    let to_unmanaged = unmanaged_conversion.to_unmanaged_suffix(arg.ty).to_string();

                    let mut m = HashMap::new();
                    m.insert("name".to_string(), arg.name.clone());
                    m.insert("managed_type".to_string(), arg_managed.clone());
                    m.insert("unmanaged_name".to_string(), arg_unmanaged);
                    m.insert("to_managed".to_string(), to_managed);
                    m.insert("to_unmanaged".to_string(), to_unmanaged);
                    args.push(m);
                }

                let mut context = Context::new();
                context.insert("name", name);
                let standard_delegate = delegate.standard_name(|id| types.get(id));
                let managed_delegate = standard_delegate.clone().unwrap_or_else(|| format!("{name}Delegate"));
                context.insert("managed_delegate", &managed_delegate);
                context.insert("custom_delegate", &standard_delegate.is_none());
                context.insert("is_void", &is_void);
                context.insert("visibility", &ty.visibility.to_string());
                context.insert("rval_managed", &rval_managed);
                context.insert("rval_unmanaged_name", &rval_unmanaged);
                context.insert("rval_to_unmanaged", &rval_to_unmanaged);
                context.insert("rval_to_managed", &rval_to_managed);
                context.insert("args", &args);
                let function_pointer = supports_function_pointer(signature.rval, types)
                    && signature.arguments.iter().all(|arg| supports_function_pointer(arg.ty, types));
                context.insert("function_pointer", &function_pointer);

                let rendered = templates.render("common/types/delegate/class.cs", &context)?;
                rendered_delegates.push(rendered);
            }

            rendered_delegates.sort();
            self.delegates.insert(file.clone(), rendered_delegates);
        }

        Ok(())
    }

    #[must_use]
    pub fn delegates_for(&self, output: &Output) -> Option<&[String]> {
        self.delegates.get(output).map(std::vec::Vec::as_slice)
    }
}

// UnmanagedCallersOnly and delegate* do not run the runtime marshaller. Inspect the
// actual wire representation, retaining delegate marshalling for strings and bools.
fn supports_function_pointer(id: TypeId, types: &model::common::types::all::Pass) -> bool {
    let Some(ty) = types.get(id) else { return false };
    if ty.decorators.param.is_some() || ty.decorators.rval.is_some() {
        return false;
    }
    match &ty.kind {
        TypeKind::Primitive(p) => *p != Primitive::Bool,
        TypeKind::Pointer(p) => matches!(p.kind, PointerKind::IntPtr(_)),
        TypeKind::Delegate(d) => d.kind == DelegateKind::Class,
        TypeKind::Composite(c) => c.fields.iter().all(|f| supports_function_pointer(f.ty, types)),
        TypeKind::Array(a) => supports_function_pointer(a.ty, types),
        TypeKind::DataEnum(e) | TypeKind::TypePattern(TypePattern::Option(_, e) | TypePattern::Result(_, _, e)) => {
            e.variants.iter().flat_map(Variant::payloads).all(|p| supports_function_pointer(p.ty, types))
        }
        TypeKind::TypePattern(p) => !matches!(p, TypePattern::CStrPointer),
        _ => false,
    }
}
