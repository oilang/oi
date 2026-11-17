use crate::common::{Run, oi, stderr};

#[test]
fn semantic_tokens_come_from_the_lexer() {
	let msgs = [
		r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
		r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
		r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///a.oi","text":"@test\nf :: fn() { # é\n\t:ok\n}\n"}}}"#,
		r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/semanticTokens/full","params":{"textDocument":{"uri":"file:///a.oi"}}}"#,
		r#"{"jsonrpc":"2.0","id":3,"method":"shutdown"}"#,
		r#"{"jsonrpc":"2.0","method":"exit"}"#,
	];
	let input: String = msgs
		.iter()
		.map(|msg| format!("Content-Length: {}\r\n\r\n{msg}", msg.len()))
		.collect();
	let out = oi(&["lsp"]).run(Some(&input));
	assert!(out.status.success(), "lsp failed:\n{}", stderr(&out));

	// decorator `@` `test`, operator `::`, keyword `fn`, utf-16 comment `# é`, atom `:ok`
	let data = "[0,0,1,6,0,0,1,4,6,0,1,2,2,4,0,0,3,2,1,0,0,7,3,0,0,1,1,3,5,0]";
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(stdout.contains(data), "{stdout}");
}
