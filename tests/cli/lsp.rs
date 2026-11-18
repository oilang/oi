use std::io::{BufRead, BufReader, Read, Write};
use std::process::Stdio;

use crate::common::oi;

/// Run `msgs` through an `oi lsp` session, returning its stdout.
fn session(msgs: &[&str]) -> String {
	session_until(msgs, "")
}

/// Like `session`, shutting down only once stdout mentions `until`.
fn session_until(msgs: &[&str], until: &str) -> String {
	let frame = |msgs: &[&str]| -> String {
		let frame = |msg: &&str| format!("Content-Length: {}\r\n\r\n{msg}", msg.len());
		msgs.iter().map(frame).collect()
	};
	let init = [
		r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
		r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
	];
	let exit = [
		r#"{"jsonrpc":"2.0","id":3,"method":"shutdown"}"#,
		r#"{"jsonrpc":"2.0","method":"exit"}"#,
	];

	let mut lsp = oi(&["lsp"]).stdin(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
	let mut stdin = lsp.stdin.take().unwrap();
	let mut stdout = BufReader::new(lsp.stdout.take().unwrap());
	stdin.write_all(frame(&[&init, msgs].concat()).as_bytes()).unwrap();

	let mut seen = vec![];
	while !String::from_utf8_lossy(&seen).contains(until) {
		let read = stdout.read_until(b'}', &mut seen).unwrap();
		assert_ne!(read, 0, "{}", String::from_utf8_lossy(&seen));
	}

	stdin.write_all(frame(&exit).as_bytes()).unwrap();
	drop(stdin);
	stdout.read_to_end(&mut seen).unwrap();
	assert!(lsp.wait().unwrap().success());
	String::from_utf8_lossy(&seen).into()
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
fn hover_under_cursor() {
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
		r#"{
			"jsonrpc":"2.0",
			"id":4,
			"method":"textDocument/definition",
			"params":{"textDocument":{"uri":"file:///a.oi"},"position":{"line":1,"character":2}}
		}"#,
	]);

	let sqrt = r#""value":"```oi\n## Square root.\npub sqrt[T] :: fn(n: T) float {\n```""#;
	let main = r#"{"range":{"end":{"character":31,"line":1},"start":{"character":0,"line":1}},"uri":"file:///a.oi"}"#;
	assert!(stdout.contains(sqrt) && stdout.contains(main), "{stdout}");
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
	// never saved, so the check reads the overlay
	let stdout = session_until(
		&[r#"{
			"jsonrpc":"2.0",
			"method":"textDocument/didOpen",
			"params":{"textDocument":{"uri":"file:///a.oi","text":"main :: fn() {\n\tx: int = \"s\"\n}"}}
		}"#],
		"publishDiagnostics",
	);

	let diag = r#"{"diagnostics":[{"message":"expected int, got string\ndoes not match the declared type","range":{"end":{"character":13,"line":1},"start":{"character":10,"line":1}},"severity":1,"source":"oi"}],"uri":"file:///a.oi"}"#;
	assert!(stdout.contains(diag), "{stdout}");
}
