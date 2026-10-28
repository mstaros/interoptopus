//! Emits the public C# layer (design step 3, "generate"; todo E7) over the bindings interoptopus
//! generates for the facade.
//!
//! The service becomes a sealed, disposable class in the managed namespace: its constructors become
//! static factories and its `&self` methods become methods. A keyed type becomes a sealed class
//! holding its repository (the service) and its key, whose methods call the service with the key;
//! keyed results become such objects, and a keyed parameter from another repository throws
//! `ArgumentException`. Free functions become static methods of a static class named after their
//! module, so `gix::env::agent` becomes `Gix.Env.Agent()`. Native member names come from
//! `interoptopus_backends::casing`, the functions the C# backend itself uses. Conversions and error
//! translation go through the hand-written C# runtime class (configuration `csharp_runtime`), which
//! provides:
//!
//! - `Call(operation, func)` and `Call(operation, action)`, translating native errors;
//! - `EncodePath(string) -> byte[]`;
//! - `Text`, `Bytes`, `Path` and `Flag` over owned native values and options (`Text` also over
//!   optional text, for optional keys), and `Text`, `Bytes`, `Flag`, `Value` and `Done` over
//!   free-function results (`IResult<T, TError>`).
//!
//! The runtime's overloads follow the native types the generated code uses: a native type exists
//! only once a generated member needs it, and a missing overload is a compile error naming it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use interoptopus_backends::casing::{pascal_to_snake, rust_to_pascal, service_method_name};

use crate::classify::Carrier;
use crate::facade::{GeneratedMember, KeyedType, MemberKind, Shape};
use crate::forbidden::is_forbidden;

/// What the C# emitter needs besides the members.
pub struct Context<'a> {
    /// The generated file's provenance line.
    pub provenance: &'a str,
    /// Root namespace of the public layer, such as `Gix`.
    pub managed_namespace: &'a str,
    /// Namespace of the interoptopus bindings, such as `GixSharp.Native`.
    pub native_namespace: &'a str,
    /// The hand-written C# runtime class, such as `GixSharp.GixInterop`.
    pub runtime: &'a str,
    /// The facade's service, such as `Repository`.
    pub service: &'a str,
}

/// The emitted public layer.
#[derive(Debug)]
pub struct Layer {
    /// C# source of the layer.
    pub source: String,
    /// Members left out, with the reason, sorted by path.
    pub skipped: Vec<(String, String)>,
}

/// Where a member is rendered, and how it reaches the repository.
#[derive(Clone, Copy)]
enum Host<'a> {
    /// The service class: `this` is the repository.
    Service,
    /// A keyed class: `Repository` is the repository, and the key goes first.
    Keyed(&'a KeyedType),
    /// A static module class, without a repository.
    Module,
}

/// Emits the public C# layer for the generated members.
#[must_use]
pub fn emit(members: &[GeneratedMember], context: &Context<'_>) -> Layer {
    let mut skipped = Vec::new();
    let mut service = Vec::new();
    let mut keyed: BTreeMap<String, (KeyedType, Vec<String>)> = BTreeMap::new();
    let mut modules: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let properties = key_properties(members);
    for member in members {
        let host = match (&member.keyed_owner, member.kind) {
            (Some(owner), _) => Host::Keyed(owner),
            (None, MemberKind::Function) => Host::Module,
            (None, _) => Host::Service,
        };
        match render(member, host, &properties, context) {
            Ok(rendered) => match host {
                Host::Service => service.push(rendered),
                Host::Keyed(owner) => keyed.entry(owner.name.clone()).or_insert_with(|| (owner.clone(), Vec::new())).1.push(rendered),
                Host::Module => modules.entry(location(&member.path, context.managed_namespace)).or_default().push(rendered),
            },
            Err(why) => skipped.push((member.path.clone(), why)),
        }
    }
    for member in members {
        for shape in member.params.iter().map(|(_, shape)| shape).chain(std::iter::once(&member.output)) {
            if let Some(owner) = keyed_in(shape) {
                keyed.entry(owner.name.clone()).or_insert_with(|| (owner.clone(), Vec::new()));
            }
        }
    }
    skipped.sort();
    let modules = without_clashes(modules, &keyed, context);
    Layer { source: assemble(&service, &keyed, &modules, &properties, context), skipped }
}

/// The key property of every keyed type: `key_name` in Pascal case, suffixed with `Key` when it
/// would repeat the class name or one of the class's members.
fn key_properties(members: &[GeneratedMember]) -> BTreeMap<String, String> {
    let mut taken: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut owners: BTreeMap<String, &KeyedType> = BTreeMap::new();
    for member in members {
        if let Some(owner) = &member.keyed_owner {
            taken.entry(owner.name.clone()).or_default().insert(public_name(member, true));
        }
        let shapes = member.params.iter().map(|(_, shape)| shape).chain(std::iter::once(&member.output));
        for owner in member.keyed_owner.iter().chain(shapes.filter_map(keyed_in)) {
            owners.entry(owner.name.clone()).or_insert(owner);
        }
    }
    owners
        .into_iter()
        .map(|(name, owner)| {
            let (_, class) = keyed_location(&owner.name, "");
            let property = rust_to_pascal(&owner.key_name);
            let clashes = property == class || taken.get(&name).is_some_and(|names| names.contains(&property));
            (name, if clashes { format!("{property}Key") } else { property })
        })
        .collect()
}

/// A member's public name: a free function or a keyed type's method is named after its last path
/// segment.
fn public_name(member: &GeneratedMember, keyed: bool) -> String {
    if member.kind == MemberKind::Function || keyed {
        rust_to_pascal(member.path.split('#').next().and_then(|path| path.rsplit("::").next()).unwrap_or(&member.name))
    } else {
        rust_to_pascal(&member.name)
    }
}

/// The key property of a keyed type.
fn property<'p>(properties: &'p BTreeMap<String, String>, owner: &KeyedType) -> &'p str {
    properties.get(&owner.name).map_or("Key", String::as_str)
}

/// The keyed type a shape carries, directly or optionally.
fn keyed_in(shape: &Shape) -> Option<&KeyedType> {
    match shape {
        Shape::Keyed(owner) => Some(owner),
        Shape::Option(inner) => keyed_in(inner),
        _ => None,
    }
}

/// The namespace and static class of a free function: `gix::remote::name::validated` lives in
/// class `Name` of namespace `Gix.Remote`.
fn location(path: &str, managed: &str) -> (String, String) {
    let path = path.split('#').next().unwrap_or(path);
    let segments: Vec<&str> = path.split("::").collect();
    let modules = &segments[1..segments.len().saturating_sub(1)];
    match modules.split_last() {
        None => (managed.to_owned(), "Functions".to_owned()),
        Some((class, outer)) => {
            let namespace = std::iter::once(managed.to_owned()).chain(outer.iter().map(|segment| rust_to_pascal(segment))).collect::<Vec<_>>();
            (namespace.join("."), rust_to_pascal(class))
        }
    }
}

/// The namespace and class of a keyed type: `gix::object::tree::EntryRef` is class `EntryRef` of
/// namespace `Gix.Object.Tree`.
fn keyed_location(name: &str, managed: &str) -> (String, String) {
    let segments: Vec<&str> = name.split("::").collect();
    let (class, outer) = segments.split_last().map_or((name, &[][..]), |(class, outer)| (*class, outer));
    let modules = outer.get(1..).unwrap_or(&[]);
    let namespace = std::iter::once(managed.to_owned()).chain(modules.iter().map(|segment| rust_to_pascal(segment))).collect::<Vec<_>>();
    (namespace.join("."), class.to_owned())
}

/// The fully qualified C# name of a keyed type's class.
fn keyed_reference(owner: &KeyedType, context: &Context<'_>) -> String {
    let (namespace, class) = keyed_location(&owner.name, context.managed_namespace);
    format!("global::{namespace}.{class}")
}

/// Renames module classes that clash with the service class, a keyed class or a namespace.
fn without_clashes(
    modules: BTreeMap<(String, String), Vec<String>>,
    keyed: &BTreeMap<String, (KeyedType, Vec<String>)>,
    context: &Context<'_>,
) -> BTreeMap<(String, String), Vec<String>> {
    let namespaces: BTreeSet<String> = modules.keys().map(|(namespace, _)| namespace.clone()).collect();
    let mut taken: BTreeSet<String> = keyed.values().map(|(owner, _)| keyed_location(&owner.name, context.managed_namespace)).map(|(namespace, class)| format!("{namespace}.{class}")).collect();
    taken.insert(format!("{}.{}", context.managed_namespace, context.service));
    modules
        .into_iter()
        .map(|((namespace, class), members)| {
            let full = format!("{namespace}.{class}");
            let clashes = taken.contains(&full) || namespaces.iter().any(|other| *other == full || other.starts_with(&format!("{full}.")));
            let class = if clashes { format!("{class}Functions") } else { class };
            ((namespace, class), members)
        })
        .collect()
}

/// One member's C# source.
fn render(member: &GeneratedMember, host: Host<'_>, properties: &BTreeMap<String, String>, context: &Context<'_>) -> Result<String, String> {
    let repository = match host {
        Host::Service => Some("this"),
        Host::Keyed(_) => Some("Repository"),
        Host::Module => None,
    };
    let mut declarations = Vec::new();
    let mut setup = Vec::new();
    let mut arguments = Vec::new();
    if let Host::Keyed(owner) = host {
        arguments.push(key_argument(&owner.key, property(properties, owner), "nativeKey", &mut setup)?);
    }
    for (name, shape) in &member.params {
        let public = parameter(name);
        let local = format!("native{}", rust_to_pascal(name));
        match shape {
            Shape::Primitive(primitive) => {
                declarations.push(format!("{} {public}", csharp_primitive(primitive)?));
                arguments.push(public);
            }
            Shape::Text | Shape::Custom(_) | Shape::Bytes | Shape::Path => {
                declarations.push(format!("{} {public}", key_type(shape)?));
                arguments.push(key_argument(shape, &public, &local, &mut setup)?);
            }
            Shape::Service => {
                declarations.push(format!("{} {public}", context.service));
                arguments.push(format!("{public}._native"));
            }
            Shape::Keyed(owner) => {
                let Some(repository) = repository else { return Err("a keyed parameter without a repository".to_owned()) };
                declarations.push(format!("{} {public}", keyed_reference(owner, context)));
                setup.push(format!(
                    "if (!ReferenceEquals({public}.Repository, {repository})) throw new ArgumentException(\"It belongs to another repository.\", nameof({public}));"
                ));
                arguments.push(key_argument(&owner.key, &format!("{public}.{}", property(properties, owner)), &local, &mut setup)?);
            }
            Shape::Option(_) => return Err("optional parameters are not supported by the C# layer yet".to_owned()),
            Shape::Unit => return Err("a unit parameter".to_owned()),
        }
    }
    let arguments = arguments.join(", ");
    let name = public_name(member, matches!(host, Host::Keyed(_)));
    let native_member = service_method_name(context.service, &format!("{}_{}", pascal_to_snake(context.service), member.name));
    let (modifier, returns, call) = match (member.kind, host) {
        (MemberKind::Constructor, _) => {
            let call = format!("new {}(Runtime.Call(\"{}\", () => Native.{}.{native_member}({arguments})))", context.service, member.path, context.service);
            ("static ", context.service.to_owned(), call)
        }
        (MemberKind::Function, _) => {
            let (returns, value) = function_result(&member.output, &format!("Native.Interop.{}({arguments})", member.name))?;
            ("static ", returns, format!("Runtime.Call(\"{}\", () => {value})", member.path))
        }
        (MemberKind::Method, _) => {
            let target = if repository == Some("this") { "_native".to_owned() } else { "Repository._native".to_owned() };
            let (returns, value) = result(&member.output, &format!("{target}.{native_member}({arguments})"), repository, context)?;
            ("", returns, format!("Runtime.Call(\"{}\", () => {value})", member.path))
        }
    };
    let mut source = String::new();
    let _ = writeln!(source, "        /// <summary>Wraps <c>{}</c>.</summary>", member.path.split('#').next().unwrap_or(&member.path));
    let signature = format!("        public {modifier}{returns} {name}({})", declarations.join(", "));
    if setup.is_empty() {
        let _ = writeln!(source, "{signature} => {call};");
    } else {
        let _ = writeln!(source, "{signature}");
        let _ = writeln!(source, "        {{");
        for line in &setup {
            let _ = writeln!(source, "            {line}");
        }
        let keyword = if returns == "void" { "" } else { "return " };
        let _ = writeln!(source, "            {keyword}{call};");
        let _ = writeln!(source, "        }}");
    }
    Ok(source)
}

/// The native argument for a value of a key-like shape, adding the `using` it needs.
fn key_argument(shape: &Shape, value: &str, local: &str, setup: &mut Vec<String>) -> Result<String, String> {
    match shape {
        Shape::Primitive(_) => Ok(value.to_owned()),
        Shape::Text | Shape::Custom(Carrier::String) => {
            setup.push(format!("using var {local} = {value}.Utf8();"));
            Ok(local.to_owned())
        }
        Shape::Bytes | Shape::Custom(Carrier::Bytes) => {
            setup.push(format!("using var {local} = {value}.Slice();"));
            Ok(local.to_owned())
        }
        Shape::Path => {
            setup.push(format!("using var {local} = Runtime.EncodePath({value}).Slice();"));
            Ok(local.to_owned())
        }
        other => Err(format!("a value of shape {other:?}")),
    }
}

/// The C# type of a key-like shape.
fn key_type(shape: &Shape) -> Result<String, String> {
    match shape {
        Shape::Primitive(primitive) => Ok(csharp_primitive(primitive)?.to_owned()),
        Shape::Text | Shape::Path | Shape::Custom(Carrier::String) => Ok("string".to_owned()),
        Shape::Bytes | Shape::Custom(Carrier::Bytes) => Ok("byte[]".to_owned()),
        other => Err(format!("a key of shape {other:?}")),
    }
}

/// The C# type of a method result and the conversion of `call`, a native value the method owns.
fn result(shape: &Shape, call: &str, repository: Option<&str>, context: &Context<'_>) -> Result<(String, String), String> {
    match shape {
        Shape::Unit => Ok(("void".to_owned(), call.to_owned())),
        Shape::Primitive(primitive) if primitive == "bool" => Ok(("bool".to_owned(), format!("(bool){call}"))),
        Shape::Primitive(primitive) => Ok((csharp_primitive(primitive)?.to_owned(), call.to_owned())),
        Shape::Text | Shape::Custom(Carrier::String) => Ok(("string".to_owned(), format!("Runtime.Text({call})"))),
        Shape::Bytes | Shape::Custom(Carrier::Bytes) => Ok(("byte[]".to_owned(), format!("Runtime.Bytes({call})"))),
        Shape::Path => Ok(("string".to_owned(), format!("Runtime.Path({call})"))),
        Shape::Keyed(owner) => {
            let Some(repository) = repository else { return Err("a keyed result without a repository".to_owned()) };
            let (_, key) = result(&owner.key, call, None, context)?;
            let class = keyed_reference(owner, context);
            Ok((class.clone(), format!("new {class}({repository}, {key})")))
        }
        Shape::Option(inner) => match inner.as_ref() {
            Shape::Primitive(primitive) if primitive == "bool" => Ok(("bool?".to_owned(), format!("Runtime.Flag({call})"))),
            Shape::Text | Shape::Custom(Carrier::String) => Ok(("string?".to_owned(), format!("Runtime.Text({call})"))),
            Shape::Bytes | Shape::Custom(Carrier::Bytes) => Ok(("byte[]?".to_owned(), format!("Runtime.Bytes({call})"))),
            Shape::Path => Ok(("string?".to_owned(), format!("Runtime.Path({call})"))),
            Shape::Keyed(owner) => {
                let Some(repository) = repository else { return Err("a keyed result without a repository".to_owned()) };
                let (_, key) = result(&Shape::Option(owner.key.clone()), call, None, context)?;
                let class = keyed_reference(owner, context);
                Ok((format!("{class}?"), format!("{key} is {{ }} nativeResultKey ? new {class}({repository}, nativeResultKey) : null")))
            }
            other => Err(format!("an optional {other:?} result is not supported by the C# layer yet")),
        },
        Shape::Service => Err("a method returning the service".to_owned()),
    }
}

/// The C# type of a free function's result and the conversion of `call`, a native result union.
fn function_result(shape: &Shape, call: &str) -> Result<(String, String), String> {
    match shape {
        Shape::Unit => Ok(("void".to_owned(), format!("Runtime.Done({call})"))),
        Shape::Primitive(primitive) if primitive == "bool" => Ok(("bool".to_owned(), format!("Runtime.Flag({call})"))),
        Shape::Primitive(primitive) => Ok((csharp_primitive(primitive)?.to_owned(), format!("Runtime.Value({call})"))),
        Shape::Text | Shape::Custom(Carrier::String) => Ok(("string".to_owned(), format!("Runtime.Text({call})"))),
        Shape::Bytes | Shape::Custom(Carrier::Bytes) => Ok(("byte[]".to_owned(), format!("Runtime.Bytes({call})"))),
        Shape::Path => Ok(("string".to_owned(), format!("Runtime.Path({call})"))),
        other => Err(format!("a free function returning {other:?} is not supported by the C# layer yet")),
    }
}

/// The C# type of a Rust primitive.
fn csharp_primitive(primitive: &str) -> Result<&'static str, String> {
    Ok(match primitive {
        "bool" => "bool",
        "u8" => "byte",
        "u16" => "ushort",
        "u32" => "uint",
        "u64" => "ulong",
        "usize" => "nuint",
        "i8" => "sbyte",
        "i16" => "short",
        "i32" => "int",
        "i64" => "long",
        "isize" => "nint",
        "f32" => "float",
        "f64" => "double",
        other => return Err(format!("the primitive `{other}`")),
    })
}

/// A C# parameter name: `rela_path` becomes `relaPath`; reserved words get `@`.
fn parameter(name: &str) -> String {
    let base = name.trim_end_matches('_');
    let pascal = rust_to_pascal(base);
    let mut chars = pascal.chars();
    let camel = chars.next().map(|first| first.to_lowercase().chain(chars).collect::<String>()).unwrap_or_default();
    if is_forbidden(&camel) { format!("@{camel}") } else { camel }
}

/// The whole C# file.
fn assemble(
    service: &[String],
    keyed: &BTreeMap<String, (KeyedType, Vec<String>)>,
    modules: &BTreeMap<(String, String), Vec<String>>,
    properties: &BTreeMap<String, String>,
    context: &Context<'_>,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "// <auto-generated>");
    let _ = writeln!(out, "// {}", context.provenance);
    let _ = writeln!(out, "// </auto-generated>");
    let _ = writeln!(out, "#nullable enable");
    let _ = writeln!(out);
    let _ = writeln!(out, "using System;");
    let _ = writeln!(out, "using {};", context.native_namespace);
    let _ = writeln!(out, "using Native = {};", context.native_namespace);
    let _ = writeln!(out, "using Runtime = {};", context.runtime);
    if !service.is_empty() || !keyed.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "namespace {}", context.managed_namespace);
        let _ = writeln!(out, "{{");
        let _ = writeln!(out, "    /// <summary>The repository handle.</summary>");
        let _ = writeln!(out, "    public sealed class {} : IDisposable", context.service);
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        internal readonly Native.{} _native;", context.service);
        let _ = writeln!(out);
        let _ = writeln!(out, "        private {}(Native.{} native) => _native = native;", context.service, context.service);
        for member in service {
            let _ = writeln!(out);
            out.push_str(member);
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "        /// <summary>Releases the native handle.</summary>");
        let _ = writeln!(out, "        public void Dispose() => _native.Dispose();");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "}}");
    }
    for (owner, members) in keyed.values() {
        let (namespace, class) = keyed_location(&owner.name, context.managed_namespace);
        let property = property(properties, owner);
        let key_type = key_type(&owner.key).unwrap_or_else(|_| "string".to_owned());
        let key_parameter = parameter(&owner.key_name);
        let repository = format!("global::{}.{}", context.managed_namespace, context.service);
        let _ = writeln!(out);
        let _ = writeln!(out, "namespace {namespace}");
        let _ = writeln!(out, "{{");
        let _ = writeln!(out, "    /// <summary>A <c>{}</c>, rebuilt from its repository and key on every call.</summary>", owner.name);
        let _ = writeln!(out, "    public sealed class {class}");
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        internal {class}({repository} repository, {key_type} {key_parameter})");
        let _ = writeln!(out, "        {{");
        let _ = writeln!(out, "            Repository = repository;");
        let _ = writeln!(out, "            {property} = {key_parameter};");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out);
        let _ = writeln!(out, "        /// <summary>The repository it belongs to.</summary>");
        let _ = writeln!(out, "        public {repository} Repository {{ get; }}");
        let _ = writeln!(out);
        let _ = writeln!(out, "        /// <summary>Its key.</summary>");
        let _ = writeln!(out, "        public {key_type} {property} {{ get; }}");
        for member in members {
            let _ = writeln!(out);
            out.push_str(member);
        }
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "}}");
    }
    for ((namespace, class), members) in modules {
        let _ = writeln!(out);
        let _ = writeln!(out, "namespace {namespace}");
        let _ = writeln!(out, "{{");
        let _ = writeln!(out, "    /// <summary>Free functions of one Rust module.</summary>");
        let _ = writeln!(out, "    public static class {class}");
        let _ = writeln!(out, "    {{");
        for (index, member) in members.iter().enumerate() {
            if index > 0 {
                let _ = writeln!(out);
            }
            out.push_str(member);
        }
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "}}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Context<'static> {
        Context { provenance: "test", managed_namespace: "Demo", native_namespace: "Demo.Native", runtime: "Demo.Interop", service: "Repo" }
    }

    fn member(path: &str, kind: MemberKind, name: &str, params: Vec<(&str, Shape)>, output: Shape) -> GeneratedMember {
        GeneratedMember {
            path: path.to_owned(),
            kind,
            name: name.to_owned(),
            params: params.into_iter().map(|(name, shape)| (name.to_owned(), shape)).collect(),
            output,
            keyed_owner: None,
        }
    }

    fn commit() -> KeyedType {
        KeyedType { name: "demo::Commit".to_owned(), key_name: "id".to_owned(), key: Box::new(Shape::Custom(Carrier::String)) }
    }

    #[test]
    fn constructors_become_factories_and_methods_convert_results() {
        let members = [
            member("demo::open", MemberKind::Constructor, "open", vec![("directory", Shape::Path)], Shape::Service),
            member("demo::Repo::is_bare", MemberKind::Method, "is_bare", Vec::new(), Shape::Primitive("bool".to_owned())),
            member("demo::Repo::workdir", MemberKind::Method, "workdir", Vec::new(), Shape::Option(Box::new(Shape::Path))),
            member("demo::init", MemberKind::Constructor, "init_", vec![("directory", Shape::Path)], Shape::Service),
        ];
        let layer = emit(&members, &context());
        let source = layer.source;
        assert!(source.contains("    public sealed class Repo : IDisposable\n"), "{source}");
        assert!(source.contains("        public static Repo Open(string directory)\n        {\n            using var nativeDirectory = Runtime.EncodePath(directory).Slice();\n            return new Repo(Runtime.Call(\"demo::open\", () => Native.Repo.Open(nativeDirectory)));\n"), "{source}");
        assert!(source.contains("        public bool IsBare() => Runtime.Call(\"demo::Repo::is_bare\", () => (bool)_native.IsBare());\n"), "{source}");
        assert!(source.contains("        public string? Workdir() => Runtime.Call(\"demo::Repo::workdir\", () => Runtime.Path(_native.Workdir()));\n"), "{source}");
        assert!(source.contains("Native.Repo.Init(nativeDirectory)"), "escaped Rust names map to the backend's names: {source}");
        assert!(source.contains("        public static Repo Init(string directory)\n"), "{source}");
        assert!(layer.skipped.is_empty());
    }

    #[test]
    fn free_functions_land_in_module_classes() {
        let members = [
            member("demo::env::agent", MemberKind::Function, "demo_env_agent", Vec::new(), Shape::Text),
            member("demo::remote::name::validated", MemberKind::Function, "demo_remote_name_validated", vec![("name", Shape::Bytes)], Shape::Bytes),
            member("demo::interrupt::reset", MemberKind::Function, "demo_interrupt_reset", Vec::new(), Shape::Unit),
        ];
        let source = emit(&members, &context()).source;
        assert!(source.contains("namespace Demo\n{\n    /// <summary>Free functions of one Rust module.</summary>\n    public static class Env\n"), "{source}");
        assert!(source.contains("        public static string Agent() => Runtime.Call(\"demo::env::agent\", () => Runtime.Text(Native.Interop.demo_env_agent()));\n"), "{source}");
        assert!(source.contains("namespace Demo.Remote\n"), "{source}");
        assert!(source.contains("            using var nativeName = name.Slice();\n            return Runtime.Call(\"demo::remote::name::validated\", () => Runtime.Bytes(Native.Interop.demo_remote_name_validated(nativeName)));\n"), "{source}");
        assert!(source.contains("        public static void Reset() => Runtime.Call(\"demo::interrupt::reset\", () => Runtime.Done(Native.Interop.demo_interrupt_reset()));\n"), "{source}");
        assert!(!source.contains("class Repo"), "no service without service members: {source}");
    }

    #[test]
    fn service_references_parameters_and_skips() {
        let members = [
            member("demo::Repo::main", MemberKind::Constructor, "main", vec![("repo", Shape::Service)], Shape::Service),
            member("demo::Repo::lock", MemberKind::Method, "lock_", vec![("reason", Shape::Option(Box::new(Shape::Bytes)))], Shape::Unit),
            member("demo::Repo::path_of", MemberKind::Method, "path_of", vec![("rela_path", Shape::Bytes), ("count", Shape::Primitive("usize".to_owned()))], Shape::Path),
        ];
        let layer = emit(&members, &context());
        assert!(layer.source.contains("Native.Repo.Main(repo._native)"), "{}", layer.source);
        assert!(layer.source.contains("        public string PathOf(byte[] relaPath, nuint count)\n"), "{}", layer.source);
        assert_eq!(layer.skipped, [("demo::Repo::lock".to_owned(), "optional parameters are not supported by the C# layer yet".to_owned())]);
    }

    #[test]
    fn module_classes_never_clash_with_the_service_or_a_namespace() {
        let members = [
            member("demo::Repo::is_bare", MemberKind::Method, "is_bare", Vec::new(), Shape::Primitive("bool".to_owned())),
            member("demo::repo::find", MemberKind::Function, "demo_repo_find", Vec::new(), Shape::Text),
            member("demo::remote::list", MemberKind::Function, "demo_remote_list", Vec::new(), Shape::Text),
            member("demo::remote::name::validated", MemberKind::Function, "demo_remote_name_validated", Vec::new(), Shape::Text),
        ];
        let source = emit(&members, &context()).source;
        assert!(source.contains("public static class RepoFunctions"), "{source}");
        assert!(source.contains("public static class RemoteFunctions"), "{source}");
        assert!(source.contains("namespace Demo.Remote\n"), "{source}");
    }

    #[test]
    fn keyed_types_become_classes_holding_repository_and_key() {
        let mut message = member("demo::Commit::message", MemberKind::Method, "commit_message", Vec::new(), Shape::Bytes);
        message.keyed_owner = Some(commit());
        let mut parent = member("demo::Commit::parent", MemberKind::Method, "commit_parent", Vec::new(), Shape::Option(Box::new(Shape::Keyed(commit()))));
        parent.keyed_owner = Some(commit());
        let head = member("demo::Repo::head_commit", MemberKind::Method, "head_commit", Vec::new(), Shape::Keyed(commit()));
        let describe = member("demo::Repo::describe", MemberKind::Method, "describe", vec![("commit", Shape::Keyed(commit()))], Shape::Text);
        let source = emit(&[message, parent, head, describe], &context()).source;
        assert!(source.contains("    public sealed class Commit\n    {\n        internal Commit(global::Demo.Repo repository, string id)\n"), "{source}");
        assert!(source.contains("        public string Id { get; }\n"), "{source}");
        assert!(source.contains("        public byte[] Message()\n        {\n            using var nativeKey = Id.Utf8();\n            return Runtime.Call(\"demo::Commit::message\", () => Runtime.Bytes(Repository._native.CommitMessage(nativeKey)));\n"), "{source}");
        assert!(source.contains("() => Runtime.Text(Repository._native.CommitParent(nativeKey)) is { } nativeResultKey ? new global::Demo.Commit(Repository, nativeResultKey) : null"), "{source}");
        assert!(source.contains("        public global::Demo.Commit HeadCommit() => Runtime.Call(\"demo::Repo::head_commit\", () => new global::Demo.Commit(this, Runtime.Text(_native.HeadCommit())));\n"), "{source}");
        assert!(source.contains("            if (!ReferenceEquals(commit.Repository, this)) throw new ArgumentException(\"It belongs to another repository.\", nameof(commit));\n            using var nativeCommit = commit.Id.Utf8();\n"), "{source}");
        assert!(source.contains("        internal readonly Native.Repo _native;\n"), "{source}");
    }

    #[test]
    fn key_properties_never_repeat_their_class_or_members() {
        let id = KeyedType { name: "demo::Id".to_owned(), key_name: "id".to_owned(), key: Box::new(Shape::Custom(Carrier::String)) };
        let mut commit_id = member("demo::Commit::id", MemberKind::Method, "commit_id", Vec::new(), Shape::Keyed(id.clone()));
        commit_id.keyed_owner = Some(commit());
        let properties = key_properties(&[commit_id]);
        assert_eq!(properties.get("demo::Id").map(String::as_str), Some("IdKey"), "repeats the class");
        assert_eq!(properties.get("demo::Commit").map(String::as_str), Some("IdKey"), "repeats the method `Id`");
        let mut message = member("demo::Commit::message", MemberKind::Method, "commit_message", Vec::new(), Shape::Bytes);
        message.keyed_owner = Some(commit());
        assert_eq!(key_properties(&[message]).get("demo::Commit").map(String::as_str), Some("Id"));
        assert_eq!(keyed_location("demo::object::tree::EntryRef", "Demo"), ("Demo.Object.Tree".to_owned(), "EntryRef".to_owned()));
    }

    #[test]
    fn parameters_follow_csharp_conventions() {
        assert_eq!(parameter("rela_path"), "relaPath");
        assert_eq!(parameter("value_"), "@value");
        assert_eq!(parameter("id"), "id");
    }
}