//! Emits owned iterator wrappers and their query extensions.
use crate::lang::TypeId;
use crate::lang::types::ManagedConversion;
use crate::lang::types::kind::{Primitive, TypeKind, TypePattern};
use crate::output::{FileType, Output};
use crate::pass::{OutputResult, PassInfo, model, output};
use interoptopus_backends::template::Context;
use std::collections::HashMap;

#[derive(Default)]
pub struct Config {}

pub struct Pass {
    info: PassInfo,
    iterators: HashMap<Output, Vec<String>>,
}

/// Values copied into a predicate must not retain borrowed or owned native state.
fn predicate_value(id: TypeId, types: &model::common::types::all::Pass) -> bool {
    let Some(ty) = types.get(id) else { return false };
    match &ty.kind {
        TypeKind::Primitive(p) => !matches!(p, Primitive::Void),
        TypeKind::TypePattern(TypePattern::Bool | TypePattern::CChar | TypePattern::Version) => true,
        TypeKind::Composite(c) => c.fields.iter().all(|f| predicate_value(f.ty, types)),
        TypeKind::DataEnum(e) => !e.is_union_projected(),
        _ => false,
    }
}

impl Pass {
    #[must_use]
    pub fn new(_: Config) -> Self {
        Self { info: PassInfo { name: file!() }, iterators: HashMap::new() }
    }

    pub fn process(
        &mut self,
        _pass_meta: &mut crate::pass::PassMeta,
        output_master: &output::common::master::Pass,
        types: &model::common::types::all::Pass,
        conversion: &model::common::types::info::managed_conversion::Pass,
    ) -> OutputResult {
        for file in output_master.outputs_of(FileType::Csharp) {
            let mut rendered = Vec::new();
            for (id, ty) in types.iter() {
                let (item, asynchronous) = match ty.kind {
                    TypeKind::TypePattern(TypePattern::Iterator(item)) => (item, false),
                    TypeKind::TypePattern(TypePattern::AsyncIterator(item)) => (item, true),
                    _ => continue,
                };
                if !output_master.type_belongs_to(*id, file) { continue; }
                let Some(element) = types.get(item) else { continue };
                let item_conversion = conversion.managed_conversion(item);
                if !predicate_value(item, types) || !matches!(item_conversion, Some(ManagedConversion::AsIs | ManagedConversion::To)) {
                    return Err(crate::Error::from(format!(
                        "Cannot emit C# iterator `{}`: element `{}` is unsupported. Iterator predicates support scalars, plain enums, and structs composed of those values; borrowed and owning elements are not supported.",
                        ty.name, element.name
                    )));
                }
                let api_name = types.managed_name(item).expect("resolved iterator element");
                let managed_element = if matches!(element.kind, TypeKind::Primitive(Primitive::Bool) | TypeKind::TypePattern(TypePattern::Bool)) {
                    "bool"
                } else { &api_name };
                let read_element = if item_conversion == Some(ManagedConversion::To) {
                    format!("(({}.Unmanaged*)item)->ToManaged()", element.name)
                } else {
                    format!("*({}*)item", element.name)
                };
                let mut context = Context::new();
                context.insert("name", &ty.name);
                context.insert("read_element", &read_element);
                let unmanaged_element = if item_conversion == Some(ManagedConversion::To) {
                    format!("{}.Unmanaged", element.name)
                } else { element.name.clone() };
                let copied_element = if item_conversion == Some(ManagedConversion::To) { "item.ToManaged()" } else { "item" };
                context.insert("unmanaged_element_type", &unmanaged_element);
                context.insert("copied_element", copied_element);
                context.insert("managed_element_type", managed_element);
                let template = if asynchronous { "rust/pattern/async_iterator.cs" } else { "rust/pattern/iterator.cs" };
                rendered.push(output_master.templates().render(template, &context)?);
            }
            rendered.sort();
            self.iterators.insert(file.clone(), rendered);
        }
        Ok(())
    }

    #[must_use]
    pub fn iterators_for(&self, output: &Output) -> Option<&[String]> {
        self.iterators.get(output).map(Vec::as_slice)
    }
}
