use proc_macro::TokenStream;
use proc_macro_error::{abort, proc_macro_error};
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

#[proc_macro_derive(ActorContext)]
#[proc_macro_error]
pub fn derive_actor_context(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    if !matches!(input.data, syn::Data::Struct(_)) {
        abort!(input, "ActorContext can only be derived for structs");
    }

    let expanded = quote! {
        impl ActorContext for #name {
            fn get_scope(&self) -> &Scope {
                &self.scope
            }

            fn get_tx(&self) -> tokio::sync::broadcast::Sender<ActorMessage> {
                self.tx.clone()
            }

            fn get_rx(&self) -> tokio::sync::broadcast::Receiver<ActorMessage> {
                self.get_tx().subscribe()
            }
        }
    };

    TokenStream::from(expanded)
}
