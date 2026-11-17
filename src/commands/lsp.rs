use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;

use logos::Logos;
use lsp_server::{Connection, ErrorCode, Message, Notification, Response};
use oi::Reported;
use oi::driver::fail;
use oi::lexer::Token;
use serde_json::{Value, json};

const LEGEND: [&str; 8] = [
	"comment",
	"keyword",
	"string",
	"number",
	"operator",
	"enumMember",
	"decorator",
	"parameter",
];

/// Serve the language server over stdio.
pub fn serve() -> Result<(), Reported> {
	run().map_err(|e| fail(format!("lsp: {e}")))
}

fn run() -> Result<(), Box<dyn Error + Sync + Send>> {
	let (conn, io) = Connection::stdio();
	conn.initialize(json!({
		"textDocumentSync": { "openClose": true, "change": 1, "save": true },
		"semanticTokensProvider": { "legend": { "tokenTypes": LEGEND, "tokenModifiers": [] }, "full": true },
	}))?;

	let (checks, entries) = mpsc::channel::<PathBuf>();
	let sender = conn.sender.clone();
	let checker = std::thread::spawn(move || {
		let mut published = HashMap::new();
		for entry in entries {
			for msg in check(&entry, &mut published) {
				sender.send(msg).ok();
			}
		}
	});

	let mut docs: HashMap<String, String> = HashMap::new();
	for msg in &conn.receiver {
		match msg {
			Message::Request(req) => {
				if conn.handle_shutdown(&req)? {
					break;
				}
				let resp = if req.method == "textDocument/semanticTokens/full" {
					let src = req.params["textDocument"]["uri"].as_str().and_then(|uri| docs.get(uri));
					Response::new_ok(
						req.id,
						json!({ "data": src.map_or_else(Vec::new, |src| highlight(src)) }),
					)
				} else {
					Response::new_err(req.id, ErrorCode::MethodNotFound as i32, req.method)
				};
				conn.sender.send(resp.into())?;
			}

			Message::Notification(note) => {
				let doc = &note.params["textDocument"];
				let uri = doc["uri"].as_str().unwrap_or_default();
				let text = match note.method.as_str() {
					"textDocument/didOpen" => doc["text"].as_str(),
					// full sync, so the last change is the whole document
					"textDocument/didChange" => note.params["contentChanges"]
						.as_array()
						.and_then(|changes| changes.last()?["text"].as_str()),
					"textDocument/didClose" => {
						docs.remove(uri);
						None
					}
					_ => None,
				};

				if let Some(text) = text {
					docs.insert(uri.into(), text.into());
				}

				if matches!(note.method.as_str(), "textDocument/didOpen" | "textDocument/didSave")
					&& let Some(file) = uri.strip_prefix("file://")
				{
					checks.send(entry(Path::new(file)))?;
				}
			}

			Message::Response(_) => {}
		}
	}

	drop(checks);
	checker.join().ok();

	drop(conn);
	io.join()?;

	Ok(())
}

/// Classify a token as an index into `LEGEND`.
fn kind(tok: Token, text: &str, prev: Option<Token>) -> Option<usize> {
	use Token::*;
	let name = match tok {
		Ident(_) if prev == Some(At) => "decorator",
		Error(_) | Ident(_) | LParen | RParen | LBrace | RBrace | LBracket | RBracket | Dot | Colon | Comma
		| Backtick | Semicolon => return None,
		BlockComment | Comment | Doc(_) => "comment",
		Int(_) | Float(_) => "number",
		String(_) | RawString(_) | PathString(_) | Rune(_) => "string",
		Bool(_) | Atom(_) => "enumMember",
		At => "decorator",
		Dollar => "parameter",
		_ if text.starts_with(char::is_alphabetic) => "keyword",
		_ => "operator",
	};
	LEGEND.iter().position(|legend| *legend == name)
}

/// Lex `src` into semantic tokens, one per line a lexeme spans.
fn highlight(src: &str) -> Vec<u32> {
	let utf16 = |text: &str| text.encode_utf16().count() as u32;
	let (mut data, mut last, mut prev) = (vec![], 0, None);

	for (tok, span) in Token::lexer(src).spanned() {
		let tok = tok.ok();

		if let Some(ty) = tok.and_then(|tok| kind(tok, &src[span.clone()], prev)) {
			let mut start = span.start;

			for line in src[span].split('\n') {
				if !line.trim().is_empty() {
					let gap = &src[last..start];
					let column = gap.rsplit('\n').next().unwrap_or_default();

					data.extend([
						gap.matches('\n').count() as u32,
						utf16(column),
						utf16(line),
						ty as u32,
						0,
					]);

					last = start;
				}

				start += line.len() + 1;
			}
		}

		prev = tok;
	}

	data
}

/// Find the program a given file belongs to.
/// Either the nearest `main.oi` above it or the file itself.
fn entry(file: &Path) -> PathBuf {
	let mut mains = file.ancestors().skip(1).map(|dir| dir.join("main.oi"));
	mains.find(|main| main.exists()).unwrap_or_else(|| file.into())
}

/// Check `entry` in a child process, publishing every file it reported now or last time.
fn check(entry: &Path, published: &mut HashMap<PathBuf, Vec<String>>) -> Vec<Message> {
	let args = ["--error-format", "json", "run", "--check"];
	let out = std::env::current_exe().and_then(|exe| Command::new(exe).args(args).arg(entry).output());
	let stderr = out.map(|out| out.stderr).unwrap_or_default();

	let stale = published.remove(entry).unwrap_or_default();
	let mut files: HashMap<_, Vec<_>> = stale.into_iter().map(|uri| (uri, vec![])).collect();
	for line in String::from_utf8_lossy(&stderr).lines() {
		let Ok(diag) = serde_json::from_str::<Value>(line) else {
			continue;
		};
		let Some(file) = diag["file"].as_str() else {
			continue;
		};

		let message = ["message", "label", "note"].iter().filter_map(|part| diag[part].as_str());
		let text = diag["text"].as_str().unwrap_or_default();
		let position = |end: &str, line: Option<&str>| {
			let [row, col] = ["line", "col"].map(|at| diag["span"][end][at].as_u64().unwrap_or(1).saturating_sub(1));
			let before = line.unwrap_or_default().chars().take(col as usize);
			json!({ "line": row, "character": before.map(char::len_utf16).sum::<usize>() })
		};

		files.entry(format!("file://{file}")).or_default().push(json!({
			"range": {
				"start": position("start", text.split('\n').next()),
				"end": position("end", text.rsplit('\n').next()),
			},
			"severity": 1,
			"source": "oi",
			"message": message.collect::<Vec<_>>().join("\n"),
		}));
	}

	published.insert(entry.into(), files.keys().cloned().collect());

	let publish = |(uri, diagnostics)| json!({ "uri": uri, "diagnostics": diagnostics });
	let note = |params| Notification::new("textDocument/publishDiagnostics".into(), params).into();
	files.into_iter().map(publish).map(note).collect()
}
