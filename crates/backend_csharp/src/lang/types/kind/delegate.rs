use crate::lang::functions::Signature;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DelegateKind {
    Class,
    Signature,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Delegate {
    pub kind: DelegateKind,
    pub signature: Signature,
}

impl Delegate {
    /// Standard managed delegate for a named callback, when its signature fits .NET's
    /// Func/Action families. The native callback representation is unaffected.
    pub(crate) fn standard_name<'a>(&self, resolve: impl Fn(crate::lang::TypeId) -> Option<&'a crate::lang::types::Type>) -> Option<String> {
        use crate::lang::types::kind::{PointerKind, Primitive, TypeKind, TypePattern};

        fn generic_name<'a>(id: crate::lang::TypeId, resolve: &dyn Fn(crate::lang::TypeId) -> Option<&'a crate::lang::types::Type>) -> Option<String> {
            let ty = resolve(id)?;
            match &ty.kind {
                TypeKind::Primitive(Primitive::Void) | TypeKind::TypePattern(TypePattern::CVoid) => None,
                TypeKind::Pointer(p) if !matches!(p.kind, PointerKind::IntPtr(_)) => None,
                TypeKind::Primitive(Primitive::Bool) | TypeKind::TypePattern(TypePattern::Bool) => Some("bool".to_string()),
                _ => Some(crate::lang::types::tuple::name(id, resolve).unwrap_or_else(|| ty.name.clone())),
            }
        }

        if self.signature.arguments.len() > 16 { return None; }
        let rval = resolve(self.signature.rval)?;
        let is_void = matches!(rval.kind, TypeKind::Primitive(Primitive::Void) | TypeKind::TypePattern(TypePattern::CVoid));
        let mut arguments = self.signature.arguments.iter()
            .map(|arg| generic_name(arg.ty, &resolve)).collect::<Option<Vec<_>>>()?;
        if !is_void { arguments.push(generic_name(self.signature.rval, &resolve)?); }
        let family = if is_void { "Action" } else { "Func" };
        if arguments.is_empty() {
            Some(format!("global::System.{family}"))
        } else {
            Some(format!("global::System.{family}<{}>", arguments.join(", ")))
        }
    }
}

