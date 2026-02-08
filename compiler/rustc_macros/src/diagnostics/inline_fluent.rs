use quote::quote;
use syn::{LitStr, parse_macro_input};

pub(crate) fn inline_fluent(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let inline = parse_macro_input!(input as LitStr);
    quote! {
        rustc_errors::DiagMessage::Inline(std::borrow::Cow::Borrowed(#inline))
    }
    .into()
}
