//! Writes function import declarations for simple overloads.
//!
//! Simple overloads do not contain a body — they are plain `DllImport` declarations.
//! The overload functions are identified via their `FunctionKind::Overload` with
//! `OverloadKind::Simple`, queried from the central `fns::all` pass.

use crate::lang::functions::FunctionKind;
use crate::lang::functions::overload::OverloadKind;
use crate::output::{FileType, Output};
use crate::pass::{OutputResult, PassInfo, format_docs, model, output};
use interoptopus_backends::template::Context;
use std::collections::HashMap;

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
    ) -> OutputResult {
        let templates = output_master.templates();

        for output in output_master.outputs_of(FileType::Csharp) {
            let mut imports = Vec::new();

            for (&overload_id, function) in fns_all.overloads() {
                // Only simple overloads get rendered here
                let FunctionKind::Overload(ref overload) = function.kind else { continue };
                if !matches!(overload.kind, OverloadKind::Simple) {
                    continue;
                }

                if !output_master.fn_belongs_to(overload.base, output) {
                    continue;
                }

                let name = &function.name;
                let rval = types
                    .managed_name(function.signature.rval)
                    .ok_or_else(|| crate::Error::from(format!("rval of overload `{name}`")))?;

                let mut args: Vec<HashMap<&str, String>> = Vec::new();
                for arg in &function.signature.arguments {
                    let arg_type = types
                        .get(arg.ty)
                        .ok_or_else(|| crate::Error::from(format!("arg `{}` of overload `{}`", arg.name, name)))?;
                    let mut m = HashMap::new();
                    m.insert("name", arg.name.clone());
                    let decorated = match &arg_type.decorators.param {
                        Some(d) => format!("{d} {}", arg_type.name),
                        None => types.managed_name(arg.ty).expect("resolved argument type"),
                    };
                    let decorated = if types.tuple_name(arg.ty).is_some() {
                        format!("[MarshalUsing(typeof({}.TupleMarshallerMeta))] {decorated}", arg_type.name)
                    } else { decorated };
                    m.insert("ty", decorated);
                    args.push(m);
                }

                let docs = format_docs(&function.docs);
                let mut context = Context::new();

                context.insert("name", name);
                context.insert("symbol", name);
                context.insert("args", &args);
                context.insert("rval", &rval);
                let rval_decorator = types.tuple_name(function.signature.rval).map(|_| {
                    format!("return: MarshalUsing(typeof({}.TupleMarshallerMeta))", types.get(function.signature.rval).expect("resolved return type").name)
                });
                context.insert("rval_decorator", &rval_decorator);
                context.insert("docs", &docs);
                context.insert("visibility", &function.visibility.to_string());

                let import = templates.render("rust/fns/overload/simple.cs", &context)?;
                imports.push(import);
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
