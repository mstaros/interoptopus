#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Primitive {
    Void,
    Bool,
    Byte,   // U8
    UShort, // U16
    UInt,   // U32
    ULong,  // U64
    NUInt,  // Usize -> nuint
    SByte,  // I8
    Short,  // I16
    Int,    // I32
    Long,   // I64
    NInt,   // Isize -> nint
    Float,  // F32
    Double, // F64
}

impl Primitive {
    /// Whether C# permits this as the underlying type of an `enum`.
    ///
    /// The language allows exactly the eight integral types below. `nint`/`nuint` are excluded
    /// deliberately — integral, but not legal enum bases — as are `void`, `bool`, `float` and
    /// `double`.
    ///
    /// This is a fact about C#, not about projection, which is why it lives here. It is the
    /// second half of the plain-enum rule: a `DataEnum` becomes a plain C# `enum` only when no
    /// variant can carry a payload **and** its discriminant is one of these. A unit-only enum
    /// whose discriminant is anything else stays a struct.
    #[must_use]
    pub fn is_csharp_enum_underlying(self) -> bool {
        matches!(self, Self::Byte | Self::SByte | Self::Short | Self::UShort | Self::Int | Self::UInt | Self::Long | Self::ULong)
    }

    /// Returns the C# keyword for this primitive type.
    #[must_use]
    pub fn cs_name(self) -> &'static str {
        match self {
            Self::Void => "void",
            Self::Bool => "bool",
            Self::Byte => "byte",
            Self::UShort => "ushort",
            Self::UInt => "uint",
            Self::ULong => "ulong",
            Self::NUInt => "nuint",
            Self::SByte => "sbyte",
            Self::Short => "short",
            Self::Int => "int",
            Self::Long => "long",
            Self::NInt => "nint",
            Self::Float => "float",
            Self::Double => "double",
        }
    }
}
