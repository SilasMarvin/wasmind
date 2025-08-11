use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Lit, parse_macro_input};

#[proc_macro]
pub fn generate_actor_trait(_input: TokenStream) -> TokenStream {
    let expanded = quote! {
        trait GeneratedActorTrait {
            fn new(scope: String, config: String) -> Self;

            fn handle_message(&mut self, message: crate::bindings::exports::wasmind::actor::actor::MessageEnvelope);

            fn destructor(&mut self) {}


            fn broadcast_common_message<S: ::wasmind_actor_utils::actors::macros::__private::serde::ser::Serialize + ::wasmind_actor_utils::messages::Message>(payload: S) -> Result<(), ::wasmind_actor_utils::actors::macros::__private::serde_json::Error> {
                use ::wasmind_actor_utils::messages::Message;
                Ok(crate::bindings::wasmind::actor::messaging::broadcast(
                    S::MESSAGE_TYPE,
                    &::wasmind_actor_utils::tools::macros::__private::serde_json::to_string(&
                        payload
                    )?.into_bytes()
                ))
            }

            fn parse_as<S: ::wasmind_actor_utils::actors::macros::__private::serde::de::DeserializeOwned + ::wasmind_actor_utils::messages::Message>(msg: &crate::bindings::exports::wasmind::actor::actor::MessageEnvelope) -> Option<S> {
                use ::wasmind_actor_utils::messages::Message;
                if let Ok(json_string) = str::from_utf8(&msg.payload) && S::MESSAGE_TYPE == &msg.message_type {
                    ::wasmind_actor_utils::tools::macros::__private::serde_json::from_str::<S>(json_string).ok()
                } else {
                    None
                }
            }
        }
    };

    TokenStream::from(expanded)
}

#[proc_macro_derive(Actor)]
pub fn actor_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let actor_name = syn::Ident::new(&format!("{name}Actor"), name.span());

    let expanded = quote! {
        const _: () = {
            fn assert_impl_tool<T: GeneratedActorTrait>() {}
            fn assert() {
                assert_impl_tool::<#name>();
            }
        };

        // Actor wrapper type
        pub struct #actor_name {
            actor: std::cell::RefCell<#name>
        }

        impl crate::bindings::exports::wasmind::actor::actor::GuestActor for #actor_name {
            fn new(scope: String, config: String) -> Self {
                // Set up panic hook to log errors before WASM trap
                std::panic::set_hook(Box::new(|panic_info| {
                    let msg = panic_info.to_string();
                    crate::bindings::wasmind::actor::logger::log(
                        crate::bindings::wasmind::actor::logger::LogLevel::Error,
                        &format!("Actor panic: {}", msg)
                    );
                }));

                let actor = <#name as GeneratedActorTrait>::new(scope, config);
                Self {
                    actor: std::cell::RefCell::new(actor)
                }
            }

            fn handle_message(&self, message: crate::bindings::exports::wasmind::actor::actor::MessageEnvelope) {
               self.actor.borrow_mut().handle_message(message);
            }

            fn destructor(&self) {
               self.actor.borrow_mut().destructor();
            }
        }

        pub struct Component;

        impl crate::bindings::exports::wasmind::actor::actor::Guest for Component {
            type Actor = #actor_name;
        }

       crate::bindings::export!(Component with_types_in bindings);
    };

    TokenStream::from(expanded)
}

struct ToolAttributes {
    name: String,
    description: String,
    schema: String,
}

#[proc_macro_derive(Tool, attributes(tool))]
pub fn tool_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let actor_name = syn::Ident::new(&format!("{name}Actor"), name.span());

    // Parse attributes
    let attrs = parse_tool_attributes(&input);
    let tool_name = &attrs.name;
    let tool_desc = &attrs.description;
    let tool_schema = &attrs.schema;

    if let Err(e) = serde_json::from_str::<serde_json::Value>(&attrs.schema) {
        return TokenStream::from(
            syn::Error::new_spanned(&input, format!("Invalid JSON schema: {e}")).to_compile_error(),
        );
    }

    let expanded = quote! {
        // Ensure the ToolTrait is implemented
        const _: () = {
            fn assert_impl_tool<T: ::wasmind_actor_utils::tools::Tool>() {}
            fn assert() {
                assert_impl_tool::<#name>();
            }
        };


        // Actor wrapper type
        pub struct #actor_name {
            scope: String,
            tool: std::cell::RefCell<#name>
        }

        impl #name {
            fn broadcast_common_message<S: serde::ser::Serialize + ::wasmind_actor_utils::messages::Message>(payload: S) -> Result<(), serde_json::Error> {
                use ::wasmind_actor_utils::messages::Message;
                Ok(crate::bindings::wasmind::actor::messaging::broadcast(
                    S::MESSAGE_TYPE,
                    &serde_json::to_string(&payload)?.into_bytes()
                ))
            }
        }

        impl crate::bindings::exports::wasmind::actor::actor::GuestActor for #actor_name {
            fn new(scope: String, config: String) -> Self {
                // Set up panic hook to log errors before WASM trap
                std::panic::set_hook(Box::new(|panic_info| {
                    let msg = panic_info.to_string();
                    crate::bindings::wasmind::actor::logger::log(
                        crate::bindings::wasmind::actor::logger::LogLevel::Error,
                        &format!("Tool panic: {}", msg)
                    );
                }));

                let s = Self {
                    scope: scope.clone(),
                    tool: std::cell::RefCell::new(<#name as ::wasmind_actor_utils::tools::Tool>::new(scope.clone(), config))
                };

                use ::wasmind_actor_utils::messages::Message;

                crate::bindings::wasmind::actor::messaging::broadcast(
                    ::wasmind_actor_utils::messages::common_messages::tools::ToolsAvailable::MESSAGE_TYPE,
                    &::wasmind_actor_utils::tools::macros::__private::serde_json::to_string(&
                        ::wasmind_actor_utils::messages::common_messages::tools::ToolsAvailable {
                            tools: vec![
                                ::wasmind_actor_utils::tools::macros::__private::wasmind_llm_types::Tool {
                                    tool_type: "function".to_string(),
                                    function: ::wasmind_actor_utils::tools::macros::__private::wasmind_llm_types::ToolFunctionDefinition {
                                        name: #tool_name.to_string(),
                                        description: #tool_desc.to_string(),
                                        parameters: ::wasmind_actor_utils::tools::macros::__private::serde_json::from_str(& #tool_schema).expect("schema should be valid JSON")
                                    }
                                }
                            ]
                        }
                    ).unwrap().into_bytes()
                );

                s
            }

            fn handle_message(&self, message: crate::bindings::exports::wasmind::actor::actor::MessageEnvelope) {
                use ::wasmind_actor_utils::messages::common_messages::Message;
                if message.message_type == ::wasmind_actor_utils::messages::common_messages::tools::ExecuteTool::MESSAGE_TYPE {
                    if let Ok(json_string) = String::from_utf8(message.payload) {
                        if let Ok(execute_tool_call) = ::wasmind_actor_utils::tools::macros::__private::serde_json::from_str::<::wasmind_actor_utils::messages::common_messages::tools::ExecuteTool>(&json_string) {
                            if execute_tool_call.tool_call.function.name == #tool_name {
                                <#name as ::wasmind_actor_utils::tools::Tool>::handle_call(&mut *self.tool.borrow_mut(), execute_tool_call)
                            }
                        }
                    }
                }
            }

            fn destructor(&self) {}
        }

        pub struct Component;

        impl crate::bindings::exports::wasmind::actor::actor::Guest for Component {
            type Actor = #actor_name;
        }

       crate::bindings::export!(Component with_types_in bindings);
    };

    TokenStream::from(expanded)
}

fn parse_tool_attributes(input: &DeriveInput) -> ToolAttributes {
    let mut name = None;
    let mut description = None;
    let mut schema = None;

    for attr in &input.attrs {
        if attr.path().is_ident("tool") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    let value = meta.value()?;
                    let s: Lit = value.parse()?;
                    if let Lit::Str(lit_str) = s {
                        name = Some(lit_str.value());
                    }
                } else if meta.path.is_ident("description") {
                    let value = meta.value()?;
                    let s: Lit = value.parse()?;
                    if let Lit::Str(lit_str) = s {
                        description = Some(lit_str.value());
                    }
                } else if meta.path.is_ident("schema") {
                    let value = meta.value()?;
                    let s: Lit = value.parse()?;
                    if let Lit::Str(lit_str) = s {
                        schema = Some(lit_str.value());
                    }
                }
                Ok(())
            })
            .ok();
        }
    }

    ToolAttributes {
        name: name.unwrap_or_else(|| input.ident.to_string()),
        description: description.unwrap_or_else(|| format!("{} tool", input.ident)),
        schema: schema.unwrap_or_else(|| "{}".to_string()),
    }
}
