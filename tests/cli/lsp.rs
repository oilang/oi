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
