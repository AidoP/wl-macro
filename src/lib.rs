use std::collections::HashMap;

use quote::{quote, format_ident};
use syn::{parse_macro_input, parse::{Parse, ParseStream}, punctuated::Punctuated, LitStr, Visibility, Token, Ident, Path, braced, spanned::Spanned};
use proc_macro2::TokenStream;

use heck::{CamelCase, SnakeCase, ShoutySnakeCase};

mod protocol;
use protocol::*;

struct ProtocolModule {
    visibility: Visibility,
    ident: Ident,
    bindings: HashMap<String, Binding>
}
impl Parse for ProtocolModule {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let visibility = input.parse()?;
        let _: Token![mod] = input.parse()?;
        let ident = input.parse()?;
        let content;
        let _ = braced!(content in input);
        let mut bindings = HashMap::new();
        let punctuated_bindings: Punctuated<Binding, Token![;]> = content.parse_terminated(Binding::parse)?;
        for binding in punctuated_bindings {
            let interface = binding.interface.to_string().to_snake_case();
            if bindings.contains_key(&interface) {
                panic!("Duplicate definition of interface {:?}", interface.to_camel_case());
            }
            bindings.insert(interface, binding);
        }
        Ok(Self {
            visibility,
            ident,
            bindings
        })
    }
}
pub(crate) struct Binding {
    is_external: bool,
    interface: Ident,
    implementation: Path
}
impl Parse for Binding {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        if lookahead.peek(Token![type]) {
            let _: Token![type] = input.parse()?;
            let interface = input.parse()?;
            let _: Token![=] = input.parse()?;
            let implementation = input.parse()?;
            Ok(Self {
                is_external: false,
                interface,
                implementation
            })
        } else {
            let _: Token![use] = input.parse()?;
            let implementation = input.parse()?;
            let _: Token![as] = input.parse()?;
            let interface = input.parse()?;
            Ok(Self {
                is_external: true,
                interface,
                implementation
            })
        }
    }
}

#[proc_macro_attribute]
/// Parses the wayland protocol specification, producing a set of interface traits inside a module named after the protocol
/// ```rust
/// use wl::{prelude::*, Result};
/// protocol!("wayland.toml")
/// 
/// struct Display;
/// #[dispatch]
/// impl wayland::WlDisplay for Lease<WlDisplay> {
///     fn sync(&mut self, client: &mut Client, id: NewId) -> Result<()> {
///         todo!()
///     }
///     fn get_regsitry(&mut self, client: &mut Client, id: NewId) -> Result<()> {
///         todo!()
///     }
/// }
/// ```
pub fn server_protocol(attr: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let path = parse_macro_input!(attr as LitStr).value();
    let module = parse_macro_input!(item as ProtocolModule);

    let module_visibility = &module.visibility;
    let module_name = &module.ident;
    let bindings = &module.bindings;

    let protocol = Protocol::load::<&str>(&path);
    let protocol_name = protocol.name.to_snake_case();
    let protocol_copyright = protocol.copyright.iter();
    let interfaces = protocol.interfaces.iter()
        .filter(|interface| bindings.get(&interface.name.to_snake_case()).map(|b| !b.is_external).unwrap_or(false))
        .map(|interface| generate_interface(interface, bindings));
    let enums = protocol.interfaces.iter().map(|interface| generate_enums(interface));

    // TODO: Reenable this error for types not marked as extern or something
    let interface_not_found_errors = bindings.iter().filter_map(|(interface, binding)|
        if protocol.interfaces.iter().find(|known_interface| interface.to_snake_case() == known_interface.name.to_snake_case()).is_some() || binding.is_external {
            None
        } else {
            Some(syn::Error::new(binding.implementation.span(), format!("No interface named {:?}", interface.to_snake_case())).to_compile_error())
        }
    );

    quote! {
        #[allow(unused_variables)]
        #module_visibility mod #module_name {
            #(#interface_not_found_errors)*
            pub const PROTOCOL: &'static str = #protocol_name;
            #(pub const COPYRIGHT: &'static str = #protocol_copyright;)*
            #(#interfaces)*
            #(#enums)*
        }
    }.into()
}

fn generate_interface(interface: &Interface, bindings: &HashMap<String, Binding>) -> TokenStream {
    let interface_name = format_ident!("{}", interface.name.to_camel_case());
    let interface_description = interface.description.iter();
    let interface_version = interface.version;
    let interface_string = &interface.name;
    let implementor_struct = &bindings[interface_string].implementation;
    let events = interface.events.iter().enumerate().map(|(opcode, event)| generate_event(event, interface, opcode as u16));
    let requests = interface.requests.iter().map(|request| generate_request(request, interface, bindings));
    let request_dispatch = interface.requests.iter().enumerate().map(|(opcode, request)| generate_request_dispatch(request, opcode as u16, interface, bindings));
    quote!{
        #(#[doc = #interface_description])*
        pub trait #interface_name: ::wl::Object {
            const VERSION: u32 = #interface_version;
            const INTERFACE: &'static str = #interface_string;
            #(#events)*
            #(#requests)*
        }
        impl ::wl::server::Dispatch for #implementor_struct {
            const INTERFACE: &'static str = #interface_string;
            const VERSION: u32 = #interface_version;
            fn dispatch(lease: ::wl::server::Lease<dyn ::std::any::Any>, client: &mut ::wl::server::Client, message: ::wl::Message) -> ::wl::server::Result<()> {
                use ::wl::Object;
                use ::std::convert::Into;
                let mut lease: ::wl::server::Lease<#implementor_struct> = lease.downcast().unwrap();
                let mut args = message.args();
                match message.opcode {
                    #(#request_dispatch)*
                    _ => ::std::result::Result::Err(::wl::DispatchError::InvalidRequest {
                        opcode: message.opcode,
                        object: lease.object(),
                        interface: Self::INTERFACE
                    }.into())
                }
            }
        }
    }
}

fn generate_event(event: &Event, interface: &Interface, opcode: u16) -> TokenStream {
    let event_name = format_ident!("r#{}", event.name.to_snake_case());
    let event_summary = event.summary.iter();
    let event_description = event.description.iter();
    let parameters = event.args.iter().map(|arg| generate_event_parameter(arg));
    let debug_print = generate_event_debug_print(event, interface);
    let arg_pushers = event.args.iter().map(|arg| arg.pusher());
    quote! {
        #(#[doc = #event_summary])*
        #[doc = "\n"]
        #(#[doc = #event_description])*
        fn #event_name(&mut self, client: &mut ::wl::server::Client, #(#parameters),*) -> ::wl::server::Result<()> {
            use ::wl::Object;
            if *::wl::DEBUG {
                #debug_print
            }
            let mut message = ::wl::Message::new(self.object(), #opcode);
            #(#arg_pushers;)*
            client.send(message)
        }
    }
}
fn generate_event_parameter(arg: &Arg) -> TokenStream {
    let arg_name = format_ident!("wl_{}", arg.name.to_snake_case());
    let arg_type = arg.event_data_type();
    quote! {
        #arg_name: #arg_type
    }
}
fn generate_event_debug_print(event: &Event, interface: &Interface) -> TokenStream {
    let interface_name = &interface.name;
    let event_name = &event.name;
    let args = event.args.iter().map(|arg| {
        let arg_name = format_ident!("wl_{}", arg.name);
        quote!{#arg_name}
    });
    let mut format_string = "-> {}@{}.{}(".to_string();
    let mut first = true;
    for arg in &event.args {
        if !first  {
            format_string.push_str(", ")
        } else {
            first = false
        }
        format_string.push_str(arg.debug_string());
    }
    format_string.push(')');
    quote! {
        ::std::eprintln!(#format_string, #interface_name, self.object(), #event_name, #(#args),*)
    }
}
fn generate_request(request: &Request, interface: &Interface, bindings: &HashMap<String, Binding>) -> TokenStream {
    let request_name = format_ident!("r#{}", request.name.to_snake_case());
    let request_summary = request.summary.iter();
    let request_description = request.description.iter();
    let owning_interface = &interface.name.to_snake_case();
    let parameters = request.args.iter().map(|arg| generate_parameter(arg, owning_interface, bindings));
    quote! {
        #(#[doc = #request_summary])*
        #[doc = "\n"]
        #(#[doc = #request_description])*
        fn #request_name(&mut self, client: &mut ::wl::server::Client, #(#parameters),*) -> ::wl::server::Result<()>;
    }
}
fn generate_parameter(arg: &Arg, owning_interface: &String, bindings: &HashMap<String, Binding>) -> TokenStream {
    let arg_name = format_ident!("wl_{}", arg.name.to_snake_case());
    let arg_type = arg.request_data_type(owning_interface, bindings);
    quote! {
        #arg_name: #arg_type
    }
}
fn generate_request_dispatch(request: &Request, opcode: u16, interface: &Interface, bindings: &HashMap<String, Binding>) -> TokenStream {
    let mut request_name = format_ident!("r#{}", request.name.to_snake_case());
    let interface_string = &interface.name;
    request_name.set_span(bindings[interface_string].implementation.span());
    let arg_names = request.args.iter().map(|arg| format_ident!("wl_{}", arg.name.to_snake_case()));
    let arg_getters = request.args.iter().map(|arg| generate_arg_getter(arg, interface_string, bindings));
    let debug_print = generate_request_debug_print(request, interface);
    quote! {
        #opcode => {
            #(#arg_getters)*
            if *::wl::DEBUG {
                #debug_print
            }
            lease.#request_name(client #(, #arg_names)*)
        }
    }
}
fn generate_arg_getter(arg: &Arg, owning_interface: &String, bindings: &HashMap<String, Binding>) -> TokenStream {
    let arg_name = format_ident!("wl_{}", arg.name.to_snake_case());
    let getter = arg.getter(owning_interface, bindings);
    quote! {
        let #arg_name = #getter;
    }
}
fn generate_request_debug_print(request: &Request, interface: &Interface) -> TokenStream {
    let interface_name = &interface.name;
    let request_name = &request.name;
    let args = request.args.iter().map(|arg| {
        let arg_name = format_ident!("wl_{}", arg.name);
        quote!{#arg_name}
    });
    let mut format_string = "{}@{}.{}(".to_string();
    let mut first = true;
    for arg in &request.args {
        if !first  {
            format_string.push_str(", ")
        } else {
            first = false
        }
        format_string.push_str(arg.debug_string());
    }
    format_string.push(')');
    quote! {
        ::std::eprintln!(#format_string, #interface_name, lease.object(), #request_name, #(#args),*)
    }
}
fn generate_enums(interface: &Interface) -> TokenStream {
    let enums = interface.enums.iter().map(|e| generate_enum(e, interface));
    quote! {
        #(#enums)*
    }
}
fn generate_enum(e: &Enum, interface: &Interface) -> TokenStream {
    let enum_name = format_ident!("{}{}", interface.name.to_camel_case(), e.name.to_camel_case());
    let enum_summary = e.summary.iter();
    let enum_description = e.description.iter();
    let enum_wl_name = format!("{}.{}", interface.name, e.name);
    let normalise_entry_name = |name: &str| if name.chars().next().map(|c| c.is_alphabetic()).unwrap_or(false) {
        name.to_shouty_snake_case()
    } else {
        format!("{}_{}", interface.name.to_shouty_snake_case(), name.to_shouty_snake_case())
    };
    let entries = e.entries.iter().map(|entry| {
        let entry_name = format_ident!("{}", normalise_entry_name(&entry.name));
        let entry_summary = entry.summary.iter();
        let entry_description = entry.description.iter();
        let value = entry.value;
        quote!{
            #(#[doc = #entry_summary])*
            #[doc = "\n"]
            #(#[doc = #entry_description])*
            pub const #entry_name: u32 = #value
        }
    });
    let entry_constructors = e.entries.iter().map(|entry| {
        let entry_name = format_ident!("{}", normalise_entry_name(&entry.name));
        let value = entry.value;
        quote!{
            #value => ::std::result::Result::Ok(Self(Self::#entry_name))
        }
    });
    quote! {
        #[derive(::std::fmt::Debug, ::std::marker::Copy, ::std::clone::Clone, ::std::cmp::Eq, ::std::cmp::PartialEq)]
        pub struct #enum_name(u32);
        #(#[doc = #enum_summary])*
        #[doc = "\n"]
        #(#[doc = #enum_description])*
        impl #enum_name {
            pub const ENUM_NAME: &'static str = #enum_wl_name;
            #(#entries;)*
            pub fn new(value: u32) -> ::wl::server::Result<Self> {
                use ::std::convert::Into;
                match value {
                    #(#entry_constructors,)*
                    _ => ::std::result::Result::Err(::wl::DispatchError::NoVariant { name: Self::ENUM_NAME, variant: value }.into())
                }
            }
        }
        impl ::std::convert::Into<u32> for #enum_name {
            fn into(self) -> u32 {
                self.0
            }
        }
        impl ::std::ops::Deref for #enum_name {
            type Target = u32;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    }
}