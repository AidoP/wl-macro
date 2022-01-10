use std::ops::Deref;

use quote::{quote};
use syn::{AttributeArgs, Ident, ItemImpl, Lit, NestedMeta, parse_macro_input, parse_quote, spanned::Spanned};

mod protocol;
use protocol::Protocol;

fn get_protocol(attributes: AttributeArgs) -> Protocol {
    if attributes.len() == 1 {
        if let NestedMeta::Lit(Lit::Str(path)) = attributes.first().unwrap() {
            return Protocol::load::<&str>(path.value().as_str())
        }
    }
    panic!("Attribute must be in the form `#[wl::server::protocol(\"specification.toml\")]`")
}

#[proc_macro_attribute]
pub fn server_protocol(attribute: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let attributes = parse_macro_input!(attribute as AttributeArgs);
    let item = parse_macro_input!(item as ItemImpl);
    let (_, interface, _) = item.trait_.clone().expect("Attribute must be applied to an implementation of the desired interface");
    let interface_ident = interface.get_ident().expect("Interface must be an identifier corresponding to the interface name in the specification file");
    let interface_name = heck::SnakeCase::to_snake_case(interface_ident.to_string().as_str());
    let interface_name_lit = syn::LitStr::new(&interface_name, interface_ident.span());

    // TODO: A way to avoid re-parsing the protocol file each time
    let protocol = get_protocol(attributes);
    let interface = protocol.interfaces.iter()
        .find(|i| i.name == interface_name)
        .expect(&format!("No interface named {:?} was found.", interface_name));
    let interface_version = interface.version;

    let self_type = item.self_ty.clone();
    let base_struct: syn::Path = match self_type.deref() {
        syn::Type::Path(path) => {
            let lease = path.path.segments.last().expect("Interface must be implemented for Lease<T>");
            match &lease.arguments {
                syn::PathArguments::AngleBracketed(args) => {
                    let args = &args.args;
                    parse_quote! {
                        #args
                    }
                },
                _ => panic!("Interface must be implemented for Lease<T>")
            }
        },
        _ => panic!("Interface must be implemented for Lease<T>")
    };

    let event_impls = interface.events.iter().enumerate().map(|(opcode, event)| {
        let opcode = opcode as u16;
        let event_name = Ident::new(&event.name, item.span());
        let event_fn_lit = event.name.clone();
        let args = event.args.iter().map(|arg| {
            let ident = Ident::new(&arg.name, item.span());
            let ty = arg.event_data_type(item.span());
            quote!{
                #ident: #ty
            }
        });
        let args_debug = event.args.iter().map(|arg| {
            let ident = Ident::new(&arg.name, item.span());
            quote!{
                #ident
            }
        });
        let args_send_method = event.args.iter().map(|arg| {
            let method = arg.send_method(item.span());
            quote!{
                #method
            }
        });
        let mut args_debug_string = "-> {}@{}.{}(".to_string();
        let mut first = true;
        for arg in event.args.iter() {
            if !first {
                args_debug_string.push_str(", ");
            }
            args_debug_string.push_str(arg.debug_string());
            first = false;
        }
        args_debug_string.push(')');
        quote!{
            fn #event_name(&mut self, client: &mut wl::server::Client, #(#args),*) -> wl::Result<()> {
                use wl::Object;
                if *wl::DEBUG {
                    eprintln!(#args_debug_string, #interface_name_lit, self.object(), #event_fn_lit, #(#args_debug),*)
                }
                let mut msg = wl::Message::new(self.object(), #opcode);
                #(#args_send_method;)*
                client.send(msg)
            }
        }
    });
    let request_sigs = interface.requests.iter().map(|request| {
        let request_impl = item.items.iter()
            .find_map(|i| if let syn::ImplItem::Method(method) = i {
                if method.sig.ident == request.name {
                    Some(method)
                } else {
                    None
                }
            } else {
                None
            }).expect(&format!("Request {:?} must be implemented", request.name));
        let request_name = Ident::new(&request.name, request_impl.sig.ident.span());
        let args = request.args.iter().map(|arg| {
            let ident = Ident::new(&arg.name, request_impl.span());
            let ty = arg.data_type(request_impl.span());
            quote!{
                #ident: #ty
            }
        });
        quote!{
            fn #request_name(&mut self, client: &mut wl::server::Client, #(#args),*) -> wl::Result<()>;
        }
    });
    let request_dispatch = interface.requests.iter().enumerate().map(|(opcode, request)| {
        let opcode = opcode as u16;
        let request_impl = item.items.iter()
            .find_map(|i| if let syn::ImplItem::Method(method) = i {
                if method.sig.ident == request.name {
                    Some(method)
                } else {
                    None
                }
            } else {
                None
            }).expect(&format!("Request {:?} must be implemented", request.name));
        let request_fn = Ident::new(&request.name, request_impl.sig.ident.span());
        let request_fn_lit = request.name.clone();
        let arg_defs = request.args.iter().map(|arg| {
            let ident = Ident::new(&arg.name, request_impl.span());
            let get = arg.get_method(interface_version);
            quote!{
                let #ident = #get;
            }
        });
        let args: Vec<_> = request.args.iter().map(|arg| {
            let ident = Ident::new(&arg.name, request_impl.span());
            quote!{
                #ident
            }
        }).collect();
        let mut args_debug_string = "{}@{}.{}(".to_string();
        let mut first = true;
        for arg in request.args.iter() {
            if !first {
                args_debug_string.push_str(", ");
            }
            args_debug_string.push_str(arg.debug_string());
            first = false;
        }
        args_debug_string.push(')');
        quote!{
            #opcode => {
                #(#arg_defs)*
                if *wl::DEBUG {
                    use wl::Object;
                    eprintln!(#args_debug_string, #interface_name_lit, lease.object(), #request_fn_lit, #(#args),*)
                }
                lease.#request_fn(client, #(#args),*)
            }
        }
    });

    quote! {
        #item
        
        trait #interface_ident: wl::Object {
            //fn init(&mut self, client: &mut wl::server::Client) -> wl::Result<()>;
            #(#event_impls)*
            #(#request_sigs)*
        }
        impl wl::server::Dispatch for #base_struct {
            const INTERFACE: &'static str = #interface_name_lit;
            const VERSION: u32 = #interface_version;
            fn dispatch(lease: wl::server::Lease<dyn std::any::Any>, client: &mut wl::server::Client, message: wl::Message) -> wl::Result<()>  {
                let mut lease: wl::server::Lease<#base_struct> = lease.downcast().unwrap();
                let mut args = message.args();
                match message.opcode {
                    #(#request_dispatch)*
                    _ => Err(wl::DispatchError::InvalidOpcode(message.object, message.opcode, Self::INTERFACE))
                }
            }
            /*fn init(lease: &mut Lease<Self>, client: &mut wl::server::Client) -> wl::Result<()> {
                lease.init(client)
            }*/
        }
    }.into()
}