//! Renders body and async overload function declarations.
//!
//! Iterates `fns::all` for overloads with `Body` and `Async` kinds. For each,
//! resolves arg transforms from the `Overload` data stored in the function's
//! `FunctionKind`, and renders via the shared `fns/overload/body.cs` template.
//! Async overloads set `is_async = true` which switches the template to
//! trampoline + Task return.

use crate::lang::FunctionId;
use crate::lang::functions::FunctionKind;
use crate::lang::functions::overload::{ArgTransform, FnTransforms, OverloadKind, RvalTransform};
use crate::lang::functions::{Argument, Function};
use crate::lang::types::{OverloadFamily, ParamDecorator};
use crate::lang::types::kind::{Primitive, TypeKind, TypePattern};
use crate::output::{FileType, Output};
use crate::pass::model::rust::fns::overload::is_mutable_service_pointer;
use crate::pass::{OutputResult, PassInfo, format_docs, model, output};
use interoptopus_backends::template::{Context, TemplateEngine, Value};
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    body_imports: HashMap<Output, Vec<String>>,
    async_imports: HashMap<Output, Vec<String>>,
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, body_imports: HashMap::default(), async_imports: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        fns_all: &model::common::fns::all::Pass,
        types: &model::common::types::all::Pass,
        type_overloads: &model::rust::types::overload::all::Pass,
        trampoline: &model::rust::types::info::trampoline::Pass,
        services: &model::common::service::all::Pass,
    ) -> OutputResult {
        let templates = output_master.templates();

        for file in output_master.outputs_of(FileType::Csharp) {
            let mut body = Vec::new();
            let mut asynk = Vec::new();

            for (&overload_id, function) in fns_all.overloads() {
                let FunctionKind::Overload(ref overload) = function.kind else { continue };

                if !output_master.fn_belongs_to(overload.base, file) {
                    continue;
                }

                // Look up the original function for context (native args, rval)
                let Some(original_fn) = fns_all.get(overload.base) else { continue };
                let is_destructor = services.iter().any(|(_, service)| service.destructor == overload.base);
                let is_constructor = services.iter().any(|(_, service)| service.sources.ctors.contains(&overload.base));

                match &overload.kind {
                    OverloadKind::Body(transforms) => {
                        body.push(render(original_fn, function, transforms, types, type_overloads, trampoline, overload_id, templates, is_destructor, is_constructor)?);
                    }
                    OverloadKind::Async(transforms) => {
                        asynk.push(render(original_fn, function, transforms, types, type_overloads, trampoline, overload_id, templates, is_destructor, is_constructor)?);
                    }
                    OverloadKind::Simple => {}
                }
            }

            body.sort();
            asynk.sort();
            self.body_imports.insert(file.clone(), body);
            self.async_imports.insert(file.clone(), asynk);
        }

        Ok(())
    }

    #[must_use]
    pub fn body_imports_for(&self, output: &Output) -> Option<&[String]> {
        self.body_imports.get(output).map(std::vec::Vec::as_slice)
    }

    #[must_use]
    pub fn async_imports_for(&self, output: &Output) -> Option<&[String]> {
        self.async_imports.get(output).map(std::vec::Vec::as_slice)
    }
}

fn render(
    original_fn: &Function,
    overload_fn: &Function,
    transforms: &FnTransforms,
    types: &model::common::types::all::Pass,
    type_overloads: &model::rust::types::overload::all::Pass,
    trampoline: &model::rust::types::info::trampoline::Pass,
    overload_id: FunctionId,
    templates: &TemplateEngine,
    is_destructor: bool,
    is_constructor: bool,
) -> Result<String, crate::Error> {
    let name = &original_fn.name;
    let is_async = matches!(transforms.rval, RvalTransform::AsyncTask(_));

    // For async: original args exclude the callback (last); for body: all args.
    // These are the args that map 1:1 to native call forwarding.
    let original_args = if is_async {
        &original_fn.signature.arguments[..original_fn.signature.arguments.len() - 1]
    } else {
        &*original_fn.signature.arguments
    };

    // Resolve overloaded arg types + detect wraps.
    // The overload's transform list may be longer than original_args (e.g., a trailing
    // CancellationToken has no native counterpart), so we pass the overload signature.
    let (args, has_wraps) = resolve_args(&overload_fn.signature.arguments, &transforms.args, types, type_overloads, name)?;

    // Build native call forwarding names — only for args that have a native counterpart
    // (i.e., skip synthetic args like CancellationToken).
    let native_args = build_native_args(original_args, &overload_fn.signature.arguments, &transforms.args, types, name)?;

    // Return type: use the overload function's rval directly (Task type for async, original for body)
    let rval = types
        .managed_name(overload_fn.signature.rval)
        .ok_or_else(|| crate::Error::from(format!("rval of overload `{name}`")))?;

    let is_void = !is_async && matches!(types.get(original_fn.signature.rval).map(|t| &t.kind), Some(TypeKind::Primitive(Primitive::Void)));

    let native_rval_is_result = is_async && matches!(types.get(original_fn.signature.rval).map(|t| &t.kind), Some(TypeKind::TypePattern(TypePattern::Result(_, _, _))));

    let result_local = service_context_name(overload_fn.signature.arguments.len(), &overload_fn.signature.arguments);
    let copy_result = borrowed_result_copy(original_fn, transforms, types, is_constructor, &result_local)?;

    let docs = format_docs(&overload_fn.docs);
    let mut context = Context::new();
    context.insert("name", name);
    context.insert("copy_result", &copy_result);
    context.insert("result_local", &result_local);
    context.insert("rval", &rval);
    context.insert("is_void", &is_void);
    context.insert("is_async", &is_async);
    context.insert("is_value_task", &matches!(types.get(overload_fn.signature.rval).map(|ty| &ty.kind), Some(TypeKind::Task(task)) if task.value_task));
    context.insert("is_task_void", &false);
    context.insert("has_wraps", &has_wraps);
    context.insert("args", &args);
    context.insert("native_args", &native_args);
    context.insert("native_rval_is_result", &native_rval_is_result);
    context.insert("docs", &docs);
    context.insert("visibility", &overload_fn.visibility.to_string());

    if let RvalTransform::AsyncTask(_) = transforms.rval
        && let Some(t) = trampoline.for_function(overload_id)
    {
        context.insert("trampoline_field", &crate::pass::output::rust::types::asynk_naming::field_name(t, types));
        let is_task_void = t.is_task_void();
        context.insert("is_task_void", &is_task_void);
    }

    // A typed destructor must close the managed owner, never destroy a borrowed pointer.
    let mut call_body = if is_destructor {
        format!("{}.Dispose();", overload_fn.signature.arguments[0].name)
    } else {
        templates.render("rust/fns/overload/body_call.cs", &context)?
    };
    // Nest acquisitions so a later argument failure releases every earlier service.
    // For async calls the scope includes native acknowledgement (also on cancellation).
    for (index, (arg, transform)) in overload_fn.signature.arguments.iter().zip(&transforms.args).enumerate().rev() {
        if !is_destructor && matches!(transform, ArgTransform::Service) {
            let context_name = service_context_name(index, &overload_fn.signature.arguments);
            let exclusive = if is_mutable_service_pointer(original_args[index].ty, types) { "true" } else { "" };
            let mut indented = String::new();
            for line in call_body.lines() {
                indented.push_str("    ");
                indented.push_str(line);
                indented.push('\n');
            }
            call_body = format!(
                "var {context_name} = {name}.__AcquireCall({exclusive});\ntry\n{{\n{indented}}}\nfinally\n{{\n    {name}.__ReleaseCall({exclusive});\n}}\n",
                name = arg.name,
            );
        }
    }
    context.insert("call_body", call_body.trim_end());
    templates.render("rust/fns/overload/body.cs", &context).map_err(Into::into)
}

fn service_context_name(index: usize, args: &[Argument]) -> String {
    let mut name = format!("__service_context_{index}");
    while args.iter().any(|arg| arg.name.trim_start_matches('@') == name) {
        name.push('_');
    }
    name
}

fn resolve_args(
    args: &[Argument],
    transforms: &[ArgTransform],
    types: &model::common::types::all::Pass,
    type_overloads: &model::rust::types::overload::all::Pass,
    fn_name: &str,
) -> Result<(Vec<HashMap<&'static str, Value>>, bool), crate::Error> {
    let mut out = Vec::new();
    let mut has_wraps = false;

    for (arg, transform) in args.iter().zip(transforms) {
        let mut m = HashMap::new();
        m.insert("name", Value::normal_string(&arg.name));

        match transform {
            ArgTransform::PassThrough => {
                let arg_type = types
                    .get(arg.ty)
                    .ok_or_else(|| crate::Error::from(format!("arg `{}` of overload `{}`", arg.name, fn_name)))?;
                let decorated = match &arg_type.decorators.param {
                    Some(d) => format!("{d} {}", arg_type.name),
                    None => types.managed_name(arg.ty).expect("resolved argument type"),
                };
                m.insert("ty", Value::normal_string(&decorated));
                m.insert("is_ref", Value::normal_string("false"));
                m.insert("is_wrap", Value::normal_string("false"));
            }
            ArgTransform::Ref => {
                let Some(OverloadFamily::Pointer(family)) = type_overloads.get(arg.ty) else {
                    return Err(crate::Error::from(format!("pointer family for arg `{}` of overload `{}`", arg.name, fn_name)));
                };
                let arg_type = types
                    .get(family.by_ref)
                    .ok_or_else(|| crate::Error::from(format!("ref type for arg `{}` of overload `{}`", arg.name, fn_name)))?;
                let decorated = match &arg_type.decorators.param {
                    Some(d) => format!("{d} {}", arg_type.name),
                    None => arg_type.name.clone(),
                };
                m.insert("ty", Value::normal_string(&decorated));
                m.insert("is_ref", Value::normal_string("true"));
                m.insert("is_wrap", Value::normal_string("false"));
            }
            ArgTransform::WrapDelegate => {
                let Some(OverloadFamily::Delegate(family)) = type_overloads.get(arg.ty) else {
                    return Err(crate::Error::from(format!("delegate family for arg `{}` of overload `{}`", arg.name, fn_name)));
                };
                let sig_name = types
                    .managed_name(family.signature)
                    .ok_or_else(|| crate::Error::from(format!("delegate sig for arg `{}` of overload `{}`", arg.name, fn_name)))?;
                let class_name = types
                    .get(family.class)
                    .map(|t| &t.name)
                    .ok_or_else(|| crate::Error::from(format!("delegate class for arg `{}` of overload `{}`", arg.name, fn_name)))?;
                m.insert("ty", Value::normal_string(&sig_name));
                m.insert("is_ref", Value::normal_string("false"));
                m.insert("is_wrap", Value::normal_string("true"));
                m.insert("wrapper_type", Value::normal_string(class_name));
                has_wraps = true;
            }
            ArgTransform::Service => {
                // The overload arg already has the service TypeId (not the original IntPtr).
                let service_ty = types
                    .get(arg.ty)
                    .ok_or_else(|| crate::Error::from(format!("service type for arg `{}` of overload `{}`", arg.name, fn_name)))?;
                m.insert("ty", Value::normal_string(&service_ty.name));
                m.insert("is_ref", Value::normal_string("false"));
                m.insert("is_wrap", Value::normal_string("false"));
            }
            ArgTransform::CancellationToken => {
                m.insert("ty", Value::normal_string("CancellationToken"));
                m.insert("is_ref", Value::normal_string("false"));
                m.insert("is_wrap", Value::normal_string("false"));
                m.insert("has_default", Value::normal_string("true"));
                m.insert("default_value", Value::normal_string("default"));
            }
        }
        // Ensure has_default is always present for template access
        m.entry("has_default").or_insert_with(|| Value::normal_string("false"));
        out.push(m);
    }

    Ok((out, has_wraps))
}

fn build_native_args(
    args: &[Argument],
    overload_args: &[Argument],
    transforms: &[ArgTransform],
    types: &model::common::types::all::Pass,
    fn_name: &str,
) -> Result<Vec<HashMap<&'static str, Value>>, crate::Error> {
    args.iter()
        .zip(overload_args)
        .zip(transforms)
        .enumerate()
        .map(|(index, ((arg, overload_arg), transform))| {
            let forwarded = match transform {
                ArgTransform::WrapDelegate => format!("{}_wrapped", arg.name),
                ArgTransform::Ref => {
                    let decorator = types
                        .get(overload_arg.ty)
                        .and_then(|t| t.decorators.param.as_ref())
                        .ok_or_else(|| crate::Error::from(format!("by-ref decorator for arg `{}` of overload `{}`", arg.name, fn_name)))?;
                    let modifier = match decorator {
                        ParamDecorator::In { .. } => "in",
                        ParamDecorator::Ref => "ref",
                        _ => return Err(crate::Error::from(format!("invalid by-ref decorator for arg `{}` of overload `{}`", arg.name, fn_name))),
                    };
                    format!("{modifier} {}", arg.name)
                }
                ArgTransform::Service => service_context_name(index, overload_args),
                ArgTransform::PassThrough => arg.name.clone(),
                ArgTransform::CancellationToken => unreachable!("CancellationToken has no native counterpart"),
            };
            let mut m = HashMap::new();
            m.insert("name", Value::normal_string(&forwarded));
            Ok(m)
        })
        .collect()
}

fn borrowed_result_copy(
    original: &Function,
    transforms: &FnTransforms,
    types: &model::common::types::all::Pass,
    is_constructor: bool,
    local: &str,
) -> Result<Option<String>, crate::Error> {
    if is_constructor || !transforms.args.iter().any(|arg| matches!(arg, ArgTransform::Service)) {
        return Ok(None);
    }
    let result_type = match transforms.rval {
        RvalTransform::AsyncTask(inner) => inner,
        _ => original.signature.rval,
    };
    let copy = output::rust::fns::rust::service_result_copy(result_type, types, &original.name, local)?;
    if matches!(transforms.rval, RvalTransform::AsyncTask(_)) && copy.is_some() {
        return Err(crate::Error::from(format!("Cannot emit async service call `{}` with a borrowed result; return owned data.", original.name)));
    }
    Ok(copy)
}
