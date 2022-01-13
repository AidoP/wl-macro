use std::{
    fs::File,
    io::Read,
    path::Path, collections::HashMap,
};
use crate::Binding;
use heck::{CamelCase, SnakeCase};
use proc_macro2::TokenStream;
use serde::Deserialize;
use quote::{quote, format_ident};
use syn::{parse_quote, spanned::Spanned};

// Note: owned strings are required as TOML allows string normalisation

#[derive(Debug, Deserialize)]
pub struct Protocol {
    pub name: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub copyright: Option<String>,
    #[serde(rename = "interface", default)]
    pub interfaces: Vec<Interface>
}
impl Protocol {
    pub fn from_str(string: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(string)
    }
    pub fn load<P: AsRef<Path>>(path: P) -> Self {
        let path = path.as_ref();
        let mut protocol = String::new();
        let mut file = File::open(path).unwrap_or_else(|error| panic!("Unable to open protocol specification file {:?}: {:?}", path, error));
        file.read_to_string(&mut protocol).unwrap_or_else(|error| panic!("Unable to read protocol specification file {:?}: {:?}", path, error));
        Self::from_str(&protocol).unwrap_or_else(|error| panic!("Failed to parse protocol specification file {:?}: {:?}", path, error))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Interface {
    pub name: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub version: u32,
    #[serde(rename = "enum", default)]
    pub enums: Vec<Enum>,
    #[serde(rename = "request", default)]
    pub requests: Vec<Request>,
    #[serde(rename = "event", default)]
    pub events: Vec<Event>
}

#[derive(Clone, Debug, Deserialize)]
pub struct Enum {
    pub name: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub since: Option<u32>,
    #[serde(rename = "entry", default)]
    pub entries: Vec<Entry>
}
#[derive(Clone, Debug, Deserialize)]
pub struct Request {
    pub name: String,
    pub since: Option<u32>,
    #[serde(default)]
    pub destructor: bool,
    pub summary: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "arg", default)]
    pub args: Vec<Arg>
}
#[derive(Clone, Debug, Deserialize)]
pub struct Event {
    pub name: String,
    pub since: Option<u32>,
    pub summary: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "arg", default)]
    pub args: Vec<Arg>
}

#[derive(Clone, Debug, Deserialize)]
pub struct Entry {
    pub name: String,
    pub since: Option<u32>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub value: u32
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestType {
    Destructor
}

#[derive(Clone, Debug, Deserialize)]
pub struct Arg {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: DataType,
    pub interface: Option<String>,
    #[serde(rename = "enum")]
    pub enumeration: Option<String>,
    pub summary: Option<String>
}
impl Arg {
    pub(crate) fn getter(&self, owning_interface: &String, bindings: &HashMap<String, Binding>) -> TokenStream {
        match self.kind {
            DataType::Int => quote!{args.next_i32()?},
            DataType::Uint => quote!{args.next_u32()?},
            DataType::Fixed => quote!{args.next_fixed()?},
            DataType::String => quote!{args.next_str()?},
            DataType::Array => quote!{args.next_array()?},
            DataType::Fd => quote!{client.next_fd()?},
            DataType::Object => if let Some(_) = &self.interface {
                quote!{client.get(args.next_u32()?)?}
            } else {
                quote!{client.get_any(args.next_u32()?)?}
            },
            DataType::NewId => if let Some(interface) = &self.interface {
                let interface_binding = &bindings.get(&interface.to_snake_case()).map(|b| &b.implementation);
                if let Some(interface_binding) = interface_binding {
                    quote!{args.next_new_id(#interface, #interface_binding::VERSION)?}
                } else {
                    let owner = owning_interface.to_camel_case();
                    let to_implement = interface.to_camel_case();
                    syn::Error::new(bindings[owning_interface].implementation.span(), format!("Interface {:?} depends on {:?}. Please specify an implementation for {:?}.", owner, to_implement, to_implement)).to_compile_error()
                }
            } else {
                quote!{args.next_dynamic_new_id()?}
            },
        }
    }
    pub(crate) fn pusher(&self) -> proc_macro2::TokenStream {
        let arg = format_ident!("wl_{}", self.name);
        match self.kind {
            DataType::Int => quote!{message.push_i32(#arg)},
            DataType::Uint => quote!{message.push_u32(#arg)},
            DataType::Fixed => quote!{message.push_fixed(#arg)},
            DataType::String => quote!{message.push_str(#arg)},
            DataType::Array => quote!{message.push_array(#arg)},
            DataType::Fd => quote!{message.push_fd(#arg)},
            DataType::Object => quote!{{use ::wl::Object; message.push_u32(#arg.object())}},
            DataType::NewId => if let Some(_) = self.interface {
                quote!{message.push_new_id(#arg)}
            } else {
                quote!{message.push_dynamic_new_id(#arg)}
            },
        }
    }
    pub(crate) fn request_data_type(&self, owning_interface: &String, bindings: &HashMap<String, Binding>) -> TokenStream {
        match self.kind {
            DataType::Int => quote!{ i32 },
            DataType::Uint => quote!{ u32 },
            DataType::Fixed => quote!{ ::wl::Fixed },
            DataType::String => quote!{ ::std::string::String },
            DataType::Array => quote!{ ::wl::Array },
            DataType::Fd => quote!{ ::wl::Fd },
            DataType::Object => {
                if let Some(interface) = &self.interface {
                    if let Some(Binding { implementation, ..}) = bindings.get(&interface.to_snake_case()) {
                        quote!{ ::wl::server::Lease<#implementation> }
                    } else {
                        let owner = owning_interface.to_camel_case();
                        let to_implement = interface.to_camel_case();
                        syn::Error::new(bindings[owning_interface].implementation.span(), format!("Interface {:?} depends on {:?}. Please specify an implementation for {:?}.", owner, to_implement, to_implement)).to_compile_error()
                    }
                } else {
                    quote!{ ::wl::server::Lease<dyn ::std::any::Any> }
                }
            },
            DataType::NewId => quote!{ ::wl::NewId }
        }
    }
    pub fn event_data_type(&self) -> syn::Type {
        match self.kind {
            DataType::Int => parse_quote!{ i32 },
            DataType::Uint => parse_quote!{ u32 },
            DataType::Fixed => parse_quote!{ ::wl::Fixed },
            DataType::String => parse_quote!{ &str },
            DataType::Array => parse_quote!{ ::wl::Array },
            DataType::Fd => parse_quote!{ ::wl::Fd },
            DataType::Object => parse_quote!{ &impl ::wl::Object },
            DataType::NewId => parse_quote!{ ::wl::NewId }
        }
    }
    pub fn debug_string(&self) -> &'static str {
        match self.kind {
            DataType::NewId if self.interface.is_none() => "dyn {}",
            DataType::String => "{:?}",
            _ => "{}"
        }
    }
}

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataType {
    Int,
    Uint,
    Fixed,
    String,
    Array,
    Fd,
    Object,
    NewId
}