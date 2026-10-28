//! Names interoptopus rejects because some backend language reserves them. The list is a copy of
//! `proc_macros_impl::forbidden::FORBIDDEN_NAMES`: a proc-macro crate cannot export it, and the
//! engine must stay packageable on its own. A test keeps the two equal.

/// Identifiers interoptopus rejects in generated code.
const FORBIDDEN_NAMES: [&str; 136] = [
    "abstract", "add", "alias", "allows", "and", "args", "as", "ascending", "assert", "async", "await", "base", "bool", "break", "by",
    "byte", "case", "catch", "char", "checked", "class", "const", "continue", "decimal", "def", "default", "del", "delegate",
    "descending", "do", "double", "dynamic", "elif", "else", "enum", "equals", "event", "except", "explicit", "extension", "extern",
    "false", "False", "field", "finally", "fixed", "float", "for", "foreach", "from", "global", "goto", "group", "if", "implicit",
    "import", "in", "init", "int", "interface", "internal", "into", "is", "join", "lambda", "let", "lock", "long", "managed", "nameof",
    "namespace", "new", "nint", "nonlocal", "not", "notnull", "nuint", "null", "object", "on", "operator", "or", "orderby", "out",
    "override", "params", "partial", "pass", "private", "protected", "public", "raise", "readonly", "record", "ref", "remove",
    "required", "return", "sbyte", "scoped", "sealed", "select", "set", "short", "signed", "sizeof", "stackalloc", "static", "string",
    "struct", "switch", "this", "throw", "true", "True", "try", "typedef", "typeof", "uint", "ulong", "unchecked", "unmanaged",
    "unsafe", "unsigned", "ushort", "using", "value", "var", "virtual", "void", "volatile", "when", "where", "while", "with", "yield",
];

/// Whether interoptopus forbids `name`; the list also covers the C# keywords.
pub fn is_forbidden(name: &str) -> bool {
    FORBIDDEN_NAMES.contains(&name)
}

/// `name`, with a trailing underscore when interoptopus forbids it (`init` becomes `init_`).
pub fn escaped(name: &str) -> String {
    if is_forbidden(name) { format!("{name}_") } else { name.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn the_copy_matches_interoptopus() {
        let source = include_str!("../../proc_macros_impl/src/forbidden.rs");
        let list = source.split_once("= [").and_then(|(_, rest)| rest.split_once("];")).map(|(list, _)| list).expect("the list is found");
        let theirs: BTreeSet<&str> = list.split('"').skip(1).step_by(2).collect();
        let ours: BTreeSet<&str> = FORBIDDEN_NAMES.into_iter().collect();
        assert_eq!(ours, theirs);
    }

    #[test]
    fn only_forbidden_names_are_escaped() {
        assert_eq!(escaped("init"), "init_");
        assert_eq!(escaped("is_bare"), "is_bare");
    }
}