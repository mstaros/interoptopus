//! Renders utility types (exceptions, string extensions) per output file.
//!
//! Each utility type has a registered `TypeId` (see `lang::types::csharp`) and
//! participates in dispatch routing. Only utility types routed to a given
//! output file are rendered into that file.

use crate::lang::types::csharp;
use crate::output::{FileType, Output};
use crate::pass::{OutputResult, PassInfo, model, output};
use interoptopus_backends::template::Context;
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    utils: HashMap<Output, String>,
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, utils: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        types: &model::common::types::all::Pass,
    ) -> OutputResult {
        let templates = output_master.templates();

        for file in output_master.outputs_of(FileType::Csharp) {
            let mut parts = Vec::new();

            if output_master.type_belongs_to(csharp::UTIL_INTEROP_EXCEPTION, file) {
                let mut ctx = Context::new();
                ctx.insert("visibility", &visibility_of(csharp::UTIL_INTEROP_EXCEPTION, types));
                parts.push(templates.render("common/types/util/interop_exception.cs", &ctx)?.trim().to_string());
            }
            if output_master.type_belongs_to(csharp::UTIL_ENUM_EXCEPTION, file) {
                let mut ctx = Context::new();
                ctx.insert("visibility", &visibility_of(csharp::UTIL_ENUM_EXCEPTION, types));
                parts.push(templates.render("common/types/util/enum_exception.cs", &ctx)?.trim().to_string());
            }
            if output_master.type_belongs_to(csharp::UTIL_ASYNC_CALLBACK_COMMON, file) {
                let mut ctx = Context::new();
                ctx.insert("visibility", &visibility_of(csharp::UTIL_ASYNC_CALLBACK_COMMON, types));
                parts.push(templates.render("common/types/util/async_callback_common.cs", &ctx)?.trim().to_string());
            }
            if output_master.type_belongs_to(csharp::UTIL_CONST_CSTR_MARSHALLER, file) {
                let mut ctx = Context::new();
                ctx.insert("visibility", &visibility_of(csharp::UTIL_CONST_CSTR_MARSHALLER, types));
                parts.push(templates.render("common/types/util/const_cstr_marshaller.cs", &ctx)?.trim().to_string());
            }
            if output_master.type_belongs_to(csharp::UTIL_TASK_HANDLE, file) {
                let mut ctx = Context::new();
                ctx.insert("visibility", &visibility_of(csharp::UTIL_TASK_HANDLE, types));
                parts.push(templates.render("common/types/util/task_handle.cs", &ctx)?.trim().to_string());
            }
            if output_master.type_belongs_to(csharp::UTIL_RESULT, file) {
                parts.push(templates.render("common/types/util/result.cs", &Context::new())?.trim().to_string());
            }

            self.utils.insert(file.clone(), parts.join("\n\n"));
        }
        Ok(())
    }

    #[must_use]
    pub fn utils_for(&self, output: &Output) -> Option<&str> {
        self.utils.get(output).map(|s| &**s)
    }
}

fn visibility_of(type_id: crate::lang::TypeId, types: &model::common::types::all::Pass) -> String {
    types.get(type_id).map_or_else(|| "public".to_string(), |t| t.visibility.to_string())
}


/// Emits cleanup of a managed value, including native leaves inside managed collections.
#[must_use]
pub fn dispose_value(
    ty: crate::lang::TypeId,
    value: &str,
    types: &model::common::types::all::Pass,
    disposable: &model::common::types::info::disposable::Pass,
    depth: usize,
) -> String {
    use crate::lang::types::kind::TypeKind;
    use crate::lang::types::kind::wire::WireOnly;
    if !disposable.is_disposable(ty).unwrap_or(false) { return String::new(); }
    match types.get(ty).map(|ty| &ty.kind) {
        Some(TypeKind::Array(crate::lang::types::kind::Array { ty: inner, .. }) | TypeKind::WireOnly(WireOnly::Vec(inner))) => {
            let item = format!("__disposeItem{depth}");
            let body = dispose_value(*inner, &item, types, disposable, depth + 1);
            format!("if ({value} is not null) foreach (var {item} in {value}) {{ {body} }}")
        }
        Some(TypeKind::WireOnly(WireOnly::Map(key, item))) => {
            let pair = format!("__disposePair{depth}");
            let key = dispose_value(*key, &format!("{pair}.Key"), types, disposable, depth + 1);
            let item = dispose_value(*item, &format!("{pair}.Value"), types, disposable, depth + 1);
            format!("if ({value} is not null) foreach (var {pair} in {value}) {{ {key} {item} }}")
        }
        Some(TypeKind::WireOnly(WireOnly::Nullable(inner))) => dispose_value(*inner, value, types, disposable, depth + 1),
        _ => format!("try {{ {value}?.Dispose(); }} catch (Exception error) {{ (__disposeErrors ??= new()).Add(error); }}"),
    }
}

/// Attempts every leaf and preserves a single exception's original identity and stack.
pub fn dispose_body(statements: impl IntoIterator<Item = String>) -> String {
    let statements = statements.into_iter().filter(|statement| !statement.is_empty()).collect::<Vec<_>>();
    if statements.is_empty() { return String::new(); }
    format!(
        "List<Exception> __disposeErrors = null;\n{}\nif (__disposeErrors is {{ Count: 1 }}) System.Runtime.ExceptionServices.ExceptionDispatchInfo.Capture(__disposeErrors[0]).Throw();\nif (__disposeErrors is not null) throw new AggregateException(\"Resource cleanup failed.\", __disposeErrors);",
        statements.join("\n")
    )
}
