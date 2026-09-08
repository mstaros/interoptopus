//! Writes function import declarations.

use crate::lang::types::{ManagedConversion, ParamDecorator};
use crate::lang::types::kind::{Primitive, TypeKind, TypePattern};
use crate::output::{FileType, Output};
use crate::pass::{OutputResult, PassInfo, format_docs, model, output};
use interoptopus_backends::template::Context;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    fn_imports: HashMap<Output, Vec<String>>,
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, fn_imports: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        fns_all: &model::common::fns::all::Pass,
        types: &model::common::types::all::Pass,
        managed_conversion: &model::common::types::info::managed_conversion::Pass,
    ) -> OutputResult {
        let templates = output_master.templates();

        for output in output_master.outputs_of(FileType::Csharp) {
            let mut imports = Vec::new();

            for (&id, function) in fns_all.originals() {
                if !output_master.fn_belongs_to(id, output) {
                    continue;
                }

                let name = &function.name;
                let rval_type = types
                    .get(function.signature.rval)
                    .ok_or_else(|| crate::Error::from(format!("rval of function `{name}`")))?;
                let rval = &rval_type.name;

                let mut args: Vec<HashMap<&str, String>> = Vec::new();
                for arg in &function.signature.arguments {
                    let arg_type = types
                        .get(arg.ty)
                        .ok_or_else(|| crate::Error::from(format!("arg `{}` of function `{}`", arg.name, name)))?;
                    let mut m = HashMap::new();
                    m.insert("name", arg.name.clone());
                    let decorated = match &arg_type.decorators.param {
                        Some(d) => format!("{d} {}", arg_type.name),
                        None => arg_type.name.clone(),
                    };
                    m.insert("ty", decorated);
                    args.push(m);
                }

                let mut context = Context::new();

                let rval_decorator = rval_type.decorators.rval.as_ref().map(|d| match d {
                    crate::lang::types::RvalDecorator::MarshalAs(m) => format!("return: MarshalAs({m})"),
                    crate::lang::types::RvalDecorator::MarshalUsing(t) => format!("return: MarshalUsing(typeof({t}))"),
                });

                let docs = format_docs(&function.docs);

                context.insert("name", name);
                context.insert("symbol", name);
                context.insert("args", &args);
                context.insert("rval", rval);
                context.insert("rval_decorator", &rval_decorator);
                context.insert("docs", &docs);
                context.insert("visibility", &function.visibility.to_string());

                let import = templates.render("rust/fns/rust.cs", &context)?;
                imports.push(import);

                // A span borrows memory only until this synchronous import returns.
                if !span_return_is_independent(function.signature.rval, types)
                    || function.signature.arguments.iter().any(|arg| {
                        types.get(arg.ty).is_some_and(|ty| matches!(ty.kind, TypeKind::TypePattern(TypePattern::AsyncCallback(_))))
                    })
                {
                    continue;
                }
                let mut span_args = args.clone();
                let mut raw_args = args;
                let mut pins: Vec<HashMap<&str, String>> = Vec::new();
                for (index, arg) in function.signature.arguments.iter().enumerate() {
                    let ty = types.get(arg.ty).expect("argument resolved above");
                    let prefix = match &ty.decorators.param {
                        Some(ParamDecorator::In { .. }) => "in ",
                        Some(ParamDecorator::Ref) => "ref ",
                        Some(ParamDecorator::Out) => "out ",
                        _ => "",
                    };
                    span_args[index].insert("call", format!("{prefix}{}", arg.name));
                    let Some((element_ty, mutable)) = span_element(arg.ty, types, managed_conversion) else { continue };
                    let span = if mutable { "Span" } else { "ReadOnlySpan" };
                    span_args[index].insert("ty", format!("{span}<{}>", element_ty.name));
                    let pointer = format!("__span_ptr_{index}");
                    span_args[index].insert("call", format!(
                        "new {}.Unmanaged {{ _data = (IntPtr){pointer}, _len = (ulong){}.Length }}", ty.name, arg.name
                    ));
                    raw_args[index].insert("ty", format!("{}.Unmanaged", ty.name));
                    let mut pin = HashMap::new();
                    pin.insert("element", element_ty.name.clone());
                    pin.insert("pointer", pointer);
                    pin.insert("name", arg.name.clone());
                    pins.push(pin);
                }
                if !pins.is_empty() {
                    let raw_name = format!("__span_{name}");
                    context.insert("name", &raw_name);
                    context.insert("visibility", "private");
                    context.insert("args", &raw_args);
                    context.insert("docs", "");
                    imports.push(templates.render("rust/fns/rust.cs", &context)?);
                    context.insert("name", name);
                    context.insert("visibility", &function.visibility.to_string());
                    context.insert("args", &span_args);
                    context.insert("raw_name", &raw_name);
                    context.insert("pins", &pins);
                    context.insert("is_void", &matches!(rval_type.kind, TypeKind::Primitive(Primitive::Void)));
                    imports.push(templates.render("rust/fns/overload/span.cs", &context)?);
                }
            }

            imports.sort();

            self.fn_imports.insert(output.clone(), imports);
        }

        Ok(())
    }

    #[must_use]
    pub fn imports_for(&self, output: &Output) -> Option<&[String]> {
        self.fn_imports.get(output).map(std::vec::Vec::as_slice)
    }
}

pub(crate) fn span_element<'a>(
    id: crate::lang::TypeId,
    types: &'a model::common::types::all::Pass,
    managed_conversion: &model::common::types::info::managed_conversion::Pass,
) -> Option<(&'a crate::lang::types::Type, bool)> {
    let ty = types.get(id)?;
    let (element, mutable) = match ty.kind {
        TypeKind::TypePattern(TypePattern::Slice(element)) => (element, false),
        TypeKind::TypePattern(TypePattern::SliceMut(element)) => (element, true),
        _ => return None,
    };
    let element_ty = types.get(element)?;
    (ty.decorators.param.is_none()
        && managed_conversion.managed_conversion(element) == Some(ManagedConversion::AsIs)
        && !matches!(element_ty.kind, TypeKind::Delegate(_) | TypeKind::TypePattern(TypePattern::CStrPointer)))
        .then_some((element_ty, mutable))
}

/// Temporary span pins cannot back returned borrowed views, even inside owned containers.
pub(crate) fn span_return_is_independent(id: crate::lang::TypeId, types: &model::common::types::all::Pass) -> bool {
    fn independent(id: crate::lang::TypeId, types: &model::common::types::all::Pass, visiting: &mut HashSet<crate::lang::TypeId>) -> bool {
        if !visiting.insert(id) {
            return false;
        }
        let safe = match types.get(id).map(|ty| &ty.kind) {
            Some(TypeKind::Primitive(_)) => true,
            Some(TypeKind::Composite(c)) => c.fields.iter().all(|field| independent(field.ty, types, visiting)),
            Some(TypeKind::Array(a)) => independent(a.ty, types, visiting),
            Some(TypeKind::DataEnum(e) | TypeKind::TypePattern(TypePattern::Option(_, e) | TypePattern::Result(_, _, e))) => {
                e.variants.iter().flat_map(crate::lang::types::kind::Variant::payloads).all(|payload| independent(payload.ty, types, visiting))
            }
            Some(TypeKind::TypePattern(TypePattern::Vec(element))) => independent(*element, types, visiting),
            Some(TypeKind::TypePattern(TypePattern::Utf8String | TypePattern::Wire(_) | TypePattern::Bool | TypePattern::CChar | TypePattern::Version)) => true,
            _ => false,
        };
        visiting.remove(&id);
        safe
    }
    independent(id, types, &mut HashSet::new())
}
