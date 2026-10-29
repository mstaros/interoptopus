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
//! A public enum crosses as an `#[ffi]` mirror named after its public path through the root
//! crate (`RemoteDirection` for `gix::remote::Direction`), with explicit discriminants copied
//! and empty tuple or struct variants as unit variants. `to_<mirror>` copies a result into the
//! mirror, converting payloads from references (primitives, text, bytes, paths, custom mappings,
//! keys, options and nested mirrors); an open enum's unknown variant becomes
//! `runtime::error(UnknownVariant(..))`. `from_<mirror>` turns a fieldless mirror back for
//! parameters. Enum parameters with data and other payloads are not generated yet.
//!
//! A public struct whose named fields are all public crosses the same way, as a record: its mirror
//! has the same fields, `to_<mirror>` copies them, and `from_<mirror>` builds an owned struct that
//! is not `#[non_exhaustive]` from primitives, text, bytes, options and nested mirrors.
//!
//! A type mapped to `string` or `bytes` other than `String` (the convert kind) converts through the
//! traits its crate's rustdoc lists: text in through `FromStr` or `TryFrom<&str>` and out through
//! `Display`; bytes in through `From<&[u8]>` or `From<Vec<u8>>` and out through a `Deref` to bytes
//! or `AsRef<[u8]>`. A direction without them is a decision. A type from a crate without rustdoc,
//! such as bstr's, is assumed to parse and display, and to convert from and dereference to bytes.
//!
//! `verdict` runs the emitter on every function the classifier passes and turns what it does not
//! generate into a decision, so an automatic item is a generated one by construction.
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

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use rustdoc_types::{GenericBound, Path, Type};

use crate::classify::{
    AS_REF_BYTES, Carrier, Class, Classifier, Conversion, DISPLAY, EnumDef, FROM_BYTE_VEC, FROM_BYTES, FROM_STR, KEY_PLACEHOLDER, Keyed, Mapping,
    REPO_PLACEHOLDER, Reason, StructDef, TRY_FROM_STR, VALUE_PLACEHOLDER, VariantFields, Verdict,
};
use crate::forbidden::escaped;
use crate::render::{Renderer, last_segment, type_args};
use crate::surface::{Function, Receiver};

/// The std text type, which `string` maps unchanged.
const STRING: &str = "alloc::string::String";

/// Primitives that cross the boundary unchanged.
const PRIMITIVES: [&str; 13] = ["bool", "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64"];

/// Rust keywords, which need a raw identifier in a pattern and another name in a mirror.
const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match",
    "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
    "where", "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof",
    "unsized", "virtual", "yield", "try", "gen",
];

/// Which conversion a mirror needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Flow {
    /// Into the mirror, for results.
    Out,
    /// Back from the mirror, for parameters.
    In,
}

/// A type crossing as an `#[ffi]` mirror.
#[derive(Clone, Copy)]
enum Mirror<'d> {
    Enum(&'d EnumDef),
    Struct(&'d StructDef),
}

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
    /// The mirrors it uses, with the conversion each needs.
    mirrors: BTreeSet<(String, Flow)>,
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
    /// `&String` from text.
    TextRef,
    /// `&[u8]` from bytes.
    Bytes,
    /// A byte-string type converted from `&[u8]` with `Into`.
    BytesInto,
    /// `Vec<u8>`, which also converts into owned byte strings.
    BytesOwned,
    /// A type mapped to `bytes`, built with `From<&[u8]>`, or with `From<Vec<u8>>` when `owned`;
    /// `borrowed` passes a reference.
    BytesFrom { owned: bool, borrowed: bool },
    /// A type mapped to `string`, parsed with `FromStr`, or with `TryFrom<&str>` when `try_from`;
    /// `borrowed` passes a reference.
    Parse { try_from: bool, borrowed: bool },
    /// A path through the runtime; `borrowed` passes `&Path`.
    Path { borrowed: bool },
    /// A custom mapping through the runtime function it names; `borrowed` passes a reference.
    Custom { function: String, borrowed: bool },
    /// An optional value.
    Option(Box<Self>),
    /// A keyed handle rebuilt from its key: `key` converts the FFI key, `accessor` names the
    /// holder accessor of the handle it is rebuilt from; `borrowed` passes a reference.
    Keyed { keyed: Box<Keyed>, key: Box<Self>, accessor: String, borrowed: bool },
    /// A boxed value.
    Boxed(Box<Self>),
    /// A `Cow` borrowing the value.
    Cow(Box<Self>),
    /// A fieldless enum or a record built back from its mirror by the named conversion; `borrowed`
    /// passes a reference.
    Mirror { function: String, borrowed: bool },
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
    let emitter = Emitter { context, service: service_name(context.classifier), used: RefCell::default() };
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
    Ok(Facade { source: emitter.module(&members)?, emitted: paths, dropped })
}

/// The classifier's verdict, with an automatic function the emitter does not generate turned
/// into a decision (category `lowering`, with the emitter's reason).
#[must_use]
pub fn verdict(function: &Function, context: &Context<'_>) -> Verdict {
    let verdict = context.classifier.classify(function);
    if verdict.class != Class::Automatic {
        return verdict;
    }
    let emitter = Emitter { context, service: service_name(context.classifier), used: RefCell::default() };
    match emitter.member(function) {
        Ok(Planned::Member(_)) => verdict,
        Ok(Planned::Dropped(why)) | Err(why) => Verdict { class: Class::Decision, reasons: vec![Reason::lowering(why)] },
    }
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
    /// The mirrors the member being lowered uses.
    used: RefCell<BTreeSet<(String, Flow)>>,
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
        self.used.borrow_mut().clear();
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
        let mut takes_keyed = false;
        for (index, (name, ty)) in function.params.iter().enumerate() {
            let name = parameter_name(name, index);
            let input = self.input(function, ty).map_err(|why| format!("parameter `{name}`: {why}"))?;
            takes_keyed |= is_keyed(&input.conv);
            declarations.push(format!("{name}: {}", input.ffi));
            arguments.push(argument(&name, &input.conv, &mut setup));
        }
        if takes_keyed && (!method || constructor.is_some()) {
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
                declarations.insert(0, format!("{handle_parameter}: &Self"));
                format!("{handle_parameter}.inner.{accessor}().{}({})", function.name, arguments.join(", "))
            }
            (true, Some(accessor)) => {
                let holder = format!("let {accessor} = &self.inner.{accessor}();");
                let receiver = if setup.contains(&holder) { accessor.clone() } else { format!("self.inner.{accessor}()") };
                format!("{receiver}.{}({})", function.name, arguments.join(", "))
            }
            (true, None) => return Err("a method of a type that is not a handle".to_owned()),
            (false, _) => format!("{}({})", rust_path(item_path(&function.path)), arguments.join(", ")),
            }
        };
        let (place, name, receiver, returns, value) = if let Some(fallible) = constructor {
            let value = if fallible { format!("{call}.map_err(runtime::error)?") } else { call };
            let holder = format!("Self {{ inner: runtime::{}::from({value}) }}", self.service);
            (Place::Service, escaped(&function.name), "", "Self".to_owned(), holder)
        } else {
            let unit = Type::Tuple(Vec::new());
            let (returns, value) = self.output(function, function.output.as_ref().unwrap_or(&unit), &call)?;
            if method {
                (Place::Service, escaped(&member_name), "&self", returns, value)
            } else {
                (Place::Free, escaped(&free_name(&function.path)), "", returns, value)
            }
        };
        let mut parameters = declarations;
        if !receiver.is_empty() {
            parameters.insert(0, receiver.to_owned());
        }
        let source = wrapper(place, &function.path, &name, &parameters, &returns, &setup, &value);
        let mirrors = self.used.take();
        Ok(Planned::Member(Box::new(Member { place, name, rank, path: function.path.clone(), source, mirrors })))
    }

    fn input(&self, function: &Function, ty: &Type) -> Result<Input, String> {
        match ty {
            Type::ImplTrait(bounds) => self.converted(bounds),
            Type::Generic(name) if name != "Self" => self.converted(function.generics.get(name).map_or(&[][..], Vec::as_slice)),
            Type::Generic(_) => self.owner_input(function, false),
            Type::Primitive(name) if PRIMITIVES.contains(&name.as_str()) => Ok(Input::new(name, Conv::Same)),
            Type::BorrowedRef { is_mutable: false, type_, .. } => self.borrowed_input(function, type_),
            Type::ResolvedPath(path) => {
                let canonical = self.context.renderer.path(path);
                match (canonical.as_str(), type_args(path).as_slice()) {
                    ("core::option::Option", [inner]) => {
                        let inner = self.input(function, inner)?;
                        let simple = matches!(
                            inner.conv,
                            Conv::Same
                                | Conv::Ref
                                | Conv::Str
                                | Conv::Text
                                | Conv::Bytes
                                | Conv::BytesInto
                                | Conv::BytesOwned
                                | Conv::Path { .. }
                                | Conv::Custom { .. }
                                | Conv::BytesFrom { .. }
                                | Conv::Parse { .. }
                                | Conv::Mirror { borrowed: false, .. }
                        );
                        if !simple {
                            return Err(format!("an optional {}", self.rendered(ty)));
                        }
                        Ok(Input { ffi: format!("ffi::Option<{}>", inner.ffi), conv: Conv::Option(Box::new(inner.conv)) })
                    }
                    ("alloc::vec::Vec", [Type::Primitive(byte)]) if byte == "u8" => Ok(Input::new("ffi::Slice<u8>", Conv::BytesOwned)),
                    ("alloc::boxed::Box", [inner]) => {
                        let inner = self.input(function, inner)?;
                        Ok(Input { ffi: inner.ffi, conv: Conv::Boxed(Box::new(inner.conv)) })
                    }
                    ("alloc::borrow::Cow", [inner]) => {
                        let inner = self.borrowed_input(function, inner)?;
                        Ok(Input { ffi: inner.ffi, conv: Conv::Cow(Box::new(inner.conv)) })
                    }
                    _ => match self.mapped_input(&canonical, self.mapping(&canonical), false)? {
                        Some(input) => Ok(input),
                        None => self.mirror_input(&canonical, false)?.ok_or_else(|| self.rendered(ty)),
                    },
                }
            }
            _ => Err(self.rendered(ty)),
        }
    }

    fn borrowed_input(&self, function: &Function, inner: &Type) -> Result<Input, String> {
        match inner {
            Type::Primitive(name) if name == "str" => Ok(Input::new("ffi::String", Conv::Str)),
            Type::Primitive(name) if PRIMITIVES.contains(&name.as_str()) => Ok(Input::new(name, Conv::Ref)),
            Type::Slice(element) if is_byte(element) => Ok(Input::new("ffi::Slice<u8>", Conv::Bytes)),
            Type::ResolvedPath(path) => {
                let canonical = self.context.renderer.path(path);
                match self.mapped_input(&canonical, self.mapping(&canonical), true)? {
                    Some(input) => Ok(input),
                    None => self.mirror_input(&canonical, true)?.ok_or_else(|| format!("&{}", self.rendered(inner))),
                }
            }
            Type::Generic(name) if name == "Self" => self.owner_input(function, true),
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
    fn output(&self, function: &Function, ty: &Type, value: &str) -> Result<(String, String), String> {
        match ty {
            Type::Generic(name) if name == "Self" => self.owner_output(function, value, ty),
            Type::Tuple(types) if types.is_empty() => Ok(("()".to_owned(), value.to_owned())),
            Type::Primitive(name) if PRIMITIVES.contains(&name.as_str()) => Ok((name.clone(), value.to_owned())),
            Type::BorrowedRef { is_mutable: false, type_, .. } => match type_.as_ref() {
                Type::Primitive(name) if name == "str" => Ok(text_out(value)),
                Type::Primitive(name) if PRIMITIVES.contains(&name.as_str()) => Ok((name.clone(), format!("*{value}"))),
                Type::Slice(element) if is_byte(element) => Ok(bytes_out(value)),
                Type::ResolvedPath(path) => self.mapped_output(&self.context.renderer.path(path), value, ty),
                Type::Generic(name) if name == "Self" => self.owner_output(function, value, ty),
                _ => Err(self.rendered(ty)),
            },
            Type::ResolvedPath(path) => {
                let canonical = self.context.renderer.path(path);
                match (canonical.as_str(), type_args(path).as_slice()) {
                    ("core::option::Option", [inner]) => {
                        let (ffi, convert) = self.output(function, inner, "value")?;
                        let mapped = if convert == "value" {
                            value.to_owned()
                        } else if !convert.contains('?') {
                            format!("{value}.map(|value| {convert})")
                        } else if self.is_result(inner) {
                            return Err("an Option of a Result".to_owned());
                        } else {
                            format!("{value}.map(|value| Ok::<_, runtime::Error>({convert})).transpose()?")
                        };
                        Ok((format!("ffi::Option<{ffi}>"), format!("ffi::Option::from({mapped})")))
                    }
                    ("core::result::Result", [ok, _]) => self.output(function, ok, &format!("{value}.map_err(runtime::error)?")),
                    ("alloc::vec::Vec", [Type::Primitive(byte)]) if byte == "u8" => Ok(bytes_out(value)),
                    ("alloc::boxed::Box", [inner]) => self.output(function, inner, &format!("(*{value})")),
                    ("alloc::borrow::Cow", [inner]) => self.output(function, &borrowed(inner), &format!("(&*{value})")),
                    _ => self.mapped_output(&canonical, value, ty),
                }
            }
            _ => Err(self.rendered(ty)),
        }
    }

    /// A mapped type as a parameter, owned or borrowed; `None` when it does not cross.
    fn mapped_input(&self, canonical: &str, mapping: Option<Mapping>, borrowed: bool) -> Result<Option<Input>, String> {
        Ok(Some(match mapping {
            Some(Mapping::String) if canonical == STRING && borrowed => Input::new("ffi::String", Conv::TextRef),
            Some(Mapping::String) if canonical == STRING => Input::new("ffi::String", Conv::Text),
            Some(Mapping::String) => return self.text_input(canonical, borrowed).map(Some),
            Some(Mapping::Bytes) => return self.bytes_input(canonical, borrowed).map(Some),
            Some(Mapping::Path) => Input::new("ffi::Slice<u8>", Conv::Path { borrowed }),
            Some(Mapping::Custom(custom)) => Input::new(carrier_in(custom.ffi), Conv::Custom { function: custom.into_rust, borrowed }),
            Some(Mapping::Keyed(keyed)) => return self.keyed_input(&keyed, borrowed).map(Some),
            Some(Mapping::Handle) | None => return Ok(None),
        }))
    }

    /// The owner's canonical or public path, whichever the type map has.
    fn owner_canonical<'f>(&self, function: &'f Function) -> Option<&'f str> {
        [&function.owner, &function.owner_public].into_iter().flatten().find(|path| self.mapping(path).is_some()).map(String::as_str)
    }

    /// `Self` or `&Self` as a parameter, through the owner's mapping.
    fn owner_input(&self, function: &Function, borrowed: bool) -> Result<Input, String> {
        let owner = self.owner_canonical(function).unwrap_or_default();
        self.mapped_input(owner, self.mapping(owner), borrowed)?.ok_or_else(|| if borrowed { "&Self".to_owned() } else { "Self".to_owned() })
    }

    /// `Self` or `&Self` as a result, through the owner's mapping.
    fn owner_output(&self, function: &Function, value: &str, ty: &Type) -> Result<(String, String), String> {
        let owner = self.owner_canonical(function).ok_or_else(|| self.rendered(ty))?;
        self.mapped_output(owner, value, ty)
    }

    fn mapped_output(&self, canonical: &str, value: &str, ty: &Type) -> Result<(String, String), String> {
        match self.mapping(canonical) {
            Some(Mapping::String) => self.text_output(canonical, value),
            Some(Mapping::Bytes) => self.bytes_output(canonical, value),
            Some(Mapping::Path) => Ok(("ffi::Vec<u8>".to_owned(), format!("runtime::path_out(&{value})"))),
            Some(Mapping::Custom(custom)) => {
                Ok((carrier_out(custom.ffi).to_owned(), format!("runtime::{}(&{value})", custom.into_ffi)))
            }
            Some(Mapping::Keyed(keyed)) => self.key_output(&keyed, &keyed.key_of.replace(VALUE_PLACEHOLDER, &format!("({value})"))),
            Some(Mapping::Handle) => Err(self.rendered(ty)),
            None => self.mirror_output(canonical, value).unwrap_or_else(|| Err(self.rendered(ty))),
        }
    }

    /// Text into a type mapped to `string`: parsed with `FromStr`, or with `TryFrom<&str>`.
    /// Without rustdoc for its crate, `FromStr` is assumed.
    fn text_input(&self, canonical: &str, borrowed: bool) -> Result<Input, String> {
        let implementations = self.context.classifier.implementations();
        let try_from = if !implementations.documents(canonical) || implementations.implements_reference(canonical, FROM_STR) {
            false
        } else if implementations.implements_reference(canonical, TRY_FROM_STR) {
            true
        } else {
            return Err(format!("`{canonical}` maps to string but implements neither `FromStr` nor `TryFrom<&str>`"));
        };
        Ok(Input::new("ffi::String", Conv::Parse { try_from, borrowed }))
    }

    /// Bytes into a type mapped to `bytes`: `into()` when the parameter's own type implements
    /// `From<&[u8]>` (as bstr's `&BStr` does), otherwise built with `From<&[u8]>` or
    /// `From<Vec<u8>>`. Without rustdoc for its crate, `into()` is assumed.
    fn bytes_input(&self, canonical: &str, borrowed: bool) -> Result<Input, String> {
        let implementations = self.context.classifier.implementations();
        let parameter = if borrowed { format!("&{canonical}") } else { canonical.to_owned() };
        if !implementations.documents(canonical) || implementations.implements_exactly(&parameter, FROM_BYTES) {
            return Ok(Input::new("ffi::Slice<u8>", Conv::BytesInto));
        }
        let owned = if implementations.implements_exactly(canonical, FROM_BYTES) {
            false
        } else if implementations.implements_exactly(canonical, FROM_BYTE_VEC) {
            true
        } else {
            return Err(format!("`{canonical}` maps to bytes but implements neither `From<&[u8]>` nor `From<Vec<u8>>`"));
        };
        Ok(Input::new("ffi::Slice<u8>", Conv::BytesFrom { owned, borrowed }))
    }

    /// A type mapped to `string` as a result: its `Display` text. Without rustdoc for its crate,
    /// `Display` is assumed.
    fn text_output(&self, canonical: &str, value: &str) -> Result<(String, String), String> {
        let implementations = self.context.classifier.implementations();
        if canonical == STRING || !implementations.documents(canonical) || implementations.implements(canonical, DISPLAY) {
            Ok(text_out(value))
        } else {
            Err(format!("`{canonical}` maps to string but does not implement `Display`"))
        }
    }

    /// A type mapped to `bytes` as a result: copied through its `Deref` to bytes, or through
    /// `AsRef<[u8]>`. Without rustdoc for its crate, a `Deref` to bytes is assumed.
    fn bytes_output(&self, canonical: &str, value: &str) -> Result<(String, String), String> {
        let implementations = self.context.classifier.implementations();
        let derefs = implementations
            .deref_target(canonical)
            .is_some_and(|target| matches!(target, "[u8]" | "alloc::vec::Vec<u8>") || self.mapping(target) == Some(Mapping::Bytes));
        if !implementations.documents(canonical) || derefs {
            Ok(bytes_out(value))
        } else if implementations.implements_reference(canonical, AS_REF_BYTES) {
            Ok(("ffi::Vec<u8>".to_owned(), format!("ffi::Vec::from_vec(AsRef::<[u8]>::as_ref(&{value}).to_vec())")))
        } else {
            Err(format!("`{canonical}` maps to bytes but neither dereferences to bytes nor implements `AsRef<[u8]>`"))
        }
    }

    fn is_result(&self, ty: &Type) -> bool {
        matches!(ty, Type::ResolvedPath(path) if self.context.renderer.path(path) == "core::result::Result")
    }

    /// The mirror's name, unless it would take the service's.
    fn mirror_name(&self, canonical: &str) -> Result<String, String> {
        let name = self.context.classifier.implementations().mirror(canonical).ok_or_else(|| format!("`{canonical}` has no mirror name"))?;
        if name == self.service {
            return Err(format!("the mirror of `{canonical}` would take the service's name"));
        }
        Ok(name.to_owned())
    }

    /// A parameter crossing as its mirror: a fieldless enum turned back, or a record built from
    /// its fields; `None` when `canonical` is neither a public enum nor a struct with public fields.
    fn mirror_input(&self, canonical: &str, borrowed: bool) -> Result<Option<Input>, String> {
        let implementations = self.context.classifier.implementations();
        let (structure, enumeration) = (implementations.struct_def(canonical), implementations.enum_def(canonical));
        if structure.is_none() && enumeration.is_none() {
            return Ok(None);
        }
        let name = self.mirror_name(canonical)?;
        if let Some(def) = structure {
            if def.borrows {
                return Err(format!("`{canonical}` borrows, so it crosses as a result only"));
            }
            if !def.constructible {
                return Err(format!("`{canonical}` is `#[non_exhaustive]`, so its mirror cannot build it"));
            }
            for (field, ty) in &def.fields {
                self.payload_in(ty, "field").map_err(|why| format!("`{canonical}`, field `{field}`: {why}"))?;
            }
        }
        if enumeration.is_some_and(|def| def.variants.iter().any(|variant| !variant.fields.is_empty() || !variant.constructible)) {
            return Err(format!("`{canonical}` carries data; enum parameters with data are not generated yet"));
        }
        self.used.borrow_mut().insert((canonical.to_owned(), Flow::In));
        Ok(Some(Input::new(&name, Conv::Mirror { function: format!("from_{}", snake(&name)), borrowed })))
    }

    /// A result copied into its mirror; `None` when `canonical` is neither a public enum nor a
    /// struct with public fields.
    fn mirror_output(&self, canonical: &str, value: &str) -> Option<Result<(String, String), String>> {
        let implementations = self.context.classifier.implementations();
        let (structure, enumeration) = (implementations.struct_def(canonical), implementations.enum_def(canonical));
        if structure.is_none() && enumeration.is_none() {
            return None;
        }
        let lowered = self.mirror_name(canonical).and_then(|name| {
            for (field, ty) in structure.map_or(&[][..], |def| def.fields.as_slice()) {
                self.payload_out(ty, "field").map_err(|why| format!("`{canonical}`, field `{field}`: {why}"))?;
            }
            for variant in enumeration.map_or(&[][..], |def| def.variants.as_slice()) {
                if variant.fields == VariantFields::Hidden {
                    return Err(format!("`{canonical}`, variant `{}`: hidden fields", variant.name));
                }
                for (index, ty) in variant.fields.types().into_iter().enumerate() {
                    self.payload_out(ty, &format!("field{index}")).map_err(|why| format!("`{canonical}`, variant `{}`: {why}", variant.name))?;
                }
            }
            self.used.borrow_mut().insert((canonical.to_owned(), Flow::Out));
            Ok((name.clone(), format!("to_{}(&{value})?", snake(&name))))
        });
        Some(lowered)
    }

    /// The FFI type of a payload field and its conversion from `binding`, a reference to it.
    fn payload_out(&self, ty: &Type, binding: &str) -> Result<(String, String), String> {
        match ty {
            Type::Primitive(name) if PRIMITIVES.contains(&name.as_str()) => Ok((name.clone(), format!("*{binding}"))),
            Type::BorrowedRef { is_mutable: false, type_, .. } => match type_.as_ref() {
                Type::Primitive(name) if name == "str" => Ok(text_out(binding)),
                Type::Primitive(name) if PRIMITIVES.contains(&name.as_str()) => Ok((name.clone(), format!("**{binding}"))),
                Type::Slice(element) if is_byte(element) => Ok(bytes_out(binding)),
                Type::ResolvedPath(path) => self.payload_path(path, binding, ty),
                _ => Err(self.rendered(ty)),
            },
            Type::ResolvedPath(path) => self.payload_path(path, binding, ty),
            _ => Err(self.rendered(ty)),
        }
    }

    fn payload_path(&self, path: &Path, binding: &str, ty: &Type) -> Result<(String, String), String> {
        let canonical = self.context.renderer.path(path);
        match (canonical.as_str(), type_args(path).as_slice()) {
            ("alloc::vec::Vec", [Type::Primitive(byte)]) if byte == "u8" => Ok(bytes_out(binding)),
            ("core::option::Option", [inner]) => {
                let (ffi, convert) = self.payload_out(inner, "value")?;
                let mapped = if convert.contains('?') {
                    format!("{binding}.as_ref().map(|value| Ok::<_, runtime::Error>({convert})).transpose()?")
                } else {
                    option_map(binding, &convert)
                };
                Ok((format!("ffi::Option<{ffi}>"), format!("ffi::Option::from({mapped})")))
            }
            (_, []) => self.mapped_output(&canonical, binding, ty),
            _ => Err(self.rendered(ty)),
        }
    }

    /// The FFI type of a field built back from a mirror and its conversion from `binding`, a
    /// reference to the mirror's field.
    fn payload_in(&self, ty: &Type, binding: &str) -> Result<(String, String), String> {
        match ty {
            Type::Primitive(name) if PRIMITIVES.contains(&name.as_str()) => Ok((name.clone(), format!("*{binding}"))),
            Type::ResolvedPath(path) => {
                let canonical = self.context.renderer.path(path);
                match (canonical.as_str(), type_args(path).as_slice()) {
                    ("alloc::string::String", []) => Ok(("ffi::String".to_owned(), format!("{binding}.as_str().to_owned()"))),
                    ("alloc::vec::Vec", [Type::Primitive(byte)]) if byte == "u8" => {
                        Ok(("ffi::Vec<u8>".to_owned(), format!("{binding}.clone().into_vec()")))
                    }
                    ("core::option::Option", [inner]) => {
                        let (ffi, convert) = self.payload_in(inner, "value")?;
                        Ok((format!("ffi::Option<{ffi}>"), option_map(binding, &convert)))
                    }
                    (_, []) => match self.mapping(&canonical) {
                        Some(Mapping::Bytes) => {
                            let implementations = self.context.classifier.implementations();
                            if !implementations.documents(&canonical) || implementations.implements_exactly(&canonical, FROM_BYTE_VEC) {
                                Ok(("ffi::Vec<u8>".to_owned(), format!("{binding}.clone().into_vec().into()")))
                            } else if implementations.implements_exactly(&canonical, FROM_BYTES) {
                                Ok(("ffi::Vec<u8>".to_owned(), format!("From::from({binding}.clone().into_vec().as_slice())")))
                            } else {
                                Err(format!("`{canonical}` maps to bytes but implements neither `From<&[u8]>` nor `From<Vec<u8>>`"))
                            }
                        }
                        Some(_) => Err(format!("{} cannot be built from a mirror yet", self.rendered(ty))),
                        None => match self.mirror_input(&canonical, false)? {
                            Some(Input { ffi, conv: Conv::Mirror { function, .. } }) => Ok((ffi, format!("{function}({binding})"))),
                            _ => Err(self.rendered(ty)),
                        },
                    },
                    _ => Err(format!("{} cannot be built from a mirror yet", self.rendered(ty))),
                }
            }
            _ => Err(format!("{} cannot be built from a mirror yet", self.rendered(ty))),
        }
    }

    /// The mirrors `members` use, with the conversions each needs.
    fn mirror_items(&self, members: &[&Member]) -> Result<String, (String, String)> {
        let implementations = self.context.classifier.implementations();
        let mut flows: BTreeMap<&str, BTreeSet<Flow>> = BTreeMap::new();
        for (canonical, flow) in members.iter().flat_map(|member| member.mirrors.iter()) {
            flows.entry(canonical.as_str()).or_default().insert(*flow);
        }
        let mut mirrors = Vec::new();
        for (canonical, flows) in flows {
            let failed = |why: String| (canonical.to_owned(), why);
            let def = match (implementations.enum_def(canonical), implementations.struct_def(canonical)) {
                (Some(def), _) => Mirror::Enum(def),
                (None, Some(def)) => Mirror::Struct(def),
                (None, None) => return Err(failed("neither a public enum nor a struct with public fields".to_owned())),
            };
            let name = self.mirror_name(canonical).map_err(failed)?;
            let path = implementations.nameable(canonical).map(|path| rust_path(&path)).ok_or_else(|| failed("not nameable through the root crate".to_owned()))?;
            mirrors.push((name, def, path, flows));
        }
        mirrors.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out = String::new();
        if mirrors.iter().any(|(_, def, _, flows)| matches!(def, Mirror::Enum(def) if def.open) && flows.contains(&Flow::Out)) {
            let _ = writeln!(out);
            let _ = writeln!(out, "/// A variant this facade does not know: the enum is `#[non_exhaustive]` or hides variants.");
            let _ = writeln!(out, "#[derive(Debug)]");
            let _ = writeln!(out, "struct UnknownVariant(&'static str);");
            let _ = writeln!(out);
            let _ = writeln!(out, "impl std::fmt::Display for UnknownVariant {{");
            let _ = writeln!(out, "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{");
            let _ = writeln!(out, "        write!(f, \"a variant of `{{}}` this facade does not know\", self.0)");
            let _ = writeln!(out, "    }}");
            let _ = writeln!(out, "}}");
            let _ = writeln!(out);
            let _ = writeln!(out, "impl std::error::Error for UnknownVariant {{}}");
        }
        for (name, def, path, flows) in &mirrors {
            self.mirror_definition(&mut out, name, *def, path, flows).map_err(|why| (path.clone(), why))?;
        }
        Ok(out)
    }

    /// One mirror and the conversions `flows` asks for.
    fn mirror_definition(&self, out: &mut String, name: &str, def: Mirror<'_>, path: &str, flows: &BTreeSet<Flow>) -> Result<(), String> {
        match def {
            Mirror::Enum(def) => {
                self.mirror_enum(out, name, def, path)?;
                if flows.contains(&Flow::Out) {
                    self.mirror_out(out, name, def, path)?;
                }
                if flows.contains(&Flow::In) {
                    mirror_in(out, name, def, path);
                }
            }
            Mirror::Struct(def) => {
                self.mirror_struct(out, name, def, path)?;
                if flows.contains(&Flow::Out) {
                    self.struct_out(out, name, def, path)?;
                }
                if flows.contains(&Flow::In) {
                    self.struct_in(out, name, def, path)?;
                }
            }
        }
        Ok(())
    }

    /// The `#[ffi]` mirror of a struct.
    fn mirror_struct(&self, out: &mut String, name: &str, def: &StructDef, path: &str) -> Result<(), String> {
        let fields = def
            .fields
            .iter()
            .map(|(field, ty)| {
                self.payload_out(ty, "field").map(|(ffi, _)| format!("    pub {}: {ffi},", mirror_field(field))).map_err(|why| format!("field `{field}`: {why}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let _ = writeln!(out);
        let _ = writeln!(out, "/// Mirrors `{path}`.");
        let _ = writeln!(out, "#[ffi]");
        let _ = writeln!(out, "pub struct {name} {{");
        for field in fields {
            let _ = writeln!(out, "{field}");
        }
        let _ = writeln!(out, "}}");
        Ok(())
    }

    /// `to_<mirror>`, which copies a struct into its mirror.
    fn struct_out(&self, out: &mut String, name: &str, def: &StructDef, path: &str) -> Result<(), String> {
        let patterns: Vec<String> = def.fields.iter().enumerate().map(|(index, (field, _))| format!("{}: field{index}", raw(field))).collect();
        let values = def
            .fields
            .iter()
            .enumerate()
            .map(|(index, (field, ty))| {
                self.payload_out(ty, &format!("field{index}"))
                    .map(|(_, value)| format!("{}: {value}", mirror_field(field)))
                    .map_err(|why| format!("field `{field}`: {why}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let _ = writeln!(out);
        let _ = writeln!(out, "/// Copies a `{path}` into its mirror.");
        let _ = writeln!(out, "fn to_{}(value: &{path}) -> Result<{name}, runtime::Error> {{", snake(name));
        let _ = writeln!(out, "    let {path} {{ {}, .. }} = value;", patterns.join(", "));
        let _ = writeln!(out, "    Ok({name} {{ {} }})", values.join(", "));
        let _ = writeln!(out, "}}");
        Ok(())
    }

    /// `from_<mirror>`, which builds a struct from its mirror.
    fn struct_in(&self, out: &mut String, name: &str, def: &StructDef, path: &str) -> Result<(), String> {
        let patterns: Vec<String> = def.fields.iter().enumerate().map(|(index, (field, _))| format!("{}: field{index}", mirror_field(field))).collect();
        let values = def
            .fields
            .iter()
            .enumerate()
            .map(|(index, (field, ty))| {
                self.payload_in(ty, &format!("field{index}")).map(|(_, value)| format!("{}: {value}", raw(field))).map_err(|why| format!("field `{field}`: {why}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let _ = writeln!(out);
        let _ = writeln!(out, "/// Builds a `{path}` from its mirror.");
        let _ = writeln!(out, "fn from_{}(value: &{name}) -> {path} {{", snake(name));
        let _ = writeln!(out, "    let {name} {{ {} }} = value;", patterns.join(", "));
        let _ = writeln!(out, "    {path} {{ {} }}", values.join(", "));
        let _ = writeln!(out, "}}");
        Ok(())
    }

    /// The `#[ffi]` mirror of an enum.
    fn mirror_enum(&self, out: &mut String, name: &str, def: &EnumDef, path: &str) -> Result<(), String> {
        let _ = writeln!(out);
        let _ = writeln!(out, "/// Mirrors `{path}`.");
        let _ = writeln!(out, "#[ffi]");
        let _ = writeln!(out, "pub enum {name} {{");
        for variant in &def.variants {
            let discriminant = variant.discriminant.as_ref().map_or_else(String::new, |value| format!(" = {value}"));
            let payload = match &variant.fields {
                fields if fields.is_empty() => String::new(),
                VariantFields::Tuple(types) => {
                    let types = types.iter().map(|ty| self.payload_out(ty, "field").map(|(ffi, _)| ffi)).collect::<Result<Vec<_>, _>>()?;
                    format!("({})", types.join(", "))
                }
                VariantFields::Named(fields) => {
                    let fields = fields
                        .iter()
                        .map(|(field, ty)| self.payload_out(ty, "field").map(|(ffi, _)| format!("{}: {ffi}", mirror_field(field))))
                        .collect::<Result<Vec<_>, _>>()?;
                    format!(" {{ {} }}", fields.join(", "))
                }
                VariantFields::Unit | VariantFields::Hidden => return Err(format!("variant `{}`: hidden fields", variant.name)),
            };
            let _ = writeln!(out, "    {}{payload}{discriminant},", escaped(&variant.name));
        }
        let _ = writeln!(out, "}}");
        Ok(())
    }

    /// `to_<mirror>`, which copies a value into the mirror.
    fn mirror_out(&self, out: &mut String, name: &str, def: &EnumDef, path: &str) -> Result<(), String> {
        let _ = writeln!(out);
        let _ = writeln!(out, "/// Copies a `{path}` into its mirror.");
        let _ = writeln!(out, "fn to_{}(value: &{path}) -> Result<{name}, runtime::Error> {{", snake(name));
        let _ = writeln!(out, "    Ok(match value {{");
        for variant in &def.variants {
            let source = format!("{path}::{}", variant.name);
            let target = format!("{name}::{}", escaped(&variant.name));
            let (pattern, value) = match &variant.fields {
                VariantFields::Unit => (source, target),
                VariantFields::Tuple(types) if types.is_empty() => (format!("{source}()"), target),
                VariantFields::Named(fields) if fields.is_empty() => (format!("{source} {{ .. }}"), target),
                VariantFields::Tuple(types) => {
                    let bindings: Vec<String> = (0..types.len()).map(|index| format!("field{index}")).collect();
                    let values = types.iter().zip(&bindings).map(|(ty, binding)| self.payload_out(ty, binding).map(|(_, value)| value)).collect::<Result<Vec<_>, _>>()?;
                    (format!("{source}({}, ..)", bindings.join(", ")), format!("{target}({})", values.join(", ")))
                }
                VariantFields::Named(fields) => {
                    let patterns: Vec<String> = fields.iter().enumerate().map(|(index, (field, _))| format!("{}: field{index}", raw(field))).collect();
                    let values = fields
                        .iter()
                        .enumerate()
                        .map(|(index, (field, ty))| self.payload_out(ty, &format!("field{index}")).map(|(_, value)| format!("{}: {value}", mirror_field(field))))
                        .collect::<Result<Vec<_>, _>>()?;
                    (format!("{source} {{ {}, .. }}", patterns.join(", ")), format!("{target} {{ {} }}", values.join(", ")))
                }
                VariantFields::Hidden => return Err(format!("variant `{}`: hidden fields", variant.name)),
            };
            let _ = writeln!(out, "        {pattern} => {value},");
        }
        if def.open {
            let _ = writeln!(out, "        _ => return Err(runtime::error(UnknownVariant(\"{path}\"))),");
        }
        let _ = writeln!(out, "    }})");
        let _ = writeln!(out, "}}");
        Ok(())
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
            Some(Mapping::String) if keyed.key == STRING => Ok(Input::new("ffi::String", Conv::Text)),
            Some(Mapping::String) => self.text_input(&keyed.key, false),
            Some(Mapping::Bytes) => self.bytes_input(&keyed.key, false),
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
            Some(Mapping::String) => self.text_output(&keyed.key, value),
            Some(Mapping::Bytes) => self.bytes_output(&keyed.key, value),
            Some(Mapping::Path) => Ok(("ffi::Vec<u8>".to_owned(), format!("runtime::path_out(&{value})"))),
            Some(Mapping::Custom(custom)) => Ok((carrier_out(custom.ffi).to_owned(), format!("runtime::{}(&{value})", custom.into_ffi))),
            _ => Err(format!("the key `{}` of `{}`", keyed.key, keyed.name)),
        }
    }

    /// The whole module.
    fn module(&self, members: &[&Member]) -> Result<String, (String, String)> {
        let service: Vec<&&Member> = members.iter().filter(|member| member.place == Place::Service).collect();
        let free: Vec<&&Member> = members.iter().filter(|member| member.place == Place::Free).collect();
        let mut out = String::new();
        let _ = writeln!(out, "//! {}", self.context.provenance);
        let _ = writeln!(out, "//!");
        let _ = writeln!(out, "//! The runtime contract is documented in `crates/rustdoc_gen/src/facade.rs` of interoptopus.");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "#![allow(clippy::needless_question_mark, clippy::needless_borrow, clippy::unit_arg, clippy::unnecessary_wraps, clippy::enum_variant_names, clippy::upper_case_acronyms, clippy::large_enum_variant, reason = \"generated wrappers and mirrors share one shape\")]"
        );
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
        out.push_str(&self.mirror_items(members)?);
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
        Ok(out)
    }
}

/// `from_<mirror>`, which turns a fieldless mirror back into the enum.
fn mirror_in(out: &mut String, name: &str, def: &EnumDef, path: &str) {
    let _ = writeln!(out);
    let _ = writeln!(out, "/// Turns the mirror back into a `{path}`.");
    let _ = writeln!(out, "fn from_{}(value: &{name}) -> {path} {{", snake(name));
    let _ = writeln!(out, "    match value {{");
    for variant in &def.variants {
        let source = format!("{path}::{}", variant.name);
        let construct = match &variant.fields {
            VariantFields::Tuple(_) => format!("{source}()"),
            VariantFields::Named(_) => format!("{source} {{}}"),
            VariantFields::Unit | VariantFields::Hidden => source,
        };
        let _ = writeln!(out, "        {name}::{} => {construct},", escaped(&variant.name));
    }
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");
}

/// `binding.as_ref()` mapped through `body`, which reads `value`: no map for `value` itself,
/// `copied` for `*value`, the function itself when `body` only calls it, otherwise a closure.
fn option_map(binding: &str, body: &str) -> String {
    let function = body.strip_suffix("(value)").filter(|function| !function.is_empty() && function.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ':'));
    match (body, function) {
        ("value", _) => format!("{binding}.as_ref()"),
        ("*value", _) => format!("{binding}.as_ref().copied()"),
        (_, Some(function)) => format!("{binding}.as_ref().map({function})"),
        _ => format!("{binding}.as_ref().map(|value| {body})"),
    }
}

/// A path usable in Rust code: segments that are keywords, such as gix's
/// `worktree_admin::move` module, become raw identifiers.
fn rust_path(path: &str) -> String {
    path.split("::").map(raw).collect::<Vec<_>>().join("::")
}

/// A field name usable in a pattern: Rust keywords become raw identifiers.
fn raw(name: &str) -> String {
    let bare = name.trim_start_matches("r#");
    if RUST_KEYWORDS.contains(&bare) { format!("r#{bare}") } else { bare.to_owned() }
}

/// A mirror field's name: Rust keywords and names interoptopus forbids get a trailing underscore.
fn mirror_field(name: &str) -> String {
    let bare = name.trim_start_matches("r#");
    if RUST_KEYWORDS.contains(&bare) { format!("{bare}_") } else { escaped(bare) }
}

/// A wrapper's source: its doc comment, signature and body.
fn wrapper(place: Place, path: &str, name: &str, parameters: &[String], returns: &str, setup: &[String], value: &str) -> String {
    let indent = if place == Place::Service { "    " } else { "" };
    let mut source = String::new();
    let _ = writeln!(source, "{indent}/// Wraps `{}`.", item_path(path));
    if place == Place::Free {
        let _ = writeln!(source, "#[ffi]");
    }
    let _ = writeln!(source, "{indent}pub fn {name}({}) -> ffi::Result<{returns}, runtime::Error> {{", parameters.join(", "));
    if setup.is_empty() {
        let _ = writeln!(source, "{indent}    finish(|| Ok({value}))");
    } else {
        let _ = writeln!(source, "{indent}    finish(|| {{");
        for line in setup {
            let _ = writeln!(source, "{indent}        {line}");
        }
        let _ = writeln!(source, "{indent}        Ok({value})");
        let _ = writeln!(source, "{indent}    }})");
    }
    let _ = writeln!(source, "{indent}}}");
    source
}

/// The Rust argument built from the FFI parameter `name`, adding any statements it needs first.
fn argument(name: &str, conv: &Conv, setup: &mut Vec<String>) -> String {
    match conv {
        Conv::Same => name.to_owned(),
        Conv::Ref => format!("&{name}"),
        Conv::Str => format!("{name}.as_str()"),
        Conv::Text => format!("{name}.into_string()"),
        Conv::TextRef => format!("&{name}.into_string()"),
        Conv::Bytes => format!("{name}.as_slice()"),
        Conv::BytesInto => format!("{name}.as_slice().into()"),
        Conv::BytesOwned => format!("{name}.as_slice().to_vec()"),
        Conv::BytesFrom { owned, borrowed } => format!("{}{}", if *borrowed { "&" } else { "" }, bytes_from(name, *owned)),
        Conv::Parse { try_from, borrowed } => format!("{}{}.map_err(runtime::error)?", if *borrowed { "&" } else { "" }, parse_text(name, *try_from)),
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
        Conv::Boxed(inner) => format!("Box::new({})", argument(name, inner, setup)),
        Conv::Cow(inner) => format!("std::borrow::Cow::Borrowed({})", argument(name, inner, setup)),
        Conv::Mirror { function, borrowed } => format!("{}{function}(&{name})", if *borrowed { "&" } else { "" }),
        Conv::Option(inner) => {
            setup.push(format!("let {name} = {name}.into_option();"));
            // `after` turns the optional value into the reference a borrowed parameter takes.
            let (body, fallible, after) = match inner.as_ref() {
                Conv::Same => ("*value".to_owned(), false, ""),
                Conv::Ref => ("value".to_owned(), false, ""),
                Conv::Str => ("value.as_str()".to_owned(), false, ""),
                Conv::Text => ("value.as_str().to_owned()".to_owned(), false, ""),
                Conv::Bytes => ("value.as_slice()".to_owned(), false, ""),
                Conv::BytesInto => ("value.as_slice().into()".to_owned(), false, ""),
                Conv::BytesOwned => ("value.as_slice().to_vec()".to_owned(), false, ""),
                Conv::BytesFrom { owned, borrowed } => (bytes_from("value", *owned), false, if *borrowed { ".as_ref()" } else { "" }),
                Conv::Parse { try_from, borrowed } => {
                    (format!("{}.map_err(runtime::error)", parse_text("value", *try_from)), true, if *borrowed { ".as_ref()" } else { "" })
                }
                Conv::Path { borrowed } => ("runtime::path(value)".to_owned(), true, if *borrowed { ".as_deref()" } else { "" }),
                Conv::Custom { function, borrowed } => (format!("runtime::{function}(value)"), true, if *borrowed { ".as_deref()" } else { "" }),
                Conv::Mirror { function, .. } => (format!("{function}(value)"), false, ""),
                Conv::Option(_) => unreachable!("nested options are rejected when lowering"),
                Conv::Keyed { .. } | Conv::TextRef | Conv::Boxed(_) | Conv::Cow(_) => {
                    unreachable!("only simple optional parameters pass lowering")
                }
            };
            let mut expression = option_map(name, &body);
            if fallible {
                expression.push_str(".transpose()?");
            }
            expression.push_str(after);
            expression
        }
    }
}

/// Text in `name` parsed into the parameter's type, by inference.
fn parse_text(name: &str, try_from: bool) -> String {
    if try_from { format!("TryFrom::try_from({name}.as_str())") } else { format!("{name}.as_str().parse()") }
}

/// Bytes in `name` built into the parameter's type, by inference.
fn bytes_from(name: &str, owned: bool) -> String {
    if owned { format!("From::from({name}.as_slice().to_vec())") } else { format!("From::from({name}.as_slice())") }
}

/// Whether a lowered parameter is a keyed handle, directly or wrapped.
fn is_keyed(conv: &Conv) -> bool {
    match conv {
        Conv::Keyed { .. } => true,
        Conv::Boxed(inner) | Conv::Cow(inner) | Conv::Option(inner) => is_keyed(inner),
        _ => false,
    }
}

/// A shared borrow of `ty`.
fn borrowed(ty: &Type) -> Type {
    Type::BorrowedRef { lifetime: None, is_mutable: false, type_: Box::new(ty.clone()) }
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
    use crate::classify::{Custom, Implementations, StructDef, VariantDef};
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
            (13, "alloc::boxed::Box"),
            (14, "alloc::borrow::Cow"),
            (15, "alloc::vec::Vec"),
            (16, "demo::Mode"),
            (17, "demo::inner::Event"),
            (18, "demo::Shape"),
            (19, "demo::Stats"),
            (20, "demo::Label"),
            (21, "demo::Name"),
            (22, "demo::Tag"),
            (23, "demo::Blob"),
            (24, "demo::Chunk"),
            (25, "demo::Opaque"),
            (26, "other::Ext"),
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
        facade_with(functions, &Implementations::default())
    }

    fn unit(name: &str) -> VariantDef {
        VariantDef { name: name.to_owned(), discriminant: None, fields: VariantFields::Unit, constructible: true }
    }

    /// A fieldless enum with an explicit discriminant and an empty tuple variant, an open data enum
    /// and a data enum whose payload the emitter does not lower yet.
    fn enums() -> Implementations {
        let mode = EnumDef {
            public: "demo::Mode".to_owned(),
            variants: vec![unit("Read"), VariantDef { discriminant: Some("5".to_owned()), ..unit("Write") }, VariantDef { fields: VariantFields::Tuple(Vec::new()), ..unit("Admin") }],
            open: false,
            generic: false,
        };
        let event = EnumDef {
            public: "demo::Event".to_owned(),
            variants: vec![
                VariantDef { fields: VariantFields::Tuple(vec![Type::Primitive("u32".to_owned()), ty(11)]), ..unit("Opened") },
                VariantDef { fields: VariantFields::Named(vec![("from".to_owned(), ty(1)), ("type".to_owned(), ty(16))]), ..unit("Renamed") },
                unit("Closed"),
            ],
            open: true,
            generic: false,
        };
        let shape = EnumDef { public: "demo::Shape".to_owned(), variants: vec![VariantDef { fields: VariantFields::Tuple(vec![generic(15, vec![ty(1)])]), ..unit("Maybe") }], open: false, generic: false };
        Implementations::default().with_root("demo").with_enum("demo::Mode", mode).with_enum("demo::inner::Event", event).with_enum("demo::Shape", shape)
    }

    /// The enums plus a record with a keyword field, an enum and an option, and a borrowed view.
    fn records() -> Implementations {
        let field = |name: &str, ty: Type| (name.to_owned(), ty);
        let stats = StructDef {
            public: "demo::Stats".to_owned(),
            fields: vec![field("count", Type::Primitive("u32".to_owned())), field("type", ty(1)), field("mode", ty(16)), field("note", generic(8, vec![ty(1)]))],
            borrows: false,
            constructible: true,
            generic: false,
        };
        let bytes = shared(Type::Slice(Box::new(Type::Primitive("u8".to_owned()))));
        let label = StructDef {
            public: "demo::Label".to_owned(),
            fields: vec![field("text", shared(Type::Primitive("str".to_owned()))), field("data", bytes)],
            borrows: true,
            constructible: true,
            generic: false,
        };
        enums().with_struct("demo::Stats", stats).with_struct("demo::Label", label)
    }

    fn facade_with(functions: &[Function], implementations: &Implementations) -> Facade {
        let renderer = renderer();
        let types = types();
        let classifier = Classifier::new(&renderer, &types, &[], implementations);
        let context = Context { renderer: &renderer, classifier: &classifier, runtime: "crate::runtime", provenance: "test" };
        let functions: Vec<&Function> = functions.iter().collect();
        emit(&functions, &context).expect("the functions lower")
    }

    #[test]
    fn enums_cross_as_mirrors() {
        let mode = function("demo::Repo::mode", Some("demo::Repo"), Receiver::Shared, Vec::new(), Some(ty(16)));
        let set_mode = function("demo::Repo::set_mode", Some("demo::Repo"), Receiver::Shared, vec![("mode", ty(16)), ("fallback", generic(8, vec![ty(16)]))], None);
        let event = function("demo::Repo::last_event", Some("demo::Repo"), Receiver::Shared, Vec::new(), Some(generic(8, vec![ty(17)])));
        let source = facade_with(&[mode, set_mode, event], &enums()).source;
        assert!(source.contains("/// Mirrors `demo::Mode`.\n#[ffi]\npub enum Mode {\n    Read,\n    Write = 5,\n    Admin,\n}\n"), "{source}");
        assert!(source.contains("fn to_mode(value: &demo::Mode) -> Result<Mode, runtime::Error> {\n    Ok(match value {\n        demo::Mode::Read => Mode::Read,\n        demo::Mode::Write => Mode::Write,\n        demo::Mode::Admin() => Mode::Admin,\n    })\n}\n"), "{source}");
        assert!(source.contains("fn from_mode(value: &Mode) -> demo::Mode {\n    match value {\n        Mode::Read => demo::Mode::Read,\n        Mode::Write => demo::Mode::Write,\n        Mode::Admin => demo::Mode::Admin(),\n    }\n}\n"), "{source}");
        assert!(source.contains("pub enum Event {\n    Opened(u32, ffi::String),\n    Renamed { from_: ffi::String, type_: Mode },\n    Closed,\n}\n"), "`from` is a name interoptopus forbids: {source}");
        assert!(source.contains("        demo::Event::Opened(field0, field1, ..) => Event::Opened(*field0, runtime::oid_out(&field1)),\n"), "{source}");
        assert!(source.contains("        demo::Event::Renamed { from: field0, r#type: field1, .. } => Event::Renamed { from_: ffi::String::from_string(field0.to_string()), type_: to_mode(&field1)? },\n"), "{source}");
        assert!(source.contains("        _ => return Err(runtime::error(UnknownVariant(\"demo::Event\"))),\n"), "an open enum falls back: {source}");
        assert!(source.contains("struct UnknownVariant(&'static str);"), "{source}");
        assert!(source.contains("Ok(to_mode(&self.inner.repo().mode())?)"), "{source}");
        assert!(source.contains("pub fn set_mode(&self, mode: Mode, fallback: ffi::Option<Mode>)"), "{source}");
        assert!(source.contains("self.inner.repo().set_mode(from_mode(&mode), fallback.as_ref().map(from_mode))"), "{source}");
        assert!(source.contains("Ok(ffi::Option::from(self.inner.repo().last_event().map(|value| Ok::<_, runtime::Error>(to_event(&value)?)).transpose()?))"), "{source}");
    }

    #[test]
    fn structs_cross_as_mirrors() {
        let stats = function("demo::Repo::stats", Some("demo::Repo"), Receiver::Shared, Vec::new(), Some(ty(19)));
        let set_stats = function("demo::Repo::set_stats", Some("demo::Repo"), Receiver::Shared, vec![("stats", ty(19))], None);
        let label = function("demo::Repo::label", Some("demo::Repo"), Receiver::Shared, Vec::new(), Some(ty(20)));
        let source = facade_with(&[stats, set_stats, label], &records()).source;
        assert!(source.contains("/// Mirrors `demo::Stats`.\n#[ffi]\npub struct Stats {\n    pub count: u32,\n    pub type_: ffi::String,\n    pub mode: Mode,\n    pub note: ffi::Option<ffi::String>,\n}\n"), "{source}");
        assert!(source.contains("fn to_stats(value: &demo::Stats) -> Result<Stats, runtime::Error> {\n    let demo::Stats { count: field0, r#type: field1, mode: field2, note: field3, .. } = value;\n    Ok(Stats { count: *field0, type_: ffi::String::from_string(field1.to_string()), mode: to_mode(&field2)?, note: ffi::Option::from(field3.as_ref().map(|value| ffi::String::from_string(value.to_string()))) })\n}\n"), "{source}");
        assert!(source.contains("fn from_stats(value: &Stats) -> demo::Stats {\n    let Stats { count: field0, type_: field1, mode: field2, note: field3 } = value;\n    demo::Stats { count: *field0, r#type: field1.as_str().to_owned(), mode: from_mode(field2), note: field3.as_ref().map(|value| value.as_str().to_owned()) }\n}\n"), "{source}");
        assert!(source.contains("pub struct Label {\n    pub text: ffi::String,\n    pub data: ffi::Vec<u8>,\n}\n"), "{source}");
        assert!(source.contains("    let demo::Label { text: field0, data: field1, .. } = value;\n    Ok(Label { text: ffi::String::from_string(field0.to_string()), data: ffi::Vec::from_vec(field1.to_vec()) })\n"), "{source}");
        assert!(!source.contains("fn from_label"), "a borrowed view crosses as a result only: {source}");
        assert!(source.contains("Ok(to_stats(&self.inner.repo().stats())?)"), "{source}");
        assert!(source.contains("self.inner.repo().set_stats(from_stats(&stats))"), "{source}");
    }

    #[test]
    fn string_and_bytes_mappings_convert_through_trait_evidence() {
        let mut types = types();
        for (path, mapping) in [
            ("demo::Name", Mapping::String),
            ("demo::Tag", Mapping::String),
            ("demo::Blob", Mapping::Bytes),
            ("demo::Chunk", Mapping::Bytes),
            ("demo::Opaque", Mapping::String),
            ("other::Ext", Mapping::String),
        ] {
            types.insert(path.to_owned(), mapping);
        }
        let implementations = Implementations::from_pairs([
            (FROM_STR, "demo::Name"),
            (DISPLAY, "demo::Name"),
            (TRY_FROM_STR, "demo::Tag"),
            (FROM_BYTES, "demo::Blob"),
            (AS_REF_BYTES, "demo::Blob"),
            (FROM_BYTE_VEC, "demo::Chunk"),
        ])
        .with_facts([("demo::Chunk", "[u8]")], &[])
        .with_crate("demo");
        let renderer = renderer();
        let classifier = Classifier::new(&renderer, &types, &[], &implementations);
        let context = Context { renderer: &renderer, classifier: &classifier, runtime: "crate::runtime", provenance: "test" };
        let repo = |name: &str, params: Vec<(&str, Type)>, output: Option<Type>| function(&format!("demo::Repo::{name}"), Some("demo::Repo"), Receiver::Shared, params, output);
        let functions = [
            repo("relabel", vec![("name", ty(21)), ("tag", shared(ty(22))), ("fallback", generic(8, vec![ty(21)]))], Some(ty(21))),
            repo("store", vec![("blob", shared(ty(23))), ("chunk", ty(24))], Some(ty(23))),
            repo("chunk", Vec::new(), Some(ty(24))),
            repo("external", vec![("ext", ty(26))], Some(ty(26))),
        ];
        let functions: Vec<&Function> = functions.iter().collect();
        let source = emit(&functions, &context).expect("the functions lower").source;
        assert!(source.contains("self.inner.repo().relabel(name.as_str().parse().map_err(runtime::error)?, &TryFrom::try_from(tag.as_str()).map_err(runtime::error)?, fallback.as_ref().map(|value| value.as_str().parse().map_err(runtime::error)).transpose()?).to_string()"), "{source}");
        assert!(source.contains("Ok(ffi::Vec::from_vec(AsRef::<[u8]>::as_ref(&self.inner.repo().store(&From::from(blob.as_slice()), From::from(chunk.as_slice().to_vec()))).to_vec()))"), "{source}");
        assert!(source.contains("Ok(ffi::Vec::from_vec(self.inner.repo().chunk().to_vec()))"), "a `Deref` to bytes: {source}");
        assert!(source.contains("Ok(ffi::String::from_string(self.inner.repo().external(ext.as_str().parse().map_err(runtime::error)?).to_string()))"), "a crate without rustdoc is assumed to parse: {source}");
        let opaque_in = repo("opaque", vec![("value", ty(25))], None);
        let opaque_out = repo("opaque_out", Vec::new(), Some(ty(25)));
        for (function, missing) in [(opaque_in, "implements neither `FromStr` nor `TryFrom<&str>`"), (opaque_out, "does not implement `Display`")] {
            let verdict = verdict(&function, &context);
            assert_eq!(verdict.reasons[0].category, crate::classify::Category::Lowering, "{verdict:?}");
            assert!(verdict.reasons[0].detail.contains(missing), "{verdict:?}");
        }
    }

    #[test]
    fn enum_parameters_with_data_and_unlowered_payloads_are_decisions() {
        let renderer = renderer();
        let types = types();
        let implementations = enums();
        let classifier = Classifier::new(&renderer, &types, &[], &implementations);
        let context = Context { renderer: &renderer, classifier: &classifier, runtime: "crate::runtime", provenance: "test" };
        let replay = function("demo::Repo::replay", Some("demo::Repo"), Receiver::Shared, vec![("event", ty(17))], None);
        let shape = function("demo::Repo::shape", Some("demo::Repo"), Receiver::Shared, Vec::new(), Some(ty(18)));
        for (function, detail) in [(replay, "carries data"), (shape, "`demo::Shape`, variant `Maybe`")] {
            assert_eq!(classifier.classify(&function).class, Class::Automatic, "the classifier passes `{}`", function.path);
            let verdict = verdict(&function, &context);
            assert_eq!(verdict.reasons[0].category, crate::classify::Category::Lowering, "{verdict:?}");
            assert!(verdict.reasons[0].detail.contains(detail), "{verdict:?}");
        }
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
        assert!(source.contains("            let commit = repo.find_commit(runtime::oid(&commit)?).map_err(runtime::error)?;\n            Ok(ffi::String::from_string(repo.describe(&commit).to_string()))\n"), "keyed parameters reuse the repository they were rebuilt from: {source}");
        assert!(facade.dropped.iter().any(|(path, why)| path == "demo::describe" && why.starts_with("takes a keyed handle")), "{:?}", facade.dropped);
    }

    #[test]
    fn boxes_cows_string_references_and_self_lower() {
        let boxed = function("demo::Repo::boxed", Some("demo::Repo"), Receiver::Shared, vec![("name", generic(13, vec![ty(1)]))], Some(generic(13, vec![ty(9)])));
        let cow = function("demo::Repo::cow", Some("demo::Repo"), Receiver::Shared, vec![("name", generic(14, vec![ty(7)]))], Some(generic(14, vec![ty(7)])));
        let find = function("demo::Repo::find", Some("demo::Repo"), Receiver::Shared, vec![("name", shared(ty(1)))], None);
        let same = function("demo::Commit::same", Some("demo::Commit"), Receiver::Shared, vec![("other", shared(Type::Generic("Self".to_owned())))], Some(Type::Generic("Self".to_owned())));
        let source = facade(&[boxed, cow, find, same]).source;
        assert!(source.contains("Ok(ffi::Vec::from_vec((*self.inner.repo().boxed(Box::new(name.into_string()))).to_vec()))"), "{source}");
        assert!(source.contains("Ok(ffi::Vec::from_vec((&*self.inner.repo().cow(std::borrow::Cow::Borrowed(name.as_slice().into()))).to_vec()))"), "{source}");
        assert!(source.contains("Ok(self.inner.repo().find(&name.into_string()))"), "{source}");
        assert!(source.contains("            let other = repo.find_commit(runtime::oid(&other)?).map_err(runtime::error)?;\n"), "{source}");
        assert!(source.contains("Ok(runtime::oid_out(&(commit_key.same(&other)).id))"), "{source}");
    }

    #[test]
    fn the_verdict_makes_what_the_emitter_cannot_lower_a_decision() {
        let renderer = renderer();
        let types = types();
        let implementations = Implementations::default();
        let classifier = Classifier::new(&renderer, &types, &[], &implementations);
        let context = Context { renderer: &renderer, classifier: &classifier, runtime: "crate::runtime", provenance: "test" };
        let names = function("demo::Repo::names", Some("demo::Repo"), Receiver::Shared, Vec::new(), Some(generic(15, vec![ty(1)])));
        assert_eq!(classifier.classify(&names).class, Class::Automatic, "the classifier passes a Vec of text");
        let verdict = verdict(&names, &context);
        assert_eq!(verdict.class, Class::Decision);
        assert_eq!(verdict.reasons[0].category, crate::classify::Category::Lowering);
        let git_dir = function("demo::Repo::git_dir", Some("demo::Repo"), Receiver::Shared, Vec::new(), Some(shared(ty(2))));
        assert_eq!(verdict_class(&git_dir, &context), Class::Automatic);
    }

    fn verdict_class(function: &Function, context: &Context<'_>) -> Class {
        verdict(function, context).class
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
        assert!(source.contains("    pub fn main(repo: &Self) -> ffi::Result<Self, runtime::Error> {\n"), "{source}");
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
        assert_eq!(rust_path("gix::worktree_admin::move::Options"), "gix::worktree_admin::r#move::Options");
        assert_eq!(snake("ThreadSafeRepository"), "thread_safe_repository");
        assert_eq!(parameter_name("_", 3), "arg3");
    }
}