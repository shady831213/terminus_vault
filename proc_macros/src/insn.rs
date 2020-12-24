use syn::{DeriveInput, DataStruct, Ident, Result, NestedMeta, LitStr};
use syn::parse::Error;
use proc_macro2::Span;
use regex::Regex;

lazy_static! {
static ref VALID_FORMAT_TYPE:Vec<&'static str> = vec![
    "USER_DEFINE",
    "R",
    "I",
    "S",
    "B",
    "U",
    "J",
    "CR",
    "CIW",
    "CI",
    "CSS",
    "CL",
    "CS",
    "CB",
    "CA",
    "CJ",
];
}



pub fn expand(ast: &DeriveInput, name: &Ident) -> Result<proc_macro2::TokenStream> {
    if let syn::Data::Struct(data) = &ast.data {
        let code_str = parse_code_attr(ast, "code")?;
        let code = parse_code_value(&code_str);
        let mask = parse_mask_value(&code_str);
        let format = parse_format_attr(ast)?;
        let decoder_ident = format_ident!("{}Decoder", name);
        let registery_ident = format_ident!("REGISTERY_{}", Ident::new(&name.to_string().to_uppercase(), name.span()));
        let name_string = name.to_string();
        check_fields(data, name)?;
        Ok(quote!(
            insn_format!(#name, #format);
            impl #name {
                fn new() -> Instruction {
                    Instruction::new(#name())
                }
            }
            impl InstructionImp for #name{}

            struct #decoder_ident(Instruction, TerminusInsnT, TerminusInsnT);
            impl Decoder for #decoder_ident {
                fn code(&self) ->  TerminusInsnT {
                    self.1
                }
                fn mask(&self) ->  TerminusInsnT {
                    self.2
                }
                fn matched(&self, ir:&TerminusInsnT) -> bool {
                    *ir & self.mask() == self.code()
                }
                fn decode(&self) -> &Instruction {
                    &self.0
                }
                fn name(&self) -> String{
                    #name_string.to_string()
                }
            }

            #[distributed_slice(REGISTERY_INSN)]
            static #registery_ident: fn(&mut GlobalInsnMap) = |map| {map.registery(#decoder_ident(#name::new(),
                TerminusInsnT::from_str_radix(#code, 2).unwrap(),
                TerminusInsnT::from_str_radix(#mask, 2).unwrap()
                ))};
        ))
    } else {
        Err(Error::new(name.span(), "Only Struct can derive"))
    }
}

fn check_fields(data: &DataStruct, name: &Ident) -> Result<()> {
    let msg = format!("expect \'struct {}();\' !", name.to_string());
    if let syn::Fields::Unnamed(ref field) = data.fields {
        if field.unnamed.len() != 0 {
            return Err(Error::new(field.paren_token.span, msg));
        } else {
            Ok(())
        }
    } else {
        Err(Error::new(name.span(), msg))
    }
}

fn parse_code_attr(ast: &DeriveInput, name: &str) -> Result<String> {
    let Attr { ident, attr } = parse_attr(ast, name)?;
    if let NestedMeta::Lit(syn::Lit::Str(ref raw)) = attr {
        parse_raw_bits(raw)
    } else {
        Err(Error::new(ident.span(), format!("\"{}\" is expected as string with \"0b\" prefix!", name)))
    }
}

fn parse_raw_bits(lit: &LitStr) -> Result<String> {
    let code = lit.value();
    lazy_static! {
        static ref VALID_CODE: Regex = Regex::new("^([0-9]+)b([10?_]+)$").unwrap();
        static ref BITS_REP: Regex = Regex::new("_").unwrap();
    }
    if let Some(caps) = VALID_CODE.captures(&code) {
        let len = usize::from_str_radix(&caps[1], 10).map_err(|e|{Error::new(lit.span(),e.to_string())})?;
        if len == 0 {
            return Err(Error::new(lit.span(), "length of code can not be zero!"));
        }
        let bits = BITS_REP.replace_all(&caps[2], "");
        let valid_bits = Regex::new(&("^[10?]{1,".to_string() + &format!("{}", len) + "}$")).unwrap();
        if !valid_bits.is_match(&bits) {
            return Err(Error::new(lit.span(), format!("code defined num of bits more than {}!", len)));
        }
        if bits.len() < len {
            Ok(ext_bits(&bits, len))
        } else {
            Ok(bits.to_string())
        }
    } else {
        return Err(Error::new(lit.span(), "code contains invalid char, valid format is ^[0-9]+b[1|0|?|_]+!"));
    }
}

fn ext_bits(bits: &str, cap: usize) -> String {
    if bits.len() == cap {
        bits.to_string()
    } else {
        ext_bits(&("?".to_owned() + bits), cap)
    }
}

fn parse_code_value(bits: &str) -> String {
    lazy_static! {
        static ref QUE: Regex = Regex::new("[?]").unwrap();
    }
    QUE.replace_all(bits, "0").to_string()
}

fn parse_mask_value(bits: &str) -> String {
    lazy_static! {
        static ref ZERO: Regex = Regex::new("0").unwrap();
    }
    parse_code_value(&ZERO.replace_all(bits, "1"))
}

fn parse_format_attr(ast: &DeriveInput) -> Result<Ident> {
    let Attr { ident, attr } = parse_attr(ast, "format")?;
    if let NestedMeta::Meta(syn::Meta::Path(ref path)) = attr {
        if let Some(ident) = path.get_ident() {
            if VALID_FORMAT_TYPE.contains(&&format!("{}", ident)[..]) {
                Ok(ident.clone())
            } else {
                Err(Error::new(ident.span(), format!("invalid \"{}\" value \"{}\", valid values are {:?}", "format", ident, *VALID_FORMAT_TYPE)))
            }
        } else {
            Err(Error::new(ident.span(), format!("\"{}\" is expected as Ident", "format")))
        }
    } else {
        Err(Error::new(ident.span(), format!("\"{}\" is expected as Ident", "format")))
    }
}

struct Attr {
    ident: Ident,
    attr: NestedMeta,
}

impl Attr {
    fn new(ident: Ident, attr: NestedMeta) -> Self {
        Attr { ident, attr }
    }
}

fn parse_attr(ast: &DeriveInput, name: &str) -> Result<Attr> {
    if let Some(attr) = ast.attrs.iter().find(|a| { a.path.segments.len() == 1 && a.path.segments[0].ident == name }) {
        let meta = attr.parse_meta()?;
        if let syn::Meta::List(ref nested_meta) = meta {
            if nested_meta.nested.len() == 1 {
                Ok(Attr::new(attr.path.segments[0].ident.clone(), nested_meta.nested[0].clone()))
            } else {
                Err(Error::new(attr.path.segments[0].ident.span(), format!("\"{}\" is expected to be a single value", name)))
            }
        } else {
            Err(Error::new(attr.path.segments[0].ident.span(), format!("\"{}\" is expected to be a single value", name)))
        }
    } else {
        Err(Error::new(Span::call_site(), format!("attr \"{}\" missed", name)))
    }
}

