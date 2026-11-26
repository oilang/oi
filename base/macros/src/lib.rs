use proc_macro::{Span, TokenStream};

/// Expands `oi!` into an `indoc!` of its body's source.
#[proc_macro]
pub fn oi(input: TokenStream) -> TokenStream {
	if input.to_string().starts_with("r#") {
		return format!("::indoc::indoc!({input})").parse().unwrap();
	}

	// rust-analyzer's proc-macro server has no source text
	let Some(call) = Span::call_site().source_text() else {
		return "\"\"".parse().unwrap();
	};

	let body = call[call.find('!').unwrap() + 1..].trim();
	let body = &body[1..body.len() - 1];
	let mut hashes = "#".to_string();
	while body.contains(&format!("\"{hashes}")) {
		hashes.push('#');
	}

	format!("::indoc::indoc!(r{hashes}\"{body}\"{hashes})").parse().unwrap()
}
