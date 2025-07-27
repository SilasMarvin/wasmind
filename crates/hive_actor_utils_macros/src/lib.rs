use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Lit, parse_macro_input};

#[proc_macro]
pub fn generate_actor_trait(_input: TokenStream) -> TokenStream {
    let expanded = quote! {
        trait GeneratedActorTrait {
            fn new(scope: String) -> Self;

            fn handle_message(&mut self, message: crate::bindings::exports::hive::actor::actor::MessageEnvelope);

            fn destructor(&mut self) {}


            fn broadcast<T: ToString, S: ::hive_actor_utils::actors::macros::__private::serde::ser::Serialize>(message_type: T, payload: S) -> Result<(), ::hive_actor_utils::actors::macros::__private::serde_json::Error> {
                Ok(crate::bindings::hive::actor::messaging::broadcast(
                    &message_type.to_string(),
                    &::hive_actor_utils::tools::macros::__private::serde_json::to_string(&
                        payload
                    )?.into_bytes()
                ))
            }

            fn parse_as<S: ::hive_actor_utils::actors::macros::__private::serde::de::DeserializeOwned>(message_type: &str, msg: &crate::bindings::exports::hive::actor::actor::MessageEnvelope) -> Option<S> {
                if let Ok(json_string) = str::from_utf8(&msg.payload) && message_type == &msg.message_type {
                    ::hive_actor_utils::tools::macros::__private::serde_json::from_str::<S>(json_string).ok()
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
    let actor_name = syn::Ident::new(&format!("{}Actor", name), name.span());

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

        impl crate::bindings::exports::hive::actor::actor::GuestActor for #actor_name {
            fn new(scope: String) -> Self {
                let actor = <#name as GeneratedActorTrait>::new(scope);
                Self {
                    actor: std::cell::RefCell::new(actor)
                }
            }

            fn handle_message(&self, message: crate::bindings::exports::hive::actor::actor::MessageEnvelope) {
               self.actor.borrow_mut().handle_message(message);
            }

            fn destructor(&self) {
               self.actor.borrow_mut().destructor();
            }
        }

        pub struct Component;

        impl crate::bindings::exports::hive::actor::actor::Guest for Component {
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
    let actor_name = syn::Ident::new(&format!("{}Actor", name), name.span());

    // Parse attributes
    let attrs = parse_tool_attributes(&input);
    let tool_name = &attrs.name;
    let tool_desc = &attrs.description;
    let tool_schema = &attrs.schema;

    if let Err(e) = serde_json::from_str::<serde_json::Value>(&attrs.schema) {
        return TokenStream::from(
            syn::Error::new_spanned(&input, format!("Invalid JSON schema: {}", e))
                .to_compile_error(),
        );
    }

    let expanded = quote! {
        // Ensure the ToolTrait is implemented
        const _: () = {
            fn assert_impl_tool<T: ::hive_actor_utils::tools::Tool>() {}
            fn assert() {
                assert_impl_tool::<#name>();
            }
        };


        // Actor wrapper type
        pub struct #actor_name {
            scope: String,
            tool: std::cell::RefCell<#name>
        }

        impl crate::bindings::exports::hive::actor::actor::GuestActor for #actor_name {
            fn new(scope: String) -> Self {
                let s = Self {
                    scope: scope.clone(),
                    tool: std::cell::RefCell::new(<#name as ::hive_actor_utils::tools::Tool>::new())
                };

                use ::hive_actor_utils::common_messages::CommonMessage;

                crate::bindings::hive::actor::messaging::broadcast(
                    ::hive_actor_utils::common_messages::tools::ToolsAvailable::MESSAGE_TYPE,
                    &::hive_actor_utils::tools::macros::__private::serde_json::to_string(&
                        ::hive_actor_utils::common_messages::tools::ToolsAvailable {
                            tools: vec![
                                ::hive_actor_utils::tools::macros::__private::hive_llm_client::types::Tool {
                                    tool_type: "function".to_string(),
                                    function: ::hive_actor_utils::tools::macros::__private::hive_llm_client::types::ToolFunctionDefinition {
                                        name: #tool_name.to_string(),
                                        description: #tool_desc.to_string(),
                                        parameters: ::hive_actor_utils::tools::macros::__private::serde_json::from_str(& #tool_schema).expect("schema should be valid JSON")
                                    }
                                }
                            ]
                        }
                    ).unwrap().into_bytes()
                );

                s
            }

            fn handle_message(&self, message: crate::bindings::exports::hive::actor::actor::MessageEnvelope) {
                use ::hive_actor_utils::common_messages::CommonMessage;
                if message.message_type == ::hive_actor_utils::common_messages::tools::ExecuteTool::MESSAGE_TYPE {
                    if let Ok(json_string) = String::from_utf8(message.payload) {
                        if let Ok(execute_tool_call) = ::hive_actor_utils::tools::macros::__private::serde_json::from_str::<::hive_actor_utils::common_messages::tools::ExecuteTool>(&json_string) {
                            <#name as ::hive_actor_utils::tools::Tool>::handle_call(&mut *self.tool.borrow_mut(), execute_tool_call)
                        }
                    }
                }
            }

            fn destructor(&self) {}
        }

        pub struct Component;

        impl crate::bindings::exports::hive::actor::actor::Guest for Component {
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
