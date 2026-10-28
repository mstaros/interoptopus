//! Emits the Rust facade (design step 3, "generate"; todo E5) for the ready items of a manifest.
//!
//! Types mapped to `handle` become one interoptopus service, named after the shortest of
//! them. Functions that return the handle become its constructors (a `&self` method that does
//! becomes a constructor taking the service by reference), `&self` methods become its methods, and everything else
//! becomes a free function named after its module path. On a name clash a free function wins
//! over a method of the service's own type, which wins over the other handle types; methods of
//! the other handle types that only convert between handle types are dropped.
//!
//! A keyed type crosses as its key: its members become service methods named `{type}_{method}`
//! that take the key first and rebuild the value from the handle and the key (`rebuild`); keyed
//! parameters are rebuilt the same way, and keyed results return their key (`key_of`). Free
//! functions and constructors taking a keyed handle are dropped, since they have no handle to
//! rebuild from.
//!
//! Every member returns `ffi::Result<T, runtime::Error>`. Text and bytes convert inline; the
//! rest goes through the hand-written runtime module (configuration `runtime_module`, default
//! `crate::runtime`), which provides:
//!
//! - `Error`, an FFI type, and `error(e) -> Error` for every error a wrapped call returns;
//! - the handle holder, named like the service, with `From` for each type mapped to `handle`
//!   and one accessor per such type, named after it in snake case (for gix `repository()` and
//!   `thread_safe_repository()`), whose result dereferences to that type;
//! - `path(&ffi::Slice<u8>) -> Result<PathBuf, Error>` and `path_out(&Path) -> ffi::Vec<u8>`;
//! - for each custom mapping, its `into_rust(&ffi value) -> Result<T, Error>` and
//!   `into_ffi(&T) -> ffi value` functions.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rustdoc_types::{GenericBound, Type};

use crate::classify::{Carrier, Classifier, Conversion, KEY_PLACEHOLDER, Keyed, Mapping, REPO_PLACEHOLDER, VALUE_PLACEHOLDER};
use crate::forbidden::escaped;
use crate::render::{Renderer, last_segment, type_args};
use crate::surface::{Function, Receiver};

/// Primitives that cross the boundary unchanged.
const PRIMITIVES: [&str; 13] = ["bool", "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64"];

/// What the emitter needs besides the functions.
pub struct Context<'a> {
    /// Renders canonical paths.
    pub renderer: &'a Renderer,
    /// Supplies the type map and the conversion table.
    pub classifier: &'a Classifier<'a>,
    /// The runtime module path, such as `crate::runtime`.
    pub runtime: &'a str,
    /// The generated file's first line, recording its provenance.
    pub provenance: &'a str,
}

/// The emitted facade module.
#[derive(Debug)]
pub struct Facade {
    /// Rust source of the module.
    pub source: String,
    /// Paths of the emitted items, sorted.
    pub emitted: Vec<String>,
    /// Paths left out, with the reason, sorted.
    pub dropped: Vec<(String, String)>,
    /// The emitted members, as the C# layer needs them, in emission order.
    pub members: Vec<GeneratedMember>,
    /// The service's name.
    pub service: String,
}

/// What crosses the boundary for one value, as the C# layer converts it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Nothing.
    Unit,
    /// A primitive, by its Rust name.
    Primitive(String),
    /// Text.
    Text,
    /// Bytes.
    Bytes,
    /// A filesystem path.
    Path,
    /// A custom mapping, by its carrier.
    Custom(Carrier),
    /// An optional value.
    Option(Box<Self>),
    /// The service: a constructor's result, or the service a constructor takes.
    Service,
    /// A keyed handle, crossing as its key.
    Keyed(KeyedType),
}

/// A keyed handle type, as the C# layer needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyedType {
    /// The type's path as the type map writes it, such as `gix::Commit`.
    pub name: String,
    /// The key's name in the public layer.
    pub key_name: String,
    /// The key's shape.
    pub key: Box<Shape>,
}

/// What a generated member is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    /// A constructor of the service.
    Constructor,
    /// A `&self` method of the service.
    Method,
    /// A free function.
    Function,
}

/// One emitted member, as the C# layer needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedMember {
    /// The surface path it wraps.
    pub path: String,
    /// Constructor, method or free function.
    pub kind: MemberKind,
    /// The member's name in the facade.
    pub name: String,
    /// The parameters after `self`; a constructor taking the service lists it first.
    pub params: Vec<(String, Shape)>,
    /// The result.
    pub output: Shape,
    /// The keyed type a method belongs to; its key is the first native argument.
    pub keyed_owner: Option<KeyedType>,
}

/// Where a member goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Place {
    /// A constructor or method of the service.
    Service,
    /// A free function.
    Free,
}

/// One generated member before name clashes are resolved.
struct Member {
    place: Place,
    name: String,
    /// Lower wins a name clash.
    rank: u8,
    path: String,
    source: String,
    generated: GeneratedMember,
}

/// A function's fate.
enum Planned {
    Member(Box<Member>),
    Dropped(String),
}

/// How an FFI argument becomes the Rust argument.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Conv {
    /// Unchanged.
    Same,
    /// A reference to the value.
    Ref,
    /// `&str` from text.
    Str,
    /// `String` from text.
    Text,
    /// `&[u8]` from bytes.
    Bytes,
    /// A byte-string type converted from `&[u8]` with `Into`.
    BytesInto,
    /// `Vec<u8>`, which also converts into owned byte strings.
    BytesOwned,
    /// A path through the runtime; `borrowed` passes `&Path`.
    Path { borrowed: bool },
    /// A custom mapping through the runtime function it names; `borrowed` passes a reference.
    Custom { function: String, borrowed: bool },
    /// An optional value.
    Option(Box<Self>),
    /// A keyed handle rebuilt from its key: `key` converts the FFI key, `accessor` names the
    /// holder accessor of the handle it is rebuilt from; `borrowed` passes a reference.
    Keyed { keyed: Box<Keyed>, key: Box<Self>, accessor: String, borrowed: bool },
}

/// A lowered parameter: the FFI type and how it converts.
struct Input {
    ffi: String,
    conv: Conv,
}

impl Input {
    fn new(ffi: &str, conv: Conv) -> Self {
        Self { ffi: ffi.to_owned(), conv }
    }
}

/// Emits the facade module for `functions`, all of which must be ready.
///
/// # Errors
///
/// The path of a function the emitter cannot lower, with the reason.
pub fn emit(functions: &[&Function], context: &Context<'_>) -> Result<Facade, (String, String)> {
    let emitter = Emitter { context, service: service_name(context.classifier) };
    let mut kept: BTreeMap<(Place, String), Member> = BTreeMap::new();
    let mut dropped = Vec::new();
    for function in functions {
        match emitter.member(function).map_err(|why| (function.path.clone(), why))? {
            Planned::Member(member) => {
                let member = *member;
                let key = (member.place, member.name.clone());
                match kept.get(&key) {
                    Some(existing) if existing.rank <= member.rank => {
                        dropped.push((member.path, format!("duplicate of `{}`", existing.path)));
                    }
                    _ => {
                        let winner = member.path.clone();
                        if let Some(replaced) = kept.insert(key, member) {
                            dropped.push((replaced.path, format!("duplicate of `{winner}`")));
                        }
                    }
                }
            }
            Planned::Dropped(reason) => dropped.push((function.path.clone(), reason)),
        }
    }
    dropped.sort();
    let mut paths: Vec<String> = kept.values().map(|member| member.path.clone()).collect();
    paths.sort();
    let members: Vec<&Member> = kept.values().collect();
    let generated = members.iter().map(|member| member.generated.clone()).collect();
    Ok(Facade { source: emitter.module(&members), emitted: paths, dropped, members: generated, service: emitter.service })
}

/// The service is named after the shortest type mapped to the repository handle.
fn service_name(classifier: &Classifier<'_>) -> String {
    classifier
        .handles()
        .map(last_segment)
        .min_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)))
        .unwrap_or("Repository")
        .to_owned()
}

struct Emitter<'a> {
    context: &'a Context<'a>,
    service: String,
}

impl Emitter<'_> {
    fn mapping(&self, canonical: &str) -> Option<Mapping> {
        self.context.classifier.mapping(canonical)
    }

    fn rendered(&self, ty: &Type) -> String {
        self.context.renderer.ty(ty)
    }

    /// The owning handle type, if the function belongs to one.
    fn handle_owner<'f>(&self, function: &'f Function) -> Option<&'f str> {
        [&function.owner, &function.owner_public]
            .into_iter()
            .flatten()
            .find(|path| self.mapping(path) == Some(Mapping::Handle))
            .map(String::as_str)
    }

    /// `Some(fallible)` when `ty` is the handle, or a `Result` of it.
    fn handle_result(&self, function: &Function, ty: &Type) -> Option<bool> {
        match ty {
            Type::Generic(name) if name == "Self" => self.handle_owner(function).map(|_| false),
            Type::ResolvedPath(path) => {
                let canonical = self.context.renderer.path(path);
                match (canonical.as_str(), type_args(path).as_slice()) {
                    ("core::result::Result", [ok, _]) => self.handle_result(function, ok).filter(|fallible| !fallible).map(|_| true),
                    _ => (self.mapping(&canonical) == Some(Mapping::Handle)).then_some(false),
                }
            }
            _ => None,
        }
    }

    fn member(&self, function: &Function) -> Result<Planned, String> {
        let owner = self.handle_owner(function);
        // Associated functions of a keyed type have no key to rebuild from; they are free functions.
        let keyed_owner = self.keyed_owner(function).filter(|_| function.receiver != Receiver::None);
        let rank = match (owner, &keyed_owner) {
            (_, Some(_)) => 3,
            (None, None) => 0,
            (Some(owner), None) if last_segment(owner) == self.service => 1,
            (Some(_), None) => 2,
        };
        let constructor = function.output.as_ref().and_then(|output| self.handle_result(function, output));
        let method = function.receiver == Receiver::Shared || (keyed_owner.is_some() && function.receiver == Receiver::Value);
        if method && constructor.is_some() && rank == 2 {
            return Ok(Planned::Dropped("converts between handle types".to_owned()));
        }
        if keyed_owner.is_some() && constructor.is_some() {
            return Ok(Planned::Dropped("returns the handle it is rebuilt from".to_owned()));
        }
        if !method && function.receiver != Receiver::None {
            return Err(format!("receiver {:?}", function.receiver));
        }
        let accessor = owner.map(|owner| snake(last_segment(owner)));
        let handle_parameter = snake(&self.service);
        let mut declarations = Vec::new();
        let mut setup = Vec::new();
        let mut arguments = Vec::new();
        let mut shapes = Vec::new();
        for (index, (name, ty)) in function.params.iter().enumerate() {
            let name = parameter_name(name, index);
            let input = self.input(function, ty).map_err(|why| format!("parameter `{name}`: {why}"))?;
            shapes.push((name.clone(), conv_shape(&input.conv, &input.ffi)));
            declarations.push(format!("{name}: {}", input.ffi));
            arguments.push(argument(&name, &input.conv, &mut setup));
        }
        if shapes.iter().any(|(_, shape)| matches!(shape, Shape::Keyed(_))) && (!method || constructor.is_some()) {
            return Ok(Planned::Dropped("takes a keyed handle, which needs the handle to rebuild from".to_owned()));
        }
        let member_name = keyed_owner.as_ref().map_or_else(|| function.name.clone(), |(path, _)| format!("{}_{}", snake(last_segment(path)), function.name));
        let call = if let Some((path, keyed)) = &keyed_owner {
            let parameter = format!("{}_key", snake(last_segment(path)));
            let key = self.key_input(keyed).map_err(|why| format!("receiver: {why}"))?;
            declarations.insert(0, format!("{parameter}: {}", key.ffi));
            let conv = Conv::Keyed { keyed: Box::new(keyed.clone()), key: Box::new(key.conv), accessor: snake(last_segment(&keyed.repo)), borrowed: false };
            let receiver = argument(&parameter, &conv, &mut setup);
            format!("{receiver}.{}({})", function.name, arguments.join(", "))
        } else {
            match (method, &accessor) {
            (true, Some(accessor)) if constructor.is_some() => {
                // interoptopus copies constructor parameters into a free function, where `Self` is unknown.
                declarations.insert(0, format!("{handle_parameter}: &{}", self.service));
                shapes.insert(0, (handle_parameter.clone(), Shape::Service));
                format!("{handle_parameter}.inner.{accessor}().{}({})", function.name, arguments.join(", "))
            }
            (true, Some(accessor)) => format!("self.inner.{accessor}().{}({})", function.name, arguments.join(", ")),
            (true, None) => return Err("a method of a type that is not a handle".to_owned()),
            (false, _) => format!("{}({})", item_path(&function.path), arguments.join(", ")),
            }
        };
        let (place, name, receiver, returns, value) = if let Some(fallible) = constructor {
            let value = if fallible { format!("{call}.map_err(runtime::error)?") } else { call };
            let holder = format!("Self {{ inner: runtime::{}::from({value}) }}", self.service);
            (Place::Service, escaped(&function.name), "", "Self".to_owned(), holder)
        } else {
            let unit = Type::Tuple(Vec::new());
            let (returns, value) = self.output(function.output.as_ref().unwrap_or(&unit), &call)?;
            if method {
                (Place::Service, escaped(&member_name), "&self", returns, value)
            } else {
                (Place::Free, escaped(&free_name(&function.path)), "", returns, value)
            }
        };
        let output_type = Type::Tuple(Vec::new());
        let (kind, output) = if constructor.is_some() {
            (MemberKind::Constructor, Shape::Service)
        } else {
            let kind = if method { MemberKind::Method } else { MemberKind::Function };
            (kind, self.shape(function.output.as_ref().unwrap_or(&output_type)))
        };
        let keyed_owner = keyed_owner.map(|(_, keyed)| self.keyed_type(&keyed));
        let generated = GeneratedMember { path: function.path.clone(), kind, name: name.clone(), params: shapes, output, keyed_owner };
        let mut parameters = declarations;
        if !receiver.is_empty() {
            parameters.insert(0, receiver.to_owned());
        }
        let indent = if place == Place::Service { "    " } else { "" };
        let mut source = String::new();
        let _ = writeln!(source, "{indent}/// Wraps `{}`.", item_path(&function.path));
        if place == Place::Free {
            let _ = writeln!(source, "#[ffi]");
        }
        let _ = writeln!(source, "{indent}pub fn {name}({}) -> ffi::Result<{returns}, runtime::Error> {{", parameters.join(", "));
        if setup.is_empty() {
            let _ = writeln!(source, "{indent}    finish(|| Ok({value}))");
        } else {
            let _ = writeln!(source, "{indent}    finish(|| {{");
            for line in &setup {
                let _ = writeln!(source, "{indent}        {line}");
            }
            let _ = writeln!(source, "{indent}        Ok({value})");
            let _ = writeln!(source, "{indent}    }})");
        }
        let _ = writeln!(source, "{indent}}}");
        Ok(Planned::Member(Box::new(Member { place, name, rank, path: function.path.clone(), source, generated })))
    }

    fn input(&self, function: &Function, ty: &Type) -> Result<Input, String> {
        match ty {
            Type::ImplTrait(bounds) => self.converted(bounds),
            Type::Generic(name) if name != "Self" => self.converted(function.generics.get(name).map_or(&[][..], Vec::as_slice)),
            Type::Primitive(name) if PRIMITIVES.contains(&name.as_str()) => Ok(Input::new(name, Conv::Same)),
            Type::BorrowedRef { is_mutable: false, type_, .. } => self.borrowed_input(type_),
            Type::ResolvedPath(path) => {
                let canonical = self.context.renderer.path(path);
                match (canonical.as_str(), type_args(path).as_slice()) {
                    ("core::option::Option", [inner]) => {
                        let inner = self.input(function, inner)?;
                        if matches!(inner.conv, Conv::Option(_)) {
                            return Err("a nested Option".to_owned());
                        }
                        if matches!(inner.conv, Conv::Keyed { .. }) {
                            return Err("an optional keyed handle".to_owned());
                        }
                        Ok(Input { ffi: format!("ffi::Option<{}>", inner.ffi), conv: Conv::Option(Box::new(inner.conv)) })
                    }
                    ("alloc::vec::Vec", [Type::Primitive(byte)]) if byte == "u8" => Ok(Input::new("ffi::Slice<u8>", Conv::BytesOwned)),
                    _ => match self.mapping(&canonical) {
                        Some(Mapping::String) => Ok(Input::new("ffi::String", Conv::Text)),
                        Some(Mapping::Bytes) => Ok(Input::new("ffi::Slice<u8>", Conv::BytesInto)),
                        Some(Mapping::Path) => Ok(Input::new("ffi::Slice<u8>", Conv::Path { borrowed: false })),
                        Some(Mapping::Custom(custom)) => {
                            Ok(Input::new(carrier_in(custom.ffi), Conv::Custom { function: custom.into_rust, borrowed: false }))
                        }
                        Some(Mapping::Keyed(keyed)) => self.keyed_input(&keyed, false),
                        Some(Mapping::Handle) | None => Err(self.rendered(ty)),
                    },
                }
            }
            _ => Err(self.rendered(ty)),
        }
    }

    fn borrowed_input(&self, inner: &Type) -> Result<Input, String> {
        match inner {
            Type::Primitive(name) if name == "str" => Ok(Input::new("ffi::String", Conv::Str)),
            Type::Primitive(name) if PRIMITIVES.contains(&name.as_str()) => Ok(Input::new(name, Conv::Ref)),
            Type::Slice(element) if is_byte(element) => Ok(Input::new("ffi::Slice<u8>", Conv::Bytes)),
            Type::ResolvedPath(path) => match self.mapping(&self.context.renderer.path(path)) {
                Some(Mapping::Bytes) => Ok(Input::new("ffi::Slice<u8>", Conv::BytesInto)),
                Some(Mapping::Path) => Ok(Input::new("ffi::Slice<u8>", Conv::Path { borrowed: true })),
                Some(Mapping::Custom(custom)) => {
                    Ok(Input::new(carrier_in(custom.ffi), Conv::Custom { function: custom.into_rust, borrowed: true }))
                }
                Some(Mapping::Keyed(keyed)) => self.keyed_input(&keyed, true),
                _ => Err(format!("&{}", self.rendered(inner))),
            },
            _ => Err(format!("&{}", self.rendered(inner))),
        }
    }

    /// A generic parameter, satisfied through a converting bound or a trait implementation.
    fn converted(&self, bounds: &[GenericBound]) -> Result<Input, String> {
        let unsupported = || format!("generic parameter `{}`", self.context.renderer.bounds(bounds));
        let Some(Conversion { mapping, borrowed }) = self.context.classifier.conversion(bounds) else { return Err(unsupported()) };
        match mapping {
            Mapping::String => Ok(Input::new("ffi::String", if borrowed { Conv::Str } else { Conv::Text })),
            Mapping::Bytes => Ok(Input::new("ffi::Slice<u8>", if borrowed { Conv::Bytes } else { Conv::BytesOwned })),
            Mapping::Path => Ok(Input::new("ffi::Slice<u8>", Conv::Path { borrowed: false })),
            Mapping::Custom(custom) => Ok(Input::new(carrier_in(custom.ffi), Conv::Custom { function: custom.into_rust, borrowed: false })),
            Mapping::Handle | Mapping::Keyed(_) => Err(unsupported()),
        }
    }

    /// The FFI type of a result and the conversion of `value` into it.
    fn output(&self, ty: &Type, value: &str) -> Result<(String, String), String> {
        match ty {
            Type::Tuple(types) if types.is_empty() => Ok(("()".to_owned(), value.to_owned())),
            Type::Primitive(name) if PRIMITIVES.contains(&name.as_str()) => Ok((name.clone(), value.to_owned())),
            Type::BorrowedRef { is_mutable: false, type_, .. } => match type_.as_ref() {
                Type::Primitive(name) if name == "str" => Ok(text_out(value)),
                Type::Primitive(name) if PRIMITIVES.contains(&name.as_str()) => Ok((name.clone(), format!("*{value}"))),
                Type::Slice(element) if is_byte(element) => Ok(bytes_out(value)),
                Type::ResolvedPath(path) => self.mapped_output(&self.context.renderer.path(path), value, ty),
                _ => Err(self.rendered(ty)),
            },
            Type::ResolvedPath(path) => {
                let canonical = self.context.renderer.path(path);
                match (canonical.as_str(), type_args(path).as_slice()) {
                    ("core::option::Option", [inner]) => {
                        let (ffi, convert) = self.output(inner, "value")?;
                        if convert.contains('?') {
                            return Err("an Option of a Result".to_owned());
                        }
                        let mapped = if convert == "value" { value.to_owned() } else { format!("{value}.map(|value| {convert})") };
                        Ok((format!("ffi::Option<{ffi}>"), format!("ffi::Option::from({mapped})")))
                    }
                    ("core::result::Result", [ok, _]) => self.output(ok, &format!("{value}.map_err(runtime::error)?")),
                    ("alloc::vec::Vec", [Type::Primitive(byte)]) if byte == "u8" => Ok(bytes_out(value)),
                    _ => self.mapped_output(&canonical, value, ty),
                }
            }
            _ => Err(self.rendered(ty)),
        }
    }

    /// The shape of a result `output` lowered.
    fn shape(&self, ty: &Type) -> Shape {
        match ty {
            Type::Primitive(name) => Shape::Primitive(name.clone()),
            Type::BorrowedRef { type_, .. } => match type_.as_ref() {
                Type::Primitive(name) if name == "str" => Shape::Text,
                Type::Primitive(name) => Shape::Primitive(name.clone()),
                Type::Slice(_) => Shape::Bytes,
                Type::ResolvedPath(path) => self.mapped_shape(&self.context.renderer.path(path)),
                _ => Shape::Unit,
            },
            Type::ResolvedPath(path) => {
                let canonical = self.context.renderer.path(path);
                match (canonical.as_str(), type_args(path).as_slice()) {
                    ("core::option::Option", [inner]) => Shape::Option(Box::new(self.shape(inner))),
                    ("core::result::Result", [ok, _]) => self.shape(ok),
                    ("alloc::vec::Vec", [_]) => Shape::Bytes,
                    _ => self.mapped_shape(&canonical),
                }
            }
            _ => Shape::Unit,
        }
    }

    fn mapped_shape(&self, canonical: &str) -> Shape {
        match self.mapping(canonical) {
            Some(Mapping::String) => Shape::Text,
            Some(Mapping::Bytes) => Shape::Bytes,
            Some(Mapping::Path) => Shape::Path,
            Some(Mapping::Custom(custom)) => Shape::Custom(custom.ffi),
            Some(Mapping::Handle) => Shape::Service,
            Some(Mapping::Keyed(keyed)) => Shape::Keyed(self.keyed_type(&keyed)),
            None => Shape::Unit,
        }
    }

    fn mapped_output(&self, canonical: &str, value: &str, ty: &Type) -> Result<(String, String), String> {
        match self.mapping(canonical) {
            Some(Mapping::String) => Ok(text_out(value)),
            Some(Mapping::Bytes) => Ok(bytes_out(value)),
            Some(Mapping::Path) => Ok(("ffi::Vec<u8>".to_owned(), format!("runtime::path_out(&{value})"))),
            Some(Mapping::Custom(custom)) => {
                Ok((carrier_out(custom.ffi).to_owned(), format!("runtime::{}(&{value})", custom.into_ffi)))
            }
            Some(Mapping::Keyed(keyed)) => self.key_output(&keyed, &keyed.key_of.replace(VALUE_PLACEHOLDER, &format!("({value})"))),
            Some(Mapping::Handle) | None => Err(self.rendered(ty)),
        }
    }

    /// The owning keyed type and its mapping, if the function belongs to one.
    fn keyed_owner<'f>(&self, function: &'f Function) -> Option<(&'f str, Keyed)> {
        [&function.owner, &function.owner_public].into_iter().flatten().find_map(|path| match self.mapping(path) {
            Some(Mapping::Keyed(keyed)) => Some((path.as_str(), keyed)),
            _ => None,
        })
    }

    /// A keyed handle parameter: its key's FFI form, rebuilt in the body.
    fn keyed_input(&self, keyed: &Keyed, borrowed: bool) -> Result<Input, String> {
        let key = self.key_input(keyed)?;
        let accessor = snake(last_segment(&keyed.repo));
        Ok(Input { ffi: key.ffi, conv: Conv::Keyed { keyed: Box::new(keyed.clone()), key: Box::new(key.conv), accessor, borrowed } })
    }

    /// How a keyed handle's key crosses into Rust.
    fn key_input(&self, keyed: &Keyed) -> Result<Input, String> {
        if PRIMITIVES.contains(&keyed.key.as_str()) {
            return Ok(Input::new(&keyed.key, Conv::Same));
        }
        match self.mapping(&keyed.key) {
            Some(Mapping::String) => Ok(Input::new("ffi::String", Conv::Text)),
            Some(Mapping::Bytes) => Ok(Input::new("ffi::Slice<u8>", Conv::BytesInto)),
            Some(Mapping::Path) => Ok(Input::new("ffi::Slice<u8>", Conv::Path { borrowed: false })),
            Some(Mapping::Custom(custom)) => Ok(Input::new(carrier_in(custom.ffi), Conv::Custom { function: custom.into_rust, borrowed: false })),
            _ => Err(format!("the key `{}` of `{}`", keyed.key, keyed.name)),
        }
    }

    /// The FFI form of a keyed handle's key; `value` is the key expression.
    fn key_output(&self, keyed: &Keyed, value: &str) -> Result<(String, String), String> {
        if PRIMITIVES.contains(&keyed.key.as_str()) {
            return Ok((keyed.key.clone(), value.to_owned()));
        }
        match self.mapping(&keyed.key) {
            Some(Mapping::String) => Ok(text_out(value)),
            Some(Mapping::Bytes) => Ok(bytes_out(value)),
            Some(Mapping::Path) => Ok(("ffi::Vec<u8>".to_owned(), format!("runtime::path_out(&{value})"))),
            Some(Mapping::Custom(custom)) => Ok((carrier_out(custom.ffi).to_owned(), format!("runtime::{}(&{value})", custom.into_ffi))),
            _ => Err(format!("the key `{}` of `{}`", keyed.key, keyed.name)),
        }
    }

    /// A keyed type as the C# layer needs it.
    fn keyed_type(&self, keyed: &Keyed) -> KeyedType {
        let key = if PRIMITIVES.contains(&keyed.key.as_str()) { Shape::Primitive(keyed.key.clone()) } else { self.mapped_shape(&keyed.key) };
        KeyedType { name: keyed.name.clone(), key_name: keyed.key_name.clone(), key: Box::new(key) }
    }

    /// The whole module.
    fn module(&self, members: &[&Member]) -> String {
        let service: Vec<&&Member> = members.iter().filter(|member| member.place == Place::Service).collect();
        let free: Vec<&&Member> = members.iter().filter(|member| member.place == Place::Free).collect();
        let mut out = String::new();
        let _ = writeln!(out, "//! {}", self.context.provenance);
        let _ = writeln!(out, "//!");
        let _ = writeln!(out, "//! The runtime contract is documented in `crates/rustdoc_gen/src/facade.rs` of interoptopus.");
        let _ = writeln!(out);
        let _ = writeln!(out, "#![allow(clippy::needless_question_mark, clippy::unit_arg, reason = \"generated wrappers share one shape\")]");
        let _ = writeln!(out);
        let _ = writeln!(out, "use interoptopus::inventory::RustInventory;");
        let mut imports = vec!["ffi"];
        if !free.is_empty() {
            imports.push("function");
        }
        if !service.is_empty() {
            imports.push("service");
        }
        let _ = writeln!(out, "use interoptopus::{{{}}};", imports.join(", "));
        let _ = writeln!(out);
        if last_segment(self.context.runtime) == "runtime" {
            let _ = writeln!(out, "use {};", self.context.runtime);
        } else {
            let _ = writeln!(out, "use {} as runtime;", self.context.runtime);
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "/// Turns a wrapper body's result into an FFI result.");
        let _ = writeln!(out, "fn finish<T>(body: impl FnOnce() -> Result<T, runtime::Error>) -> ffi::Result<T, runtime::Error> {{");
        let _ = writeln!(out, "    match body() {{");
        let _ = writeln!(out, "        Ok(value) => ffi::Ok(value),");
        let _ = writeln!(out, "        Err(error) => ffi::Err(error),");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "}}");
        if !service.is_empty() {
            let _ = writeln!(out);
            let _ = writeln!(out, "/// The repository handle.");
            let _ = writeln!(out, "#[ffi(service)]");
            let _ = writeln!(out, "pub struct {} {{", self.service);
            let _ = writeln!(out, "    inner: runtime::{},", self.service);
            let _ = writeln!(out, "}}");
            let _ = writeln!(out);
            let _ = writeln!(out, "#[ffi]");
            let _ = writeln!(out, "impl {} {{", self.service);
            for (index, member) in service.iter().enumerate() {
                if index > 0 {
                    let _ = writeln!(out);
                }
                out.push_str(&member.source);
            }
            let _ = writeln!(out, "}}");
        }
        for member in &free {
            let _ = writeln!(out);
            out.push_str(&member.source);
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "/// Registers the generated items with an inventory.");
        let _ = writeln!(out, "pub fn register(inventory: &mut RustInventory) -> &mut RustInventory {{");
        let _ = writeln!(out, "    inventory");
        if !service.is_empty() {
            let _ = writeln!(out, "        .register(service!({}))", self.service);
        }
        for member in &free {
            let _ = writeln!(out, "        .register(function!({}))", member.name);
        }
        let _ = writeln!(out, "}}");
        out
    }
}

/// The Rust argument built from the FFI parameter `name`, adding any statements it needs first.
fn argument(name: &str, conv: &Conv, setup: &mut Vec<String>) -> String {
    match conv {
        Conv::Same => name.to_owned(),
        Conv::Ref => format!("&{name}"),
        Conv::Str => format!("{name}.as_str()"),
        Conv::Text => format!("{name}.into_string()"),
        Conv::Bytes => format!("{name}.as_slice()"),
        Conv::BytesInto => format!("{name}.as_slice().into()"),
        Conv::BytesOwned => format!("{name}.as_slice().to_vec()"),
        Conv::Path { borrowed } => format!("{}runtime::path(&{name})?", if *borrowed { "&" } else { "" }),
        Conv::Custom { function, borrowed } => format!("{}runtime::{function}(&{name})?", if *borrowed { "&" } else { "" }),
        Conv::Keyed { keyed, key, accessor, borrowed } => {
            let key = argument(name, key, setup);
            // A reference to the holder's temporary, kept alive for the body; `{repo}` names it.
            let holder = format!("let {accessor} = &self.inner.{accessor}();");
            if !setup.contains(&holder) {
                setup.push(holder);
            }
            let rebuilt = keyed.rebuild.replace(REPO_PLACEHOLDER, accessor).replace(KEY_PLACEHOLDER, &key);
            let fallible = if keyed.fallible { ".map_err(runtime::error)?" } else { "" };
            setup.push(format!("let {name} = {rebuilt}{fallible};"));
            if *borrowed { format!("&{name}") } else { name.to_owned() }
        }
        Conv::Option(inner) => {
            setup.push(format!("let {name} = {name}.into_option();"));
            let (body, fallible, borrowed) = match inner.as_ref() {
                Conv::Same => ("*value".to_owned(), false, false),
                Conv::Ref => ("value".to_owned(), false, false),
                Conv::Str => ("value.as_str()".to_owned(), false, false),
                Conv::Text => ("value.as_str().to_owned()".to_owned(), false, false),
                Conv::Bytes => ("value.as_slice()".to_owned(), false, false),
                Conv::BytesInto => ("value.as_slice().into()".to_owned(), false, false),
                Conv::BytesOwned => ("value.as_slice().to_vec()".to_owned(), false, false),
                Conv::Path { borrowed } => ("runtime::path(value)".to_owned(), true, *borrowed),
                Conv::Custom { function, borrowed } => (format!("runtime::{function}(value)"), true, *borrowed),
                Conv::Option(_) => unreachable!("nested options are rejected when lowering"),
                Conv::Keyed { .. } => unreachable!("optional keyed handles are rejected when lowering"),
            };
            let mut expression = format!("{name}.as_ref().map(|value| {body})");
            if fallible {
                expression.push_str(".transpose()?");
                if borrowed {
                    expression.push_str(".as_deref()");
                }
            }
            expression
        }
    }
}

/// The shape of a lowered parameter.
fn conv_shape(conv: &Conv, ffi: &str) -> Shape {
    match conv {
        Conv::Same | Conv::Ref => Shape::Primitive(ffi.to_owned()),
        Conv::Str | Conv::Text => Shape::Text,
        Conv::Bytes | Conv::BytesInto | Conv::BytesOwned => Shape::Bytes,
        Conv::Path { .. } => Shape::Path,
        Conv::Custom { .. } => Shape::Custom(if ffi == "ffi::String" { Carrier::String } else { Carrier::Bytes }),
        Conv::Keyed { keyed, key, .. } => {
            Shape::Keyed(KeyedType { name: keyed.name.clone(), key_name: keyed.key_name.clone(), key: Box::new(conv_shape(key, ffi)) })
        }
        Conv::Option(inner) => {
            let inner_ffi = ffi.strip_prefix("ffi::Option<").and_then(|rest| rest.strip_suffix('>')).unwrap_or(ffi);
            Shape::Option(Box::new(conv_shape(inner, inner_ffi)))
        }
    }
}

fn text_out(value: &str) -> (String, String) {
    ("ffi::String".to_owned(), format!("ffi::String::from_string({value}.to_string())"))
}

fn bytes_out(value: &str) -> (String, String) {
    ("ffi::Vec<u8>".to_owned(), format!("ffi::Vec::from_vec({value}.to_vec())"))
}

/// The FFI parameter type of a custom carrier.
const fn carrier_in(carrier: Carrier) -> &'static str {
    match carrier {
        Carrier::String => "ffi::String",
        Carrier::Bytes => "ffi::Slice<u8>",
    }
}

/// The FFI result type of a custom carrier.
const fn carrier_out(carrier: Carrier) -> &'static str {
    match carrier {
        Carrier::String => "ffi::String",
        Carrier::Bytes => "ffi::Vec<u8>",
    }
}

fn is_byte(ty: &Type) -> bool {
    matches!(ty, Type::Primitive(name) if name == "u8")
}

/// A surface path without its duplicate suffix.
fn item_path(path: &str) -> &str {
    path.split('#').next().unwrap_or(path)
}

/// The free-function name for a surface path, such as `gix_env_agent` for `gix::env::agent`.
fn free_name(path: &str) -> String {
    let base = item_path(path).replace("::", "_");
    path.split_once('#').map_or_else(|| base.clone(), |(_, suffix)| format!("{base}_{suffix}"))
}

/// A usable parameter name.
fn parameter_name(name: &str, index: usize) -> String {
    let valid = !name.is_empty() && name != "_" && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if valid { escaped(name) } else { format!("arg{index}") }
}

/// `ThreadSafeRepository` becomes `thread_safe_repository`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (index, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{Custom, Implementations};
    use crate::render::tests::{bound, resolved, shared};
    use rustdoc_types::GenericArg;

    fn renderer() -> Renderer {
        Renderer::from_paths([
            (1, "alloc::string::String"),
            (2, "std::path::Path"),
            (3, "core::result::Result"),
            (4, "demo::Repo"),
            (5, "demo::Error"),
            (6, "core::convert::AsRef"),
            (7, "bstr::bstr::BStr"),
            (8, "core::option::Option"),
            (9, "bstr::bstring::BString"),
            (10, "demo::ThreadSafeRepo"),
            (11, "demo::Oid"),
            (12, "demo::Commit"),
        ])
    }

    fn ty(id: u32) -> Type {
        resolved(id, Vec::new())
    }

    fn generic(id: u32, args: Vec<Type>) -> Type {
        resolved(id, args.into_iter().map(GenericArg::Type).collect())
    }

    fn function(path: &str, owner: Option<&str>, receiver: Receiver, params: Vec<(&str, Type)>, output: Option<Type>) -> Function {
        Function {
            path: path.to_owned(),
            name: last_segment(path).to_owned(),
            owner: owner.map(str::to_owned),
            owner_public: owner.map(str::to_owned),
            receiver,
            params: params.into_iter().map(|(name, ty)| (name.to_owned(), ty)).collect(),
            output,
            generics: BTreeMap::new(),
            deprecated: false,
            is_async: false,
            is_unsafe: false,
        }
    }

    /// Two handle types, bstr's byte strings, a custom object id and a commit keyed by it.
    fn types() -> BTreeMap<String, Mapping> {
        let oid = Mapping::Custom(Custom { ffi: Carrier::String, into_rust: "oid".to_owned(), into_ffi: "oid_out".to_owned() });
        let commit = Mapping::Keyed(Keyed {
            repo: "demo::Repo".to_owned(),
            key: "demo::Oid".to_owned(),
            rebuild: "{repo}.find_commit({key})".to_owned(),
            fallible: true,
            key_of: "{value}.id".to_owned(),
            key_name: "id".to_owned(),
            name: "demo::Commit".to_owned(),
        });
        [
            ("demo::Commit", commit),
            ("demo::Repo", Mapping::Handle),
            ("demo::ThreadSafeRepo", Mapping::Handle),
            ("bstr::bstr::BStr", Mapping::Bytes),
            ("bstr::bstring::BString", Mapping::Bytes),
            ("demo::Oid", oid),
        ]
        .into_iter()
        .map(|(path, mapping)| (path.to_owned(), mapping))
        .collect()
    }

    fn facade(functions: &[Function]) -> Facade {
        let renderer = renderer();
        let types = types();
        let implementations = Implementations::default();
        let classifier = Classifier::new(&renderer, &types, &[], &implementations);
        let context = Context { renderer: &renderer, classifier: &classifier, runtime: "crate::runtime", provenance: "test" };
        let functions: Vec<&Function> = functions.iter().collect();
        emit(&functions, &context).expect("the functions lower")
    }

    #[test]
    fn keyed_handles_cross_as_keys_and_are_rebuilt_per_call() {
        let tree = function("demo::Commit::tree", Some("demo::Commit"), Receiver::Shared, Vec::new(), Some(generic(3, vec![ty(12), ty(5)])));
        let detach = function("demo::Commit::detach", Some("demo::Commit"), Receiver::Value, Vec::new(), Some(ty(11)));
        let head = function("demo::Repo::head_commit", Some("demo::Repo"), Receiver::Shared, Vec::new(), Some(generic(8, vec![ty(12)])));
        let describe = function("demo::Repo::describe", Some("demo::Repo"), Receiver::Shared, vec![("commit", shared(ty(12)))], Some(ty(1)));
        let free = function("demo::describe", None, Receiver::None, vec![("commit", shared(ty(12)))], Some(ty(1)));
        let facade = facade(&[tree, detach, head, describe, free]);
        let source = &facade.source;
        assert!(source.contains("    pub fn commit_tree(&self, commit_key: ffi::String) -> ffi::Result<ffi::String, runtime::Error> {\n        finish(|| {\n            let repo = &self.inner.repo();\n            let commit_key = repo.find_commit(runtime::oid(&commit_key)?).map_err(runtime::error)?;\n            Ok(runtime::oid_out(&(commit_key.tree().map_err(runtime::error)?).id))\n"), "{source}");
        assert!(source.contains("Ok(runtime::oid_out(&commit_key.detach()))"), "by-value methods consume the rebuilt value: {source}");
        assert!(source.contains("Ok(ffi::Option::from(self.inner.repo().head_commit().map(|value| runtime::oid_out(&(value).id))))"), "{source}");
        assert!(source.contains("            let commit = repo.find_commit(runtime::oid(&commit)?).map_err(runtime::error)?;\n            Ok(ffi::String::from_string(self.inner.repo().describe(&commit).to_string()))\n"), "{source}");
        assert!(facade.dropped.iter().any(|(path, why)| path == "demo::describe" && why.starts_with("takes a keyed handle")), "{:?}", facade.dropped);
        let tree = facade.members.iter().find(|member| member.path == "demo::Commit::tree").expect("emitted");
        assert_eq!(tree.keyed_owner.as_ref().map(|owner| owner.name.as_str()), Some("demo::Commit"));
        assert!(matches!(&tree.output, Shape::Keyed(owner) if owner.key_name == "id"));
    }

    #[test]
    fn methods_borrow_the_handle_and_convert_results() {
        let git_dir = function("demo::Repo::git_dir", Some("demo::Repo"), Receiver::Shared, Vec::new(), Some(shared(ty(2))));
        let source = facade(&[git_dir]).source;
        assert!(source.contains("    pub fn git_dir(&self) -> ffi::Result<ffi::Vec<u8>, runtime::Error> {\n"), "{source}");
        assert!(source.contains("finish(|| Ok(runtime::path_out(&self.inner.repo().git_dir())))"), "{source}");
        assert!(source.contains("pub struct Repo {\n    inner: runtime::Repo,\n}"), "{source}");
        assert!(source.contains("use crate::runtime;\n"), "{source}");
    }

    #[test]
    fn functions_returning_the_handle_become_constructors() {
        let directory = Type::ImplTrait(vec![bound(6, vec![GenericArg::Type(ty(2))])]);
        let open = function("demo::open", None, Receiver::None, vec![("directory", directory)], Some(generic(3, vec![ty(4), ty(5)])));
        let source = facade(&[open]).source;
        assert!(source.contains("    pub fn open(directory: ffi::Slice<u8>) -> ffi::Result<Self, runtime::Error> {\n"), "{source}");
        assert!(
            source.contains("Ok(Self { inner: runtime::Repo::from(demo::open(runtime::path(&directory)?).map_err(runtime::error)?) })"),
            "{source}"
        );
    }

    #[test]
    fn methods_returning_the_handle_become_constructors_taking_it() {
        let main = function("demo::Repo::main", Some("demo::Repo"), Receiver::Shared, Vec::new(), Some(generic(3, vec![ty(4), ty(5)])));
        let source = facade(&[main]).source;
        assert!(source.contains("    pub fn main(repo: &Repo) -> ffi::Result<Self, runtime::Error> {\n"), "{source}");
        assert!(source.contains("runtime::Repo::from(repo.inner.repo().main().map_err(runtime::error)?)"), "{source}");
    }

    #[test]
    fn free_functions_convert_text_and_bytes_inline() {
        let params =
            vec![("operation", shared(Type::Primitive("str".to_owned()))), ("message", shared(ty(7))), ("count", Type::Primitive("usize".to_owned()))];
        let message = function("demo::log::message", None, Receiver::None, params, Some(ty(9)));
        let source = facade(&[message]).source;
        assert!(
            source.contains(
                "#[ffi]\npub fn demo_log_message(operation: ffi::String, message: ffi::Slice<u8>, count: usize) -> ffi::Result<ffi::Vec<u8>, runtime::Error> {\n"
            ),
            "{source}"
        );
        assert!(source.contains("ffi::Vec::from_vec(demo::log::message(operation.as_str(), message.as_slice().into(), count).to_vec())"), "{source}");
        assert!(source.contains("        .register(function!(demo_log_message))"), "{source}");
        assert!(!source.contains("service!"), "{source}");
    }

    #[test]
    fn optional_borrowed_inputs_are_bound_before_the_call() {
        let reason = generic(8, vec![shared(ty(7))]);
        let output = generic(8, vec![Type::Primitive("bool".to_owned())]);
        let lock = function("demo::Repo::lock", Some("demo::Repo"), Receiver::Shared, vec![("reason", reason)], Some(output));
        let source = facade(&[lock]).source;
        assert!(
            source.contains("    pub fn lock_(&self, reason: ffi::Option<ffi::Slice<u8>>) -> ffi::Result<ffi::Option<bool>, runtime::Error> {\n"),
            "{source}"
        );
        assert!(source.contains("            let reason = reason.into_option();\n"), "{source}");
        assert!(source.contains("Ok(ffi::Option::from(self.inner.repo().lock(reason.as_ref().map(|value| value.as_slice().into()))))"), "{source}");
    }

    #[test]
    fn other_handle_types_lose_clashes_and_identity_conversions() {
        let primary = function("demo::Repo::git_dir", Some("demo::Repo"), Receiver::Shared, Vec::new(), Some(shared(ty(2))));
        let duplicate =
            function("demo::ThreadSafeRepo::git_dir", Some("demo::ThreadSafeRepo"), Receiver::Shared, Vec::new(), Some(shared(ty(2))));
        let identity = function("demo::ThreadSafeRepo::to_local", Some("demo::ThreadSafeRepo"), Receiver::Shared, Vec::new(), Some(ty(4)));
        let objects =
            function("demo::ThreadSafeRepo::objects_dir", Some("demo::ThreadSafeRepo"), Receiver::Shared, Vec::new(), Some(shared(ty(2))));
        let facade = facade(&[duplicate, identity, primary, objects]);
        assert_eq!(facade.emitted, ["demo::Repo::git_dir", "demo::ThreadSafeRepo::objects_dir"]);
        assert_eq!(
            facade.dropped,
            [
                ("demo::ThreadSafeRepo::git_dir".to_owned(), "duplicate of `demo::Repo::git_dir`".to_owned()),
                ("demo::ThreadSafeRepo::to_local".to_owned(), "converts between handle types".to_owned()),
            ]
        );
        assert!(facade.source.contains("runtime::path_out(&self.inner.thread_safe_repo().objects_dir())"), "{}", facade.source);
        assert!(facade.source.contains("        .register(service!(Repo))"), "{}", facade.source);
    }

    #[test]
    fn custom_mappings_convert_through_their_runtime_functions() {
        let id = Type::ImplTrait(vec![bound(6, vec![GenericArg::Type(ty(11))])]);
        let peel = function("demo::Repo::peel", Some("demo::Repo"), Receiver::Shared, vec![("id", id)], Some(ty(11)));
        let source = facade(&[peel]).source;
        assert!(source.contains("    pub fn peel(&self, id: ffi::String) -> ffi::Result<ffi::String, runtime::Error> {\n"), "{source}");
        assert!(source.contains("finish(|| Ok(runtime::oid_out(&self.inner.repo().peel(runtime::oid(&id)?))))"), "{source}");
    }

    #[test]
    fn unlowerable_items_name_the_parameter() {
        let renderer = renderer();
        let handles = BTreeMap::new();
        let implementations = Implementations::default();
        let classifier = Classifier::new(&renderer, &handles, &[], &implementations);
        let context = Context { renderer: &renderer, classifier: &classifier, runtime: "crate::runtime", provenance: "test" };
        let take = function("demo::take", None, Receiver::None, vec![("repo", ty(4))], None);
        let error = emit(&[&take], &context).unwrap_err();
        assert_eq!(error.0, "demo::take");
        assert!(error.1.starts_with("parameter `repo`"), "{}", error.1);
    }

    #[test]
    fn forbidden_names_are_escaped() {
        let init = function("demo::init", None, Receiver::None, vec![("value", Type::Primitive("bool".to_owned()))], Some(generic(3, vec![ty(4), ty(5)])));
        let source = facade(&[init]).source;
        assert!(source.contains("    pub fn init_(value_: bool) -> ffi::Result<Self, runtime::Error> {\n"), "{source}");
        assert!(source.contains("runtime::Repo::from(demo::init(value_).map_err(runtime::error)?)"), "{source}");
    }

    #[test]
    fn names_follow_rust_paths() {
        assert_eq!(free_name("gix::env::agent"), "gix_env_agent");
        assert_eq!(free_name("gix::open#2"), "gix_open_2");
        assert_eq!(snake("ThreadSafeRepository"), "thread_safe_repository");
        assert_eq!(parameter_name("_", 3), "arg3");
    }
}