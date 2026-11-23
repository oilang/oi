use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use logos::Logos;
use lsp_server::{Connection, ErrorCode, Message, Notification, Response};
use oi::Reported;
use oi::driver::fail;
use oi::lexer::{Token, lex_at};
use oi::loader::{Program, load, scan};
use serde_json::{Value, json};

use super::repl::{definition, lookup};
use super::run::root;

const LEGEND: [&str; 9] = [
	"comment",
	"keyword",
	"string",
	"number",
	"operator",
	"enumMember",
	"decorator",
	"parameter",
	"macro",
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
		"documentSymbolProvider": true,
		"hoverProvider": true,
		"definitionProvider": true,
	}))?;

	let docs = Arc::new(Mutex::new(HashMap::<PathBuf, String>::new()));
	let (checks, requests) = mpsc::channel::<PathBuf>();
	let (sender, unsaved) = (conn.sender.clone(), docs.clone());
	let checker = std::thread::spawn(move || {
		let (mut queue, mut published) = (vec![], HashMap::new());
		while let Some(entry) = queue.pop().or_else(|| requests.recv().ok()) {
			// collect requests until edits pause
			let pause = Duration::from_millis(400);
			queue.extend(std::iter::from_fn(|| requests.recv_timeout(pause).ok()));
			queue.retain(|queued| *queued != entry);

			let overlay = serde_json::to_string(&*unsaved.lock().unwrap()).unwrap_or_default();
			let mut oi = |args: &[&str]| output(args, &entry, &overlay, &requests, &mut queue);
			let Some(mut stderr) = oi(&["run", "--check", &entry.to_string_lossy()]) else {
				continue;
			};

			// give strays a hint and a check of their module
			let strays = strays(&entry, &stderr, &unsaved.lock().unwrap());
			for file in strays.keys() {
				let hint = json!({ "file": file, "severity": 3, "message": "not reached from main.oi" });
				stderr += &format!("{hint}\n");
			}
			let uses: HashSet<_> = strays.values().collect();
			let more: Option<String> = uses.iter().map(|source| oi(&["exec", "--check", source])).collect();
			let Some(more) = more else {
				continue;
			};

			for msg in check(&entry, &(stderr + &more), &mut published) {
				sender.send(msg).ok();
			}
		}
	});

	for msg in &conn.receiver {
		let mut docs = docs.lock().unwrap();
		match msg {
			Message::Request(req) => {
				if conn.handle_shutdown(&req)? {
					break;
				}

				let file = req.params["textDocument"]["uri"].as_str().and_then(file_path);
				let src = file.and_then(|file| docs.get(&file)).map_or("", String::as_str);
				let resp = match req.method.as_str() {
					"textDocument/semanticTokens/full" => Response::new_ok(req.id, json!({ "data": highlight(src) })),
					"textDocument/documentSymbol" => Response::new_ok(req.id, symbols(src)),
					"textDocument/hover" => {
						let found = program_at(&docs, &req.params).and_then(|(program, name)| lookup(&program, &name));
						let hover = |text| json!({ "contents": { "kind": "markdown", "value": format!("```oi\n{text}\n```") } });
						Response::new_ok(req.id, found.map(hover))
					}
					"textDocument/definition" => {
						let found = program_at(&docs, &req.params).and_then(|(program, name)| {
							let span = definition(&program, &name)?;
							let (file, ..) = program.map.locate_span(span.into_range());
							let at = |offset| position(program.map.before(offset));
							let range = json!({ "start": at(span.start), "end": at(span.end) });
							// embedded core has no file
							file.starts_with('/')
								.then_some(json!({ "uri": file_uri(file), "range": range }))
						});
						Response::new_ok(req.id, found)
					}
					_ => Response::new_err(req.id, ErrorCode::MethodNotFound as i32, req.method),
				};
				conn.sender.send(resp.into())?;
			}

			Message::Notification(note) => {
				let doc = &note.params["textDocument"];
				let Some(file) = doc["uri"].as_str().and_then(file_path) else {
					continue;
				};

				let text = match note.method.as_str() {
					"textDocument/didOpen" => doc["text"].as_str(),
					// full sync, so the last change is the whole document
					"textDocument/didChange" => note.params["contentChanges"]
						.as_array()
						.and_then(|changes| changes.last()?["text"].as_str()),
					"textDocument/didClose" => {
						docs.remove(&file);
						None
					}
					_ => None,
				};

				let rust = file.extension().is_some_and(|ext| ext == "rs");
				if let Some(text) = text {
					docs.insert(file.clone(), if rust { embedded(text) } else { text.into() });
				}

				if !rust && (text.is_some() || note.method == "textDocument/didSave") {
					checks.send(entry(&file))?;
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
		Ident(_) if text.ends_with('!') => "macro",
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

	let mut toks = vec![];
	for (tok, span) in Token::lexer(src).spanned() {
		let (tok, mut from) = (tok.ok(), span.start);
		if let Some(Token::String(_)) = tok {
			let code = lex_at(&src[span.clone()], from).into_iter();
			for (inner, at) in code.filter(|(_, at)| at.into_range().len() < span.len()) {
				toks.extend([(tok, from..at.start), (Some(inner), at.into_range())]);
				from = at.end;
			}
		}

		if tok == Some(Token::Not)
			&& let Some((Some(Token::Ident(_)), name)) = toks.last_mut()
			&& name.end == from
		{
			name.end = span.end;
			continue;
		}

		toks.push((tok, from..span.end));
	}

	for (tok, span) in toks {
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

/// Blank all but the bodies of `oi! { .. }` calls, keeping every position.
fn embedded(src: &str) -> String {
	let blank = |text: &str| text.replace(|ch| ch != '\n', " ");
	let (mut out, mut rest) = (String::new(), src);
	while let Some(at) = rest.find("oi! {") {
		let (call, body) = rest.split_at(at + 4);
		let mut depth = 0;
		let end = Token::lexer(body).spanned().find_map(|(tok, span)| {
			depth += match tok {
				Ok(Token::LBrace) => 1,
				Ok(Token::RBrace) => -1,
				_ => 0,
			};
			(depth == 0).then_some(span.end)
		});

		let (code, after) = body.split_at(end.unwrap_or(body.len()));
		out += &blank(call);
		out += code;
		rest = after;
	}
	out + &blank(rest)
}

/// List top-level declarations as document symbols.
fn symbols(src: &str) -> Vec<Value> {
	let (toks, mut found) = (lex_at(src, 0), vec![]);
	for (name, decl) in scan(&toks).decls {
		// lsp SymbolKind
		let kind = match toks.get(decl.end) {
			Some((Token::Fn, _)) => 12,
			Some((Token::Struct, _)) => 23,
			Some((Token::Enum, _)) => 10,
			Some((Token::Trait, _)) => 11,
			_ => 14,
		};
		let span = toks[decl.start].1;
		let range = json!({ "start": position(&src[..span.start]), "end": position(&src[..span.end]) });
		found.push(json!({ "name": name, "kind": kind, "range": range, "selectionRange": range }));
	}
	found
}

/// Find the LSP position at the end of `before`.
fn position(before: &str) -> Value {
	let column = before.rsplit('\n').next().unwrap_or_default();
	json!({ "line": before.matches('\n').count(), "character": column.encode_utf16().count() })
}

/// Load the program around the request's file, with the dotted name under its cursor.
fn program_at(docs: &HashMap<PathBuf, String>, params: &Value) -> Option<(Program, String)> {
	let file = file_path(params["textDocument"]["uri"].as_str()?)?;
	let src = docs.get(&file)?;
	let [row, column] = ["line", "character"].map(|key| params["position"][key].as_u64().unwrap_or_default() as usize);
	let line = src.split_inclusive('\n').take(row).map(str::len).sum::<usize>();
	let before: String = char::decode_utf16(src[line..].encode_utf16().take(column)).flatten().collect();
	let (toks, at) = (lex_at(src, 0), line + before.len());
	let end = toks
		.iter()
		.position(|(tok, span)| matches!(tok, Token::Ident(_)) && (span.start..=span.end).contains(&at))?;
	let mut start = end;
	while let [.., (Token::Ident(_), _), (Token::Dot, _)] = &toks[..start] {
		start -= 2;
	}

	// `load` reads modules from disk, so only the entry can be an open buffer
	let main = entry(&file);
	let text = docs.get(&main).cloned().or_else(|| std::fs::read_to_string(&main).ok())?;
	let program = load(vec![(main.display().to_string(), text)], root(&main)).ok()?;
	Some((program, src[toks[start].1.start..toks[end].1.end].into()))
}

/// Find the program a given file belongs to.
/// Either the nearest `main.oi` above it or the file itself.
fn entry(file: &Path) -> PathBuf {
	let mut mains = file.ancestors().skip(1).map(|dir| dir.join("main.oi"));
	mains.find(|main| main.exists()).unwrap_or_else(|| file.into())
}

/// Percent-decode a `file://` URI into its path.
fn file_path(uri: &str) -> Option<PathBuf> {
	let mut parts = uri.strip_prefix("file://")?.split('%');
	let mut bytes = parts.next()?.as_bytes().to_vec();
	for part in parts {
		bytes.push(u8::from_str_radix(part.get(..2)?, 16).ok()?);
		bytes.extend(&part.as_bytes()[2..]);
	}
	String::from_utf8(bytes).ok().map(PathBuf::from)
}

/// Percent-encode a path into a `file://` URI.
fn file_uri(path: &str) -> String {
	let encode = |byte: u8| match byte {
		_ if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) => char::from(byte).to_string(),
		_ => format!("%{byte:02X}"),
	};
	format!("file://{}", path.bytes().map(encode).collect::<String>())
}

/// Map open files of `entry`'s program that its check never loaded to the `use` that loads each.
fn strays(entry: &Path, stderr: &str, open: &HashMap<PathBuf, String>) -> HashMap<PathBuf, String> {
	let files = |line| serde_json::from_str::<Value>(line).ok()?["files"].as_array().cloned();
	let loaded = stderr.lines().find_map(files);
	let stray = |file: &&PathBuf| loaded.as_ref().is_some_and(|loaded| !loaded.contains(&json!(file)));
	let module_use = |file: &PathBuf| {
		let rel = file.strip_prefix(entry.parent()?).ok()?.with_extension("");
		let module = rel.parent().filter(|sub| *sub != Path::new("")).unwrap_or(&rel);
		let source = format!("use {}", module.display()).replace('/', ".");
		(self::entry(file) == entry).then_some((file.clone(), source))
	};
	open.keys().filter(stray).filter_map(module_use).collect()
}

/// Run `oi` with `args` from `entry`'s dir in a child process, returning its stderr.
/// Gives up on a newer request for `entry`, a 20s run, or shutdown.
fn output(
	args: &[&str],
	entry: &Path,
	overlay: &str,
	requests: &Receiver<PathBuf>,
	queue: &mut Vec<PathBuf>,
) -> Option<String> {
	let dir = entry.ancestors().find(|dir| dir.is_dir())?;
	let mut cmd = Command::new(std::env::current_exe().ok()?);
	cmd.current_dir(dir).args(["--error-format", "json", "--overlay"]);
	cmd.args(args).stdin(Stdio::piped());
	let mut child = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).spawn().ok()?;
	let pipe = child.stderr.take()?;
	let reader = std::thread::spawn(move || std::io::read_to_string(pipe));
	child.stdin.take()?.write_all(overlay.as_bytes()).ok();

	let started = Instant::now();
	while child.try_wait().ok()?.is_none() {
		let next = requests.recv_timeout(Duration::from_millis(50));
		let stop = match &next {
			Ok(next) => next == entry,
			Err(RecvTimeoutError::Timeout) => started.elapsed() > Duration::from_secs(20),
			Err(RecvTimeoutError::Disconnected) => {
				queue.clear();
				true
			}
		};
		queue.extend(next);
		if stop {
			child.kill().and_then(|_| child.wait()).ok();
			return None;
		}
	}
	reader.join().ok()?.ok()
}

/// Publish a check's diagnostics for every file `entry` reported now or last time.
fn check(entry: &Path, stderr: &str, published: &mut HashMap<PathBuf, Vec<String>>) -> Vec<Message> {
	let stale = published.remove(entry).unwrap_or_default();
	let mut files: HashMap<_, Vec<_>> = stale.into_iter().map(|uri| (uri, vec![])).collect();
	for line in stderr.lines() {
		let Ok(diag) = serde_json::from_str::<Value>(line) else {
			continue;
		};

		// `<exec>` and embedded core have no file to publish to
		let file = diag["file"].as_str().unwrap_or_default();
		let path: PathBuf = entry.with_file_name(file).components().collect();
		let Some(file) = path.to_str().filter(|_| path == entry || path.is_file()) else {
			continue;
		};

		let message = ["message", "label", "note"].iter().filter_map(|part| diag[part].as_str());
		let text = diag["text"].as_str().unwrap_or_default();
		let position = |end: &str, line: Option<&str>| {
			let [row, col] = ["line", "col"].map(|at| diag["span"][end][at].as_u64().unwrap_or(1).saturating_sub(1));
			let before = line.unwrap_or_default().chars().take(col as usize);
			json!({ "line": row, "character": before.map(char::len_utf16).sum::<usize>() })
		};

		files.entry(file_uri(file)).or_default().push(json!({
			"range": {
				"start": position("start", text.split('\n').next()),
				"end": position("end", text.rsplit('\n').next()),
			},
			"severity": diag["severity"].as_u64().unwrap_or(1),
			"source": "oi",
			"message": message.collect::<Vec<_>>().join("\n"),
		}));
	}

	published.insert(entry.into(), files.keys().cloned().collect());

	let publish = |(uri, diagnostics)| json!({ "uri": uri, "diagnostics": diagnostics });
	let note = |params| Notification::new("textDocument/publishDiagnostics".into(), params).into();
	files.into_iter().map(publish).map(note).collect()
}
