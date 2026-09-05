use crate::docs::extract_docs;
use crate::skip::has_ffi_skip_attribute;
use crate::types::args::FfiTypeArgs;
use crate::types::discriminant::DiscriminantChoice;
use syn::{Data, DeriveInput, Fields, Generics, Ident, Type, Visibility};

#[derive(Clone)]
pub struct TypeModel {
    pub name: Ident,
    pub generics: Generics,
    pub data: TypeData,
    pub args: FfiTypeArgs,
    pub docs: Vec<String>,
}

#[derive(Clone)]
pub enum TypeData {
    Struct(StructData),
    Enum(EnumData),
}

#[derive(Clone)]
pub struct StructData {
    pub fields: Vec<FieldModel>,
}

#[derive(Clone)]
pub struct EnumData {
    pub variants: Vec<VariantModel>,
    /// The smallest discriminant type that fits all variant values.
    pub discriminant: DiscriminantChoice,
}

#[derive(Clone)]
#[allow(unused)]
pub struct FieldModel {
    pub name: Option<Ident>,
    pub ty: Type,
    pub vis: Visibility,
    pub skip: bool,
    pub docs: Vec<String>,
}

#[derive(Clone)]
pub struct VariantModel {
    pub name: Ident,
    pub data: VariantData,
    pub discriminant: Option<syn::Expr>,
    pub docs: Vec<String>,
}

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub enum VariantData {
    Unit,
    Tuple(Vec<Type>),
    Named(Vec<FieldModel>),
}

impl VariantModel {
    /// Declared payload types in field order, independent of variant shape.
    pub fn payloads(&self) -> impl Iterator<Item = &Type> {
        let tuple = match &self.data {
            VariantData::Tuple(types) => types.as_slice(),
            _ => &[],
        };
        let named = match &self.data {
            VariantData::Named(fields) => fields.as_slice(),
            _ => &[],
        };
        tuple.iter().chain(named.iter().map(|field| &field.ty))
    }
}

impl TypeModel {
    pub fn from_derive_input(input: DeriveInput, args: FfiTypeArgs) -> syn::Result<Self> {
        let docs = extract_docs(&input.attrs);

        let data = match input.data {
            Data::Struct(data_struct) => {
                let fields = match data_struct.fields {
                    Fields::Named(fields) => fields
                        .named
                        .into_iter()
                        .map(|field| {
                            let skip = has_ffi_skip_attribute(&field.attrs);
                            FieldModel { name: field.ident, ty: field.ty, vis: field.vis, skip, docs: extract_docs(&field.attrs) }
                        })
                        .collect(),
                    Fields::Unnamed(fields) => fields
                        .unnamed
                        .into_iter()
                        .map(|field| {
                            let skip = has_ffi_skip_attribute(&field.attrs);
                            FieldModel { name: None, ty: field.ty, vis: field.vis, skip, docs: extract_docs(&field.attrs) }
                        })
                        .collect(),
                    Fields::Unit => vec![],
                };

                TypeData::Struct(StructData { fields })
            }
            Data::Enum(data_enum) => {
                let variants = data_enum
                    .variants
                    .into_iter()
                    .map(|variant| {
                        let data = match variant.fields {
                            Fields::Unit => VariantData::Unit,
                            Fields::Unnamed(fields) => VariantData::Tuple(fields.unnamed.into_iter().map(|field| field.ty).collect()),
                            Fields::Named(fields) => VariantData::Named(fields.named.into_iter().map(|field| {
                                FieldModel { name: field.ident, ty: field.ty, vis: field.vis, skip: false, docs: extract_docs(&field.attrs) }
                            }).collect()),
                        };

                        Ok(VariantModel { name: variant.ident, data, discriminant: variant.discriminant.map(|(_, expr)| expr), docs: extract_docs(&variant.attrs) })
                    })
                    .collect::<syn::Result<Vec<_>>>()?;

                let discriminant = crate::types::discriminant::optimal_discriminant(variants.iter().map(|v| v.discriminant.as_ref()));
                TypeData::Enum(EnumData { variants, discriminant })
            }
            Data::Union(_) => return Err(syn::Error::new_spanned(input, "Unions are not supported")),
        };

        let model = Self { name: input.ident, generics: input.generics, data, args, docs };

        Ok(model)
    }
}
