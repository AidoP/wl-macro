use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::{quote, format_ident};
use syn::{AttributeArgs, Ident, ItemEnum, Lit, NestedMeta, parse_macro_input, spanned::Spanned};

mod protocol;
use protocol::Protocol;

fn get_protocol(attributes: AttributeArgs) -> Vec<Protocol> {
    let mut protocols = vec![];
    for attribute in attributes {
        if let NestedMeta::Lit(Lit::Str(path)) = attribute {
            protocols.push(Protocol::load::<&str>(path.value().as_str()))
        } else {
            panic!("Attribute must be in the form `#[wl::server::protocol(\"name1\", \"name2\")]`")
        }
    }
    protocols
}

#[proc_macro_attribute]
pub fn server_protocol(attribute: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let attributes = parse_macro_input!(attribute as AttributeArgs);
    let mut item = parse_macro_input!(item as ItemEnum);
    let protocols = get_protocol(attributes);
    let mut interfaces = HashMap::new();
    for interface in protocols.iter().map(|p| p.interfaces.iter()).flatten() {
        interfaces.insert(interface.name.clone(), interface.clone());
    }

    let enum_name = &item.ident.clone();
    let copyrights = protocols.iter().map(|p| {
        let protocol = format!("# {} - Copyright", p.name);
        let summary = p.summary.as_ref().map(|s| quote! {#[doc = #s]}).unwrap_or(quote! {});
        let description = p.description.as_ref().map(|s| quote! {#[doc = #s]}).unwrap_or(quote! {});
        p.copyright.as_ref().map(|copyright| quote! {
            #[doc = #protocol]
            #summary
            #description
            #[doc = ""]
            #[doc = #copyright]
        }).unwrap_or_default()
    });

    let display_variant = item.variants.iter().find(|v| v.attrs.iter().any(|a| a.path.get_ident().map_or(None, |i| Some(i.to_string())) == Some("display".into()))).expect("A variant must be tagged with the `#[display]` attribute");
    let display_variant_name = display_variant.ident.clone();
    item.variants.iter_mut().for_each(|v| {
        v.attrs.iter().enumerate().find_map(|(i, a)|
            if a.path.get_ident().map_or(None, |i| Some(i.to_string())) == Some("display".into()) {
                Some(i)
            } else {
                None
            }
        ).map(|i| v.attrs.remove(i));
    });

    let into_protocol_impls = item.variants.iter().map(|v| {
        let mut fields = v.fields.iter();
        let concrete_type = &fields.next().unwrap_or_else(|| panic!("Variant `{}` requires an unnamed field specifying the concrete interface type", v.ident.to_string())).ty;
        if fields.next().is_some() {
            panic!("Variant `{}` must have exactly one unnamed field", v.ident.to_string())
        }
        let interface = &v.ident;
        quote! {
            impl From<#concrete_type> for Protocol {
                fn from(object: #concrete_type) -> Self {
                    Self::#interface(object)
                }
            }
        }
    });

    let args_getter = Ident::new("_args", enum_name.span());

    let variant_dispatch = item.variants.iter()
        .map(|v| {
            let interface_name = &heck::SnakeCase::to_snake_case(v.ident.to_string().as_str());
            let interface = &interfaces.get(interface_name)
                .unwrap_or_else(|| panic!("Interface `{}` not found in the specified protocols", interface_name));
            let interface_name = &v.ident.to_string();
            (0u16 .. interface.requests.len() as u16)
                .map(|i| {
                    let interface_name = interface_name.clone();
                    let name = v.ident.clone();
                    let request = Ident::new(&interface.requests[i as usize].name, v.span());
                    let args = &interface.requests[i as usize].args;
                    let lease_glue = args.iter().map(|a| {
                        let arg = Ident::new(&a.name, request.span());
                        let arg_lease = format_ident!("{}_lease", arg);
                        use protocol::DataType::*;
                        match a.kind {
                            Uint => quote! { let #arg = #args_getter.next_u32().ok_or(wl::DispatchError::ExpectedArgument("uint"))?; },
                            Int => quote! { let #arg = #args_getter.next_i32().ok_or(wl::DispatchError::ExpectedArgument("int"))?; },
                            Fixed => quote! { let #arg = #args_getter.next_fixed().ok_or(wl::DispatchError::ExpectedArgument("fixed"))?; },
                            String => quote! { let #arg = #args_getter.next_str().ok_or(wl::DispatchError::ExpectedArgument("string"))?; },
                            Array => quote! { let #arg = #args_getter.next_array().ok_or(wl::DispatchError::ExpectedArgument("array"))?; },
                            Fd => quote! { let #arg = client.next_fd()?; },
                            Object => quote! {
                                let #arg_lease = client.lease(#args_getter.next_u32().ok_or(wl::DispatchError::ExpectedArgument("object"))?)?;
                                let #arg = #arg_lease.try_map(|p| match p {
                                    #enum_name::#name(this) => Ok(this),
                                    _ => Err(wl::DispatchError::InvalidObject(#interface_name, self.interface()))
                                })?;
                            },
                            NewId => if let Some(interface) = &a.interface {
                                let interface_version = interfaces[interface].version as u32;
                                quote! {
                                    let #arg = wl::NewId::new(#args_getter.next_u32().ok_or(wl::DispatchError::ExpectedArgument("newid id"))?, #interface_version, #interface);
                                }
                            } else {
                                quote! {
                                    let #arg = #args_getter.next_new_id()?;
                                }
                            }
                        }
                    });
                    let release_glue = args.iter().map(|a| {
                        let arg = Ident::new(&a.name, request.span());
                        let arg_lease = format_ident!("{}_lease", arg);
                        use protocol::DataType::*;
                        match a.kind {
                            Object => quote! {
                                client.release(#arg_lease);
                            },
                            _ => quote!{}
                        }
                    });
                    let args: Vec<_> = args.iter().map(|a| {
                        let i = Ident::new(&a.name, v.span());
                        match a.kind {
                            protocol::DataType::Object => quote!{ &mut #i },
                            _ => quote! { #i }
                        }
                    }).collect();
                    let debug_string = format!("-> {}@{{}}.{}({})", interface_name, request.to_string(), args.iter().map(|_| "{:?}, ").collect::<String>());
                    quote! {
                        (Self::#name(object), #i) => {
                            #( #lease_glue )*
                            let lease = generic_lease.map(|p| match p {
                                Self::#name(object) => object,
                                _ => unreachable!()
                            });
                            #[cfg(debug_assertions)]
                            println!(#debug_string, lease.id, #(#args),*);
                            let result = #name::#request(lease, client, #(#args),*);
                            #( #release_glue )*
                            Ok(result.map(|lease| lease.map(|object| Self::#name(object))))
                        },
                    }
                }).collect::<TokenStream>().into_iter()
        }).flatten();
    let variant_interface_names = item.variants.iter()
        .map(|v| {
            let v = v.ident.clone();
            let name = heck::SnakeCase::to_snake_case(v.to_string().as_str());
            quote! { Self::#v(_) => #name }
        });
    let interface_traits = item.variants.iter()
        .map(|v| {
            let interface_name = &heck::SnakeCase::to_snake_case(v.ident.to_string().as_str());
            let interface = &interfaces.get(interface_name)
                .unwrap_or_else(|| panic!("Interface `{}` not found in the specified protocols", interface_name));
            let interface_trait = Ident::new(v.ident.to_string().as_str(), v.ident.span());
            let summary = &interface.summary;
            let description = &interface.description;
            let requests = interface.requests.iter().map(|r| {
                let summary = r.summary.as_ref().map(|s| quote!{ #[doc = #s]}).unwrap_or_default();
                let description = &r.description;
                let request_fn = Ident::new(&r.name, v.ident.span());
                let args = r.args.iter().map(|a| {
                    let arg_field = Ident::new(&a.name, v.ident.span());
                    use protocol::DataType::*;
                    let arg_type = match a.kind {
                        Int => quote! { i32 },
                        Uint => quote! { u32 },
                        Fixed => quote! { wl::Fixed },
                        Array => quote! { &[u8] },
                        String => quote! { &[u8] },
                        Fd => quote! { i32 },
                        Object => {
                            if let Some(interface_name) = a.interface.clone() {
                                let interface = item.variants.iter()
                                    .find(|v| v.ident.to_string() == heck::CamelCase::to_camel_case(interface_name.as_str()))
                                    .unwrap_or_else(|| panic!("Protocol does not implement interface `{}` referenced by request `{}`", interface_name, r.name));
                                let mut fields = interface.fields.iter();
                                let e = || panic!("Protocol variant `{}` must have a single field referencing a struct implementing the interface `{}`", v.ident.to_string(), interface_name);
                                let interface = fields.next().map(|f| f.clone()).unwrap_or_else(e).ty;
                                if fields.next().is_some() {
                                    e();
                                }
                                quote! { wl::server::Lease<#interface> }
                            } else {
                                quote! { wl::server::GenericLease<Protocol> }
                            }
                        },
                        NewId => quote! { wl::NewId }
                    };
                    quote! {
                        #arg_field: #arg_type
                    }
                });
                quote! {
                    #summary
                    #[doc = "# Description"]
                    #[doc = #description]
                    fn #request_fn(self, client: &mut wl::server::Client<Protocol>, #(#args),*) -> Option<Self>;
                }
            });
            let events = interface.events.iter().enumerate().map(|(opcode, e)| {
                let event_fn = Ident::new(&e.name, interface_trait.span());
                let args = e.args.iter().map(|a| {
                    let arg_ident = Ident::new(&a.name, event_fn.span());
                    use protocol::DataType::*;
                    let arg_type = match a.kind {
                        Int => quote! { i32 },
                        Uint => quote! { u32 },
                        Fixed => quote! { wl::Fixed },
                        Array => quote! { &[u8] },
                        String => quote! { &str },
                        Fd => quote! { i32 },
                        Object => {
                            if let Some(interface_name) = a.interface.clone() {
                                let interface = item.variants.iter()
                                    .find(|v| v.ident.to_string() == heck::CamelCase::to_camel_case(interface_name.as_str()))
                                    .unwrap_or_else(|| panic!("Protocol does not implement interface `{}` referenced by event `{}`", interface_name, e.name));
                                let mut fields = interface.fields.iter();
                                let e = || panic!("Protocol variant `{}` must have a single field referencing a struct implementing the interface `{}`", v.ident.to_string(), interface_name);
                                let interface = fields.next().map(|f| f.clone()).unwrap_or_else(e).ty;
                                if fields.next().is_some() {
                                    e();
                                }
                                quote! { wl::server::Lease<#interface> }
                            } else {
                                quote! { wl::server::GenericLease<Protocol> }
                            }
                        },
                        NewId => quote! { wl::NewId }
                    };
                    quote! { #arg_ident: #arg_type }
                });
                let summary = e.summary.as_ref().map(|s| quote!{ #[doc = #s]}).unwrap_or_default();
                let description = &e.description;
                let msg = Ident::new("_internal_message", event_fn.span());
                let arg_glue = e.args.iter().map(|a| {
                    let arg_ident = Ident::new(&a.name, event_fn.span());
                    use protocol::DataType::*;
                    match a.kind {
                        Int => quote! { #msg.push_i32(#arg_ident) },
                        Uint => quote! { #msg.push_u32(#arg_ident) },
                        Fixed => quote! { #msg.push_fixed(#arg_ident) },
                        Array => quote! { #msg.push_bytes(#arg_ident) },
                        String => quote! { #msg.push_str(#arg_ident) },
                        Fd => quote! { ! },
                        Object => quote! { #msg.push_u32(#arg_ident.id) },
                        NewId => quote! { #msg.push_u32(#arg_ident) }
                    }
                });
                let debug_args = e.args.iter().map(|a| Ident::new(&a.name, event_fn.span()));
                let debug_string = format!("<- {}@{{}}.{}({})", interface_name, event_fn.to_string(), e.args.iter().map(|_| "{:?}, ").collect::<String>());
                let opcode = opcode as u16;
                quote! {
                    #summary
                    #[doc = "# Description"]
                    #[doc = #description]
                    fn #event_fn(&mut self, client: &mut wl::server::Client<Protocol>, #(#args),*) -> wl::Result<()> {
                        let mut #msg = wl::Message {
                            object: self.id(),
                            opcode: #opcode,
                            args: vec![]
                        };
                        #(#arg_glue;)*
                        #[cfg(debug_assertions)]
                        println!(#debug_string, self.id(), #(#debug_args),*);
                        client.send(#msg)
                    }
                }
            });
            quote! {
                #[doc = #summary]
                #[doc = "# Description"]
                #[doc = #description]
                pub trait #interface_trait: wl::server::Object + Sized {
                    #(#requests)*
                    #(#events)*
                }
            }
        });
    
    (quote! {
        #(#copyrights)*
        #item
        impl Default for #enum_name {
            fn default() -> Self {
                Self::#display_variant_name(Default::default())
            }
        }
        impl wl::server::Protocol for #enum_name {
            fn request(generic_lease: wl::server::GenericLease<Self>, client: &mut wl::server::Client<Self>, message: wl::Message) -> wl::Result<Option<wl::server::GenericLease<Self>>> {
                let mut #args_getter = message.args();
                let interface = generic_lease.interface();
                match (&*generic_lease, message.opcode) {
                    #(
                        #variant_dispatch
                    )*
                    (_, opcode) => Err(wl::DispatchError::InvalidOpcode(message.object, opcode, interface))
                }
            }
            fn interface(&self) -> &'static str {
                match self {
                    #(
                        #variant_interface_names
                    ),*
                }
            }
        }
        #( #into_protocol_impls )*

        #( #interface_traits )*
    }).into()
}