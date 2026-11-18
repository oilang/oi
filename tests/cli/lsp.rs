use crate::common::{Project, Run, oi, stderr};

/// Run `msgs` through an `oi lsp` session, returning its stdout.
fn session(msgs: &[&str]) -> String {
	let msgs = [
		&[
			r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
			r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
		],
		msgs,
		&[
			r#"{"jsonrpc":"2.0","id":3,"method":"shutdown"}"#,
			r#"{"jsonrpc":"2.0","method":"exit"}"#,
		],
	]
	.concat();
	let input: String = msgs
		.iter()
		.map(|msg| format!("Content-Length: {}\r\n\r\n{msg}", msg.len()))
		.collect();
	let out = oi(&["lsp"]).run(Some(&input));
	assert!(out.status.success(), "lsp failed:\n{}", stderr(&out));
	String::from_utf8_lossy(&out.stdout).into()
}

#[test]
fn document_symbols_list_top_level_decls() {
	let stdout = session(&[
		r#"{
			"jsonrpc":"2.0",
			"method":"textDocument/didOpen",
			"params":{"textDocument":{"uri":"file:///a.oi","text":"pub sqrt[T] :: fn() {\n\tx :: 1\n}\nCAP : int : 1"}}
		}"#,
		r#"{
			"jsonrpc":"2.0",
			"id":2,
			"method":"textDocument/documentSymbol",
			"params":{"textDocument":{"uri":"file:///a.oi"}}
		}"#,
	]);

	let sqrt = r#"{"kind":12,"name":"sqrt","range":{"end":{"character":8,"line":0},"start":{"character":4,"line":0}}"#;
	let cap = r#"{"kind":14,"name":"CAP","range":{"end":{"character":3,"line":3},"start":{"character":0,"line":3}}"#;
	let top_level = stdout.contains(sqrt) && stdout.contains(cap) && !stdout.contains(r#""name":"x""#);
	assert!(top_level, "{stdout}");
}

#[test]
fn hover_shows_docs_and_signature() {
	let stdout = session(&[
		r#"{
			"jsonrpc":"2.0",
			"method":"textDocument/didOpen",
			"params":{"textDocument":{"uri":"file:///a.oi","text":"use math\nmain :: fn() { math.sqrt(2.0) }"}}
		}"#,
		r#"{
			"jsonrpc":"2.0",
			"id":2,
			"method":"textDocument/hover",
			"params":{"textDocument":{"uri":"file:///a.oi"},"position":{"line":1,"character":21}}
		}"#,
	]);

	let sqrt = r#""value":"```oi\n## Square root.\npub sqrt[T] :: fn(n: T) float {\n```""#;
	assert!(stdout.contains(sqrt), "{stdout}");
}

#[test]
fn semantic_tokens_come_from_the_lexer() {
	let stdout = session(&[
		r#"{
			"jsonrpc":"2.0",
			"method":"textDocument/didOpen",
			"params":{"textDocument":{"uri":"file:///a.oi","text":"@test\nf :: fn() { # é\n\t:ok\n}\nx! \"a{1}b\""}}
		}"#,
		r#"{
			"jsonrpc":"2.0",
			"id":2,
			"method":"textDocument/semanticTokens/full",
			"params":{"textDocument":{"uri":"file:///a.oi"}}
		}"#,
	]);

	// decorator `@` `test`, operator `::`, keyword `fn`, utf-16 comment `# é`, atom `:ok`, macro `x!`, interpolated `1`
	let data = "[0,0,1,6,0,0,1,4,6,0,1,2,2,4,0,0,3,2,1,0,0,7,3,0,0,1,1,3,5,0,2,0,2,8,0,0,3,3,2,0,0,3,1,3,0,0,1,3,2,0]";
	assert!(stdout.contains(data), "{stdout}");
}

#[test]
fn diagnostics_publish_on_open() {
	let project = Project::new().main("main :: fn() {\n\tx: int = \"s\"\n}");
	let uri = format!("file://{}", project.as_ref().join("main.oi").display());
	let stdout = session(&[&format!(
		r#"{{
			"jsonrpc":"2.0",
			"method":"textDocument/didOpen",
			"params":{{"textDocument":{{"uri":"{uri}","text":""}}}}
		}}"#
	)]);

	let diag = r#"{"message":"expected int, got string\ndoes not match the declared type","range":{"end":{"character":13,"line":2},"start":{"character":10,"line":2}},"severity":1,"source":"oi"}"#;
	let publish = format!(r#"{{"diagnostics":[{diag}],"uri":"{uri}"}}"#);
	assert!(stdout.contains(&publish), "{stdout}");
}
