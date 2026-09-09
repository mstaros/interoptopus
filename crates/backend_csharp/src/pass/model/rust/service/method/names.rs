//! Computes C# method names for service functions (constructors, methods, destructor).
//!
//! Given a service type name like `ServiceBasic` and a function name like
//! `service_basic_do_something`, this pass strips the `snake_case` service prefix
//! and `PascalCases` the remainder to produce `DoSomething`.

use crate::lang::FunctionId;
use crate::pass::Outcome::Unchanged;
use crate::pass::{ModelResult, PassInfo, model};
use interoptopus_backends::casing::service_method_name;
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    names: HashMap<FunctionId, String>,
}

fn managed_method_name(type_name: &str, func: &crate::lang::functions::Function, types: &model::common::types::all::Pass) -> String {
    use crate::lang::functions::{FunctionKind, overload::OverloadKind};
    use crate::lang::types::kind::{TypeKind, TypePattern};
    let is_async = matches!(&func.kind, FunctionKind::Overload(overload) if matches!(overload.kind, OverloadKind::Async(_)))
        || func.signature.arguments.iter().any(|arg| types.get(arg.ty).is_some_and(|ty| matches!(ty.kind, TypeKind::TypePattern(TypePattern::AsyncCallback(_)))));
    let mut name = service_method_name(type_name, &func.name);
    if is_async && !name.ends_with("Async") {
        name.push_str("Async");
    }
    name
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, names: HashMap::default() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        services: &model::common::service::all::Pass,
        fns: &model::common::fns::all::Pass,
        types: &model::common::types::all::Pass,
    ) -> ModelResult {
        let mut outcome = Unchanged;

        for (_service_id, service) in services.iter() {
            let Some(type_name) = types.get(service.ty).map(|t| &t.name) else { continue };

            let mut declared_names = HashMap::new();
            for fn_id in service.sources.ctors.iter().chain(service.sources.methods.iter()) {
                let Some(func) = fns.get(*fn_id) else { continue };
                let name = managed_method_name(type_name, func, types);
                if let Some(previous) = declared_names.insert(name.clone(), *fn_id) {
                    if previous != *fn_id {
                        return Err(format!("Service {type_name} has conflicting generated member {name}; rename one Rust method.").into());
                    }
                }
            }

            let source_fns: Vec<_> = service
                .sources
                .ctors
                .iter()
                .chain(service.sources.methods.iter())
                .chain(service.ctors.iter())
                .chain(service.methods.iter())
                .chain(std::iter::once(&service.destructor))
                .copied()
                .collect();

            for fn_id in &source_fns {
                if self.names.contains_key(fn_id) {
                    continue;
                }

                let Some(func) = fns.get(*fn_id) else { continue };

                let method_name = managed_method_name(type_name, func, types);

                self.names.insert(*fn_id, method_name);
                outcome.changed();
            }

            // Also assign names to overloads of source functions so they're
            // available when `service_method_overload` adds them to the
            // renderable lists.
            for &fn_id in service.sources.ctors.iter().chain(service.sources.methods.iter()) {
                for (overload_id, overload_fn) in fns.overloads_for(fn_id) {
                    if self.names.contains_key(overload_id) {
                        continue;
                    }

                    let method_name = managed_method_name(type_name, overload_fn, types);
                    self.names.insert(*overload_id, method_name);
                    outcome.changed();
                }
            }
        }

        Ok(outcome)
    }

    #[must_use]
    pub fn get(&self, fn_id: FunctionId) -> Option<&str> {
        self.names.get(&fn_id).map(std::string::String::as_str)
    }
}
