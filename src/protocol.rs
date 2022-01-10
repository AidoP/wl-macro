use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use serde::Deserialize;

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
    fn default_path(named: &str) -> PathBuf {
        let mut path = if let Some(path) = option_env!("WL_PROTOCOLS") {
            PathBuf::from(path)
        } else {
            PathBuf::from("protocol/")
        };
        path.push(named);
        path.set_extension("toml");
        path
    }
    pub fn from_str(string: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(string)
    }
    pub fn load<P: AsRef<Path>>(named: &str) -> Self {
        let mut protocol = String::new();
        let path = &Self::default_path(named);
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
    pub fn is_dyn_new_id(&self) -> bool {
        if let DataType::NewId = self.kind {
            self.interface.is_none()
        } else {
            false
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
impl DataType {
    pub fn send_method(&self, arg: syn::Ident, interface: Option<syn::LitStr>) -> proc_macro2::TokenStream {
        use quote::quote;
        match self {
            Self::Int => quote!{msg.push_i32(#arg)},
            Self::Uint => quote!{msg.push_u32(#arg)},
            Self::Fixed => quote!{msg.push_fixed(#arg)},
            Self::String => quote!{msg.push_str(#arg)},
            Self::Array => quote!{msg.push_array(#arg)},
            Self::Fd => quote!{client.push_fd(#arg)},
            Self::Object => quote!{msg.push_u32(#arg)},
            Self::NewId => if let Some(_) = interface {
                quote!{msg.push_new_id(#arg)}
            } else {
                quote!{msg.push_dynamic_new_id(#arg)}
            },
        }
    }
    pub fn get_method(&self, interface: Option<syn::LitStr>, version: u32) -> proc_macro2::TokenStream {
        use quote::quote;
        match self {
            Self::Int => quote!{args.next_i32()?},
            Self::Uint => quote!{args.next_u32()?},
            Self::Fixed => quote!{args.next_fixed()?},
            Self::String => quote!{args.next_str()?},
            Self::Array => quote!{args.next_array()?},
            Self::Fd => quote!{client.next_fd()?},
            Self::Object => quote!{args.next_u32()?},
            Self::NewId => if let Some(_) = interface {
                quote!{args.next_new_id(#interface, #version)?}
            } else {
                quote!{args.next_dynamic_new_id()?}
            },
        }
    }
    pub fn data_type(&self, _: proc_macro2::Span) -> syn::Type {
        use syn::parse_quote;
        match self {
            Self::Int => parse_quote!{ i32 },
            Self::Uint => parse_quote!{ u32 },
            Self::Fixed => parse_quote!{ wl::Fixed },
            Self::String => parse_quote!{ std::string::String },
            Self::Array => parse_quote!{ wl::Array },
            Self::Fd => parse_quote!{ wl::Fd },
            Self::Object => parse_quote!{ u32 },
            Self::NewId => parse_quote!{ wl::NewId }
        }
    }
}