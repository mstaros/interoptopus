//! Emits the public C# layer (design step 3, "generate"; todo E7) over the bindings interoptopus
//! generates for the facade.
//!
//! The service becomes a sealed, disposable class in the managed namespace: its constructors become
//! static factories and its `&self` methods become methods. Free functions become static methods of
//! a static class named after their module, so `gix::env::agent` becomes `Gix.Env.Agent()`. Native
//! member names come from `interoptopus_backends::casing`, the functions the C# backend itself
//! uses. Conversions and error translation go through the hand-written C# runtime class
//! (configuration `csharp_runtime`), which provides:
//!
//! - `Call(operation, func)` and `Call(operation, action)`, translating native errors;
//! - `EncodePath(string) -> byte[]`;
//! - `Text`, `Bytes`, `Path` and `Flag` over owned native values and options, and `Text`, `Bytes`,
//!   `Flag`, `Value` and `Done` over free-function results (`IResult<T, TError>`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use interoptopus_backends::casing::{pascal_to_snake, rust_to_pascal, service_method_name};

use crate::classify::Carrier;
use crate::facade::{GeneratedMember, MemberKind, Shape};
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

/// Emits the public C# layer for the generated members.
#[must_use]
pub fn emit(members: &[GeneratedMember], context: &Context<'_>) -> Layer {
    let mut skipped = Vec::new();
    let mut service = Vec::new();
    let mut modules: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for member in members {
        match render(member, context) {
            Ok(rendered) if member.kind == MemberKind::Function => {
                modules.entry(location(&member.path, context.managed_namespace)).or_default().push(rendered);
            }
            Ok(rendered) => service.push(rendered),
            Err(why) => skipped.push((member.path.clone(), why)),
        }
    }
    skipped.sort();
    let modules = without_clashes(modules, context);
    Layer { source: assemble(&service, &modules, context), skipped }
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

/// Renames module classes that clash with the service class or with a namespace of the same name.
fn without_clashes(modules: BTreeMap<(String, String), Vec<String>>, context: &Context<'_>) -> BTreeMap<(String, String), Vec<String>> {
    let namespaces: BTreeSet<String> = modules.keys().map(|(namespace, _)| namespace.clone()).collect();
    let service = format!("{}.{}", context.managed_namespace, context.service);
    modules
        .into_iter()
        .map(|((namespace, class), members)| {
            let full = format!("{namespace}.{class}");
            let clashes = full == service || namespaces.iter().any(|other| *other == full || other.starts_with(&format!("{full}.")));
            let class = if clashes { format!("{class}Functions") } else { class };
            ((namespace, class), members)
        })
        .collect()
}

/// One member's C# source.
fn render(member: &GeneratedMember, context: &Context<'_>) -> Result<String, String> {
    let mut declarations = Vec::new();
    let mut setup = Vec::new();
    let mut arguments = Vec::new();
    for (name, shape) in &member.params {
        let public = parameter(name);
        let local = format!("native{}", rust_to_pascal(name));
        match shape {
            Shape::Primitive(primitive) => {
                declarations.push(format!("{} {public}", csharp_primitive(primitive)?));
                arguments.push(public);
            }
            Shape::Text | Shape::Custom(Carrier::String) => {
                declarations.push(format!("string {public}"));
                setup.push(format!("using var {local} = {public}.Utf8();"));
                arguments.push(local);
            }
            Shape::Bytes | Shape::Custom(Carrier::Bytes) => {
                declarations.push(format!("byte[] {public}"));
                setup.push(format!("using var {local} = {public}.Slice();"));
                arguments.push(local);
            }
            Shape::Path => {
                declarations.push(format!("string {public}"));
                setup.push(format!("using var {local} = Runtime.EncodePath({public}).Slice();"));
                arguments.push(local);
            }
            Shape::Service => {
                declarations.push(format!("{} {public}", context.service));
                arguments.push(format!("{public}._native"));
            }
            Shape::Option(_) => return Err("optional parameters are not supported by the C# layer yet".to_owned()),
            Shape::Unit => return Err("a unit parameter".to_owned()),
        }
    }
    let arguments = arguments.join(", ");
    // A free function is named after its last path segment; its class carries the module.
    let name = match member.kind {
        MemberKind::Function => rust_to_pascal(member.path.split('#').next().and_then(|path| path.rsplit("::").next()).unwrap_or(&member.name)),
        MemberKind::Constructor | MemberKind::Method => rust_to_pascal(&member.name),
    };
    let native_member = service_method_name(context.service, &format!("{}_{}", pascal_to_snake(context.service), member.name));
    let (modifier, returns, call) = match member.kind {
        MemberKind::Constructor => {
            let call = format!("new {}(Runtime.Call(\"{}\", () => Native.{}.{native_member}({arguments})))", context.service, member.path, context.service);
            ("static ", context.service.to_owned(), call)
        }
        MemberKind::Method => {
            let (returns, value) = result(&member.output, &format!("_native.{native_member}({arguments})"))?;
            ("", returns, format!("Runtime.Call(\"{}\", () => {value})", member.path))
        }
        MemberKind::Function => {
            let (returns, value) = function_result(&member.output, &format!("Native.Interop.{}({arguments})", member.name))?;
            ("static ", returns, format!("Runtime.Call(\"{}\", () => {value})", member.path))
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

/// The C# type of a method result and the conversion of `call`, a native value the method owns.
fn result(shape: &Shape, call: &str) -> Result<(String, String), String> {
    match shape {
        Shape::Unit => Ok(("void".to_owned(), call.to_owned())),
        Shape::Primitive(primitive) if primitive == "bool" => Ok(("bool".to_owned(), format!("(bool){call}"))),
        Shape::Primitive(primitive) => Ok((csharp_primitive(primitive)?.to_owned(), call.to_owned())),
        Shape::Text | Shape::Custom(Carrier::String) => Ok(("string".to_owned(), format!("Runtime.Text({call})"))),
        Shape::Bytes | Shape::Custom(Carrier::Bytes) => Ok(("byte[]".to_owned(), format!("Runtime.Bytes({call})"))),
        Shape::Path => Ok(("string".to_owned(), format!("Runtime.Path({call})"))),
        Shape::Option(inner) => match inner.as_ref() {
            Shape::Primitive(primitive) if primitive == "bool" => Ok(("bool?".to_owned(), format!("Runtime.Flag({call})"))),
            Shape::Text | Shape::Custom(Carrier::String) => Ok(("string?".to_owned(), format!("Runtime.Text({call})"))),
            Shape::Bytes | Shape::Custom(Carrier::Bytes) => Ok(("byte[]?".to_owned(), format!("Runtime.Bytes({call})"))),
            Shape::Path => Ok(("string?".to_owned(), format!("Runtime.Path({call})"))),
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
fn assemble(service: &[String], modules: &BTreeMap<(String, String), Vec<String>>, context: &Context<'_>) -> String {
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
    if !service.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "namespace {}", context.managed_namespace);
        let _ = writeln!(out, "{{");
        let _ = writeln!(out, "    /// <summary>The repository handle.</summary>");
        let _ = writeln!(out, "    public sealed class {} : IDisposable", context.service);
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        private readonly Native.{} _native;", context.service);
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
        }
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
    fn parameters_follow_csharp_conventions() {
        assert_eq!(parameter("rela_path"), "relaPath");
        assert_eq!(parameter("value_"), "@value");
        assert_eq!(parameter("id"), "id");
    }
}