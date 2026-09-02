mod args;
mod discriminant;
mod emit;
mod model;
#[cfg(test)]
mod tests;
mod validation;
mod wireio;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse2};

use crate::skip::is_ffi_skip_attribute;

use args::FfiTypeArgs;
use model::TypeModel;

pub fn ffi(attr: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let args: FfiTypeArgs = parse2(attr)?;
    let mut input_ast: DeriveInput = parse2(input)?;

    // Parse the model BEFORE removing skip attributes so it can detect them
    let model = TypeModel::from_derive_input(input_ast.clone(), args.clone())?;

    // Validate
    args.validate()?;
    model.validate()?;

    // Add repr attributes and remove skip attributes
    reject_declared_repr(&input_ast, &model)?;

    add_repr_attribute(&mut input_ast, &model);
    remove_skip_attributes(&mut input_ast);

    let typeinfo_impl = model.emit_typeinfo_impl()?;
    let wireio_impl = model.emit_wireio_impl();

    let result = quote! {
        #input_ast
        #typeinfo_impl
        #wireio_impl
    };

    if args.debug {
        let formatted = prettyplease::unparse(&syn::parse2(result.clone()).unwrap());
        eprintln!("Generated code for {}:\n{}", model.name, formatted);
    }

    Ok(result)
}

/// Refuse a source-declared `#[repr(...)]` on a non-service type.
///
/// The macro owns this attribute. For enums it picks the discriminant width
/// (see [`discriminant::optimal_discriminant`]) and for structs it enforces a
/// known layout, and that same choice is what the inventory reports to a
/// backend. Honouring a declared `repr` would let the Rust type and the
/// generated binding disagree about the width of a value.
///
/// This was previously a silent `retain` that deleted the attribute, which left
/// nothing to explain the result: a caller who wrote `#[repr(u32)]` saw `: byte`
/// in the generated C# and had no way to connect the two.
fn reject_declared_repr(input: &DeriveInput, model: &TypeModel) -> syn::Result<()> {
    if model.args.service {
        return Ok(());
    }

    match input.attrs.iter().find(|attr| attr.path().is_ident("repr")) {
        Some(attr) => Err(syn::Error::new_spanned(
            attr,
            "`#[ffi]` chooses the representation itself and would discard this attribute. \
             Remove it: enums receive the narrowest discriminant their variants fit in, \
             structs receive `#[repr(C)]`, and `#[ffi(opaque)]`, `#[ffi(transparent)]` \
             and `#[ffi(packed)]` select the alternatives.",
        )),
        None => Ok(()),
    }
}

fn add_repr_attribute(input: &mut DeriveInput, model: &TypeModel) {
    if model.args.service {
        return;
    }

    // Defensive only: `reject_declared_repr` has already refused a declared repr, so
    // this finds nothing. Enums get the optimal discriminant size, structs a known layout.
    input.attrs.retain(|attr| !attr.path().is_ident("repr"));

    let repr_attr = if model.args.opaque {
        syn::parse_quote! { #[repr(C)] }
    } else if model.args.transparent {
        syn::parse_quote! { #[repr(transparent)] }
    } else if model.args.packed {
        syn::parse_quote! { #[repr(C, packed)] }
    } else {
        match &model.data {
            model::TypeData::Struct(_) => syn::parse_quote! { #[repr(C)] },
            model::TypeData::Enum(enum_data) => discriminant::repr_attribute(&enum_data.discriminant),
        }
    };

    input.attrs.push(repr_attr);
}

fn remove_skip_attributes(input: &mut DeriveInput) {
    use syn::Data;

    match &mut input.data {
        Data::Struct(data_struct) => match &mut data_struct.fields {
            syn::Fields::Named(fields) => {
                for field in &mut fields.named {
                    field.attrs.retain(|attr| !is_ffi_skip_attribute(attr));
                }
            }
            syn::Fields::Unnamed(fields) => {
                for field in &mut fields.unnamed {
                    field.attrs.retain(|attr| !is_ffi_skip_attribute(attr));
                }
            }
            syn::Fields::Unit => {}
        },
        Data::Enum(data_enum) => {
            for variant in &mut data_enum.variants {
                variant.attrs.retain(|attr| !is_ffi_skip_attribute(attr));
                match &mut variant.fields {
                    syn::Fields::Named(fields) => {
                        for field in &mut fields.named {
                            field.attrs.retain(|attr| !is_ffi_skip_attribute(attr));
                        }
                    }
                    syn::Fields::Unnamed(fields) => {
                        for field in &mut fields.unnamed {
                            field.attrs.retain(|attr| !is_ffi_skip_attribute(attr));
                        }
                    }
                    syn::Fields::Unit => {}
                }
            }
        }
        Data::Union(_) => {}
    }
}
