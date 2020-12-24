use syn::parse::{Parse, ParseStream, Result, Error, ParseBuffer};
use syn::{parenthesized, braced, Ident, Token, LitInt};
use syn::punctuated::Punctuated;
use proc_macro2::TokenStream;
use std::collections::HashMap;
use super::*;

mod attr_kw {
    syn::custom_keyword!(fields);
    syn::custom_keyword!(fields32);
    syn::custom_keyword!(fields64);
}

#[derive(Debug)]
struct Csr {
    name: Ident,
    attrs: Punctuated<CsrAttr, Token![,]>,
}


impl Parse for Csr {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;
        let content: ParseBuffer;
        braced!(content in input);
        Ok(Csr {
            name: name,
            attrs: content.parse_terminated(CsrAttr::parse)?,
        })
    }
}

type AttrPunctuated = Punctuated<Field, Token![;]>;

#[derive(Debug)]
struct Attr<K> {
    key: K,
    attrs: AttrPunctuated,
}

impl<K> Attr<K> {
    fn new(key: K, attrs: AttrPunctuated) -> Attr<K> {
        Attr { key, attrs }
    }
}


#[derive(Debug)]
enum CsrAttr {
    Fields(Attr<attr_kw::fields>),
    Fields32(Attr<attr_kw::fields32>),
    Fields64(Attr<attr_kw::fields64>),
}

macro_rules! parse_attr {
    ( $stream: ident, $key: path, $rt: path) => {
        || {
            let span = $stream.span();
            $stream.parse::<$key>()?;
            let content;
            syn::braced !(content in $ stream);
            Ok($rt(Attr::new($key(span), content.parse_terminated( <Field>::parse)?)))
        }
    };
}

impl Parse for CsrAttr {
    fn parse(input: ParseStream) -> Result<Self> {
        let lookahead = input.lookahead1();
        if lookahead.peek(attr_kw::fields) {
            parse_attr!(input, attr_kw::fields, CsrAttr::Fields)()
        } else if lookahead.peek(attr_kw::fields32) {
            parse_attr!(input, attr_kw::fields32, CsrAttr::Fields32)()
        } else if lookahead.peek(attr_kw::fields64) {
            parse_attr!(input, attr_kw::fields64, CsrAttr::Fields64)()
        } else {
            Err(lookahead.error())
        }
    }
}

#[derive(Debug)]
struct Field {
    name: Ident,
    msb: LitInt,
    lsb: LitInt,
    privilege: CsrPrivilege,
}

impl Field {
    fn range(&self) -> (usize, usize) {
        (self.msb.base10_parse().unwrap(), self.lsb.base10_parse().unwrap())
    }

    fn same_name(&self, rhs: &Self) -> bool {
        self.name.to_string() == rhs.name.to_string()
    }

    fn overlap(&self, rhs: &Self) -> bool {
        let ((msb, lsb), (rmsb, rlsb)) = (self.range(), rhs.range());
        msb >= rlsb && msb <= rmsb || lsb >= rlsb && lsb <= rmsb || rmsb >= lsb && rmsb <= msb || rlsb >= lsb && rlsb <= msb
    }

    fn setter_name(&self) -> Ident {
        format_ident!("set_{}", self.name)
    }

    fn getter_name(&self) -> Ident {
        self.name.clone()
    }
}

impl Parse for Field {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;
        let content: ParseBuffer;
        parenthesized!(content in input);
        let privilege = content.call(CsrPrivilege::parse)?;
        input.parse::<Token![:]>()?;

        let msb: LitInt = input.parse()?;
        input.parse::<Token![,]>()?;
        let lsb: LitInt = input.parse()?;

        if msb.base10_parse::<usize>()? < lsb.base10_parse::<usize>()? {
            return Err(Error::new(msb.span(), format!("msb {} is smaller than lsb {} !", msb.to_string(), lsb.to_string())));
        }

        Ok(Field {
            name,
            msb,
            lsb,
            privilege,
        })
    }
}

struct Fields<'a> {
    name: Ident,
    size: usize,
    fields: Vec<&'a Field>,
}

impl<'a> Fields<'a> {
    fn new(name: Ident, size: usize) -> Self {
        Fields {
            name,
            size: size,
            fields: vec![],
        }
    }

    fn overflow(&self, field: &Field) -> bool {
        let (msb, lsb) = field.range();
        msb >= self.size || lsb >= self.size
    }

    fn add(&mut self, field: &'a Field) -> Result<()> {
        if self.overflow(field) {
            Err(Error::new(field.name.span(), format!("field {}{:?} overflow!", field.name.to_string(), field.range())))
        } else {
            for prev in self.fields.iter() {
                if field.same_name(prev) {
                    return Err(Error::new(field.name.span(), format!("field {} is redefined!", field.name.to_string())));
                }
                if field.overlap(prev) {
                    return Err(Error::new(field.name.span(), format!("field {}{:?} is overlapped with field {}{:?}!", field.name.to_string(), field.range(), prev.name.to_string(), prev.range())));
                }
            }
            Ok(self.fields.push(field))
        }
    }

    fn default_field(&self, id: &Ident) -> Field {
        Field {
            name: id.clone(),
            msb: LitInt::new(&format!("{}", self.size - 1), id.span()),
            lsb: LitInt::new("0", id.span()),
            privilege: CsrPrivilege::RW(privilege_kw::RW(id.span())),
        }
    }

    fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    fn struct_name(&self) -> Ident {
        format_ident!("{}{}", self.name.to_string(),self.size)
    }

    fn struct_expand(&self, trait_name: &Ident, transforms_name: &Ident) -> TokenStream {
        let size = format_ident!("u{}", self.size);

        let fields = quote_map_fold(self.fields.iter(), |field| {
            let (setter, getter) = (field.setter_name(), field.getter_name());
            let (msb, lsb) = (&field.msb, &field.lsb);
            quote! {
                    #[inline(always)]
                    #getter, #setter: #msb, #lsb;
                }
        });

        let fns = quote_map_fold(self.fields.iter(), |field| {
            let (setter, getter) = (field.setter_name(), field.getter_name());
            let (setter_with_trans, getter_with_trans) = (format_ident!("{}_with_trans", field.setter_name()), format_ident!("{}_with_trans",field.getter_name()));
            let (setter_transform, getter_transform) = (format_ident!("{}_transform", field.setter_name()), format_ident!("{}_transform",field.getter_name()));
            quote! {
                fn #getter_with_trans(&self, t:&#transforms_name) -> u64 {
                    let value = self.#getter();
                    if let Some(ref f) = t.#getter_transform {
                        (*f)(value)
                    } else {
                        value
                    }
                }
                fn #setter_with_trans(&mut self, value:u64, t:&#transforms_name) {
                    let v = if let Some(ref f) = t.#setter_transform {
                        (*f)(value)
                    } else {
                        value
                    };
                    self.#setter(v);
                }
            }
        });

        let set = quote_map_fold(self.fields.iter().filter(|field| { field.privilege.writeable() }), |field| {
            let lsb = &field.lsb;
            let setter_with_trans = format_ident!("{}_with_trans", field.setter_name());
            quote! {
                self.#setter_with_trans(value >> (#lsb as u64), t);
            }
        });
        let get = self.fields.iter()
            .filter(|field| { field.privilege.readable() })
            .map(|field| {
                let lsb = &field.lsb;
                let getter_with_trans = format_ident!("{}_with_trans", field.getter_name());
                quote! {
                    (self.#getter_with_trans(t) << (#lsb as u64))
                }
            })
            .fold(quote! {(0 as u64)}, |acc, q| {
                quote! {
                    #acc | #q
                }
            });
        let struct_name = self.struct_name();
        quote! {
            #[derive(Copy, Clone)]
            struct #struct_name(#size);
            impl BitRange<u64> for #struct_name {
                fn bit_range(&self, msb: usize, lsb: usize) -> u64 {
                    let width = msb - lsb + 1;
                    if width == (std::mem::size_of::<#size>() << 3) {
                        self.0 as u64
                    } else {
                        let mask:#size = ((1 as #size) << (width as #size)) - 1;
                        ((self.0 >> (lsb as #size)) & mask) as u64
                    }
                }

                fn set_bit_range(&mut self, msb: usize, lsb: usize, value: u64) {
                    let width = msb - lsb + 1;
                    let bitlen = (std::mem::size_of::<#size>() << 3);
                    if width == bitlen {
                        self.0 = value as #size
                    } else {
                        let low = self.0 & (((1 as #size) << (lsb as #size)) - 1);
                        let high = if msb == bitlen - 1 {0} else {(self.0 >> ((msb + 1) as #size)) << ((msb + 1) as #size)};
                        let mask:#size = ((1 as #size) << (width as #size)) - 1;
                        self.0 = high | low | (((value as #size) & mask) << (lsb as #size));
                    }
                }
            }
            impl #struct_name {
                fn get(&self, t:&#transforms_name) -> u64 {
                   #get
                }
                fn set(&mut self, value:u64, t:&#transforms_name) {
                    #set
                }
                bitfield_fields! {
                    u64;
                    #fields
                }
            }
            impl #trait_name for #struct_name {
                #fns
            }
        }
    }
}

struct FieldSet<'a> {
    name: Ident,
    field_names: HashMap<String, &'a Field>,
}

impl<'a> FieldSet<'a> {
    fn new(name: Ident) -> Self {
        FieldSet { name, field_names: HashMap::new() }
    }

    fn add(&mut self, field: &'a Field) {
        self.field_names.insert(field.name.to_string(), field);
    }

    fn trait_name(&self) -> Ident {
        format_ident!("{}Trait", self.name.to_string())
    }


    fn transforms_name(&self) -> Ident {
        format_ident!("{}Transforms", self.name.to_string())
    }

    fn trait_expand(&self) -> TokenStream {
        let transforms_name = self.transforms_name();
        let fns = quote_map_fold(self.field_names.values(), |field| {
            let (setter, getter) = (field.setter_name(), field.getter_name());
            let (setter_with_trans, getter_with_trans) = (format_ident!("{}_with_trans", field.setter_name()), format_ident!("{}_with_trans",field.getter_name()));
            let getter_msg = format!("{} not implement {} in current xlen setting!", self.name.to_string(), getter.to_string());
            let setter_msg = format!("{} not implement {} in current xlen setting!", self.name.to_string(), setter.to_string());
            quote! {
                fn #getter_with_trans(&self, t:&#transforms_name) -> u64 { panic!(#getter_msg)}
                fn #setter_with_trans(&mut self, value:u64, t:&#transforms_name) { panic!(#setter_msg)}
            }
        });
        let trait_name = self.trait_name();
        quote! {
            trait #trait_name {
                #fns
            }
        }
    }

    fn transforms_expand(&self) -> TokenStream {
        let transforms_name = self.transforms_name();
        let transforms = quote_map_fold(self.field_names.values(), |field| {
            let (setter_transform, getter_transform) = (format_ident!("{}_transform", field.setter_name()), format_ident!("{}_transform",field.getter_name()));
            quote! {
                #getter_transform:Option<Box<dyn Fn(u64)->u64>>,
                #setter_transform:Option<Box<dyn Fn(u64)->u64>>,
            }
        });

        let transform_inits = quote_map_fold(self.field_names.values(), |field| {
            let (setter_transform, getter_transform) = (format_ident!("{}_transform", field.setter_name()), format_ident!("{}_transform",field.getter_name()));
            quote! {
                #getter_transform:None,
                #setter_transform:None,
            }
        });

        let transform_fns = quote_map_fold(self.field_names.values(), |field| {
            let (setter_transform, getter_transform) = (format_ident!("{}_transform", field.setter_name()), format_ident!("{}_transform",field.getter_name()));
            quote! {
                pub fn #setter_transform<F:Fn(u64)->u64 +'static>(&mut self, f:F) {
                    self.#setter_transform = Some(Box::new(f))
                }
                pub fn #getter_transform<F:Fn(u64)->u64 +'static>(&mut self, f:F) {
                    self.#getter_transform = Some(Box::new(f))
                }
            }
        });
        quote! {
            struct #transforms_name {
                #transforms
            }

            impl #transforms_name {
                fn new() -> #transforms_name {
                    #transforms_name{
                        #transform_inits
                    }
                }
                #transform_fns
            }
        }
    }

    fn top_expand(&self, struct32_name: &Ident, struct64_name: &Ident) -> TokenStream {
        let union_name = format_ident!("{}Union", self.name.to_string());
        let union_target = quote! {
            union #union_name {
                x32: #struct32_name,
                x64: #struct64_name,
            }
        };

        let top_name = &self.name;
        let transforms_name = self.transforms_name();
        let transform_fns = quote_map_fold(self.field_names.values(), |field| {
            let (setter_transform, getter_transform) = (format_ident!("{}_transform", field.setter_name()), format_ident!("{}_transform",field.getter_name()));
            quote! {
                pub fn #setter_transform<F:Fn(u64)->u64 +'static>(&mut self, f:F) {
                    self.transforms.#setter_transform(f)
                }
                pub fn #getter_transform<F:Fn(u64)->u64 +'static>(&mut self, f:F) {
                    self.transforms.#getter_transform(f)
                }
            }
        });

        let fns = quote_map_fold(self.field_names.values(), |field| {
            let (setter, getter) = (field.setter_name(), field.getter_name());
            let (setter_with_trans, getter_with_trans) = (format_ident!("{}_with_trans", field.setter_name()), format_ident!("{}_with_trans",field.getter_name()));
            quote! {
                pub fn #getter(&self) -> u64 {
                    match self.xlen {
                        64 => unsafe { self.csr.x64.#getter_with_trans(&self.transforms) },
                        32 => unsafe { self.csr.x32.#getter_with_trans(&self.transforms) },
                        _ => unreachable!()
                    }
                }
                pub fn #setter(&mut self, value:u64) {
                    match self.xlen {
                        64 => unsafe { self.csr.x64.#setter_with_trans(value, &self.transforms) },
                        32 => unsafe { self.csr.x32.#setter_with_trans(value, &self.transforms) },
                        _ => unreachable!()
                    }
                }
            }
        });
        quote! {
            #union_target
            pub struct #top_name {
                pub xlen:usize,
                csr:#union_name,
                transforms:#transforms_name
            }

            impl #top_name {
                pub fn new(xlen:usize, init:u64) -> #top_name {
                    #top_name{
                        xlen,
                        csr:#union_name{x64:{#struct64_name(init)}},
                        transforms:#transforms_name::new()
                    }
                }
                #transform_fns
                pub fn get(&self) -> u64 {
                    match self.xlen {
                        64 => unsafe { self.csr.x64.get(&self.transforms) },
                        32 => unsafe { self.csr.x32.get(&self.transforms) },
                        _ => unreachable!()
                    }
                }
                pub fn set(&mut self, value:u64) {
                    match self.xlen {
                        64 => unsafe { self.csr.x64.set(value, &self.transforms) },
                        32 => unsafe { self.csr.x32.set(value, &self.transforms) },
                        _ => unreachable!()
                    }
                }
                #fns
            }
        }
    }
}


macro_rules! get_attr {
    ($attrs: expr, $exp: path) => {
        || {
            let _attr = $attrs.iter().filter_map(|f| {
                if let $exp(a) = f {
                    Some(a)
                } else {
                    None
                }
            }).collect::<Vec<_>>();
            if _attr.len() == 0 {
                Ok(None)
            } else if _attr.len() == 1 {
                Ok(Some(_attr[0]))
            } else {
                Err(Error::new(_attr[1].key.span, format!("{:?} is redefined!", _attr[1].key)))
            }

        }
    };
}

pub fn expand(input: TokenStream) -> TokenStream {
    let csr: Csr = expand_call!(syn::parse2(input));
    let fields = expand_call!(get_attr!(csr.attrs, CsrAttr::Fields)());
    let fields32 = expand_call!(get_attr!(csr.attrs, CsrAttr::Fields32)());
    let fields64 = expand_call!(get_attr!(csr.attrs, CsrAttr::Fields64)());

    let mut field32s = Fields::new(csr.name.clone(), 32);
    let mut field64s = Fields::new(csr.name.clone(), 64);
    let mut field_set = FieldSet::new(csr.name.clone());
    if let Some(Attr { key: _, attrs }) = fields {
        for field in attrs {
            expand_call!(field32s.add(field));
            expand_call!(field64s.add(field));
            field_set.add(field);
        }
    }
    if let Some(Attr { key: _, attrs }) = fields32 {
        for field in attrs {
            expand_call!(field32s.add(field));
            field_set.add(field);
        }
    }
    if let Some(Attr { key: _, attrs }) = fields64 {
        for field in attrs {
            expand_call!(field64s.add(field));
            field_set.add(field);
        }
    }
    let default_id = Ident::new(&csr.name.to_string().to_lowercase(), csr.name.span());
    let defalut_field32 = field32s.default_field(&default_id);
    let defalut_field64 = field64s.default_field(&default_id);
    if field32s.is_empty() {
        expand_call!(field32s.add(&defalut_field32));
        field_set.add(&defalut_field32);
    }
    if field64s.is_empty() {
        expand_call!(field64s.add(&defalut_field64));
        field_set.add(&defalut_field64);
    }

    let trait_name = field_set.trait_name();
    let trait_target = field_set.trait_expand();
    let transforms_name = field_set.transforms_name();
    let transforms_target = field_set.transforms_expand();
    let struct32_target = field32s.struct_expand(&trait_name, &transforms_name);
    let struct64_target = field64s.struct_expand(&trait_name, &transforms_name);
    let top_target = field_set.top_expand(&field32s.struct_name(), &field64s.struct_name());

    quote! {
        #trait_target
        #transforms_target
        #struct32_target
        #struct64_target
        #top_target
    }
}




