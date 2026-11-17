use crate::common::{Run, oi, stderr};

#[test]
fn initializes_and_shuts_down() {
	let msgs = [
		r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
		r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
		r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#,
		r#"{"jsonrpc":"2.0","method":"exit"}"#,
	];
	let input: String = msgs
		.iter()
		.map(|msg| format!("Content-Length: {}\r\n\r\n{msg}", msg.len()))
		.collect();
	let out = oi(&["lsp"]).run(Some(&input));
	assert!(out.status.success(), "lsp failed:\n{}", stderr(&out));
}
