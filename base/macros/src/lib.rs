use proc_macro::{Literal, Span, TokenStream};

/// Expands `oi! { .. }` into an `indoc!` of its body's source text.
#[proc_macro]
pub fn oi(_: TokenStream) -> TokenStream {
	let call = Span::call_site().source_text().unwrap();
	let body = &call[call.find('{').unwrap() + 1..call.len() - 1];
	format!("::indoc::indoc!({})", Literal::string(body)).parse().unwrap()
}
