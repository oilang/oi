use std::io::IsTerminal as _;

use oi::Reported;
use oi::ast::Expr;
use oi::driver::{DebugOpts, run_source};
use oi::loader::parse_file;

const HELP: &str = indoc::indoc! {"
	The Oi REPL.

	Runs code you input as if it were running a script.
	Definitions and bindings persist across lines, plain statements run once.
	If you run into any issues `:clear` your session.

	Commands:
		:h, :help -> help
		:q, :quit -> quit
		:x, :exit -> quit
		:c, :clear -> clear session context
"};

pub fn run() -> Result<(), Reported> {
	eprintln!("Oi! Type :help for help.");
	// reedline needs a tty, so piped stdin reads plain lines
	let mut next: Box<dyn FnMut() -> Option<String>> = if std::io::stdin().is_terminal() {
		let mut rl = reedline();
		let prompt = reedline::DefaultPrompt::new(
			reedline::DefaultPromptSegment::Basic("oi".to_string()),
			reedline::DefaultPromptSegment::Empty,
		);
		Box::new(move || {
			loop {
				match rl.read_line(&prompt) {
					Ok(reedline::Signal::Success(line)) => return Some(line),
					Ok(reedline::Signal::CtrlC) => {}
					Ok(_) => return None,
					Err(e) => {
						eprintln!("oi: {e}");
						return None;
					}
				}
			}
		})
	} else {
		let mut lines = std::io::stdin().lines();
		Box::new(move || lines.next()?.ok())
	};

	let mut session = String::new();
	while let Some(line) = next() {
		match line.trim() {
			"" => {}
			":help" | ":h" => eprint!("{HELP}"),
			":quit" | ":q" | ":exit" | ":x" => break,
			":clear" | ":c" => session.clear(),
			_ => {
				let src = format!("{session}{line}\n");
				let entry = vec![("<repl>".into(), src)];
				if run_source(entry, std::path::Path::new("."), DebugOpts::default()).is_ok() {
					session.push_str(&defs(&line));
				}
			}
		}
	}
	eprintln!("goodbye");
	Ok(())
}

fn reedline() -> reedline::Reedline {
	let commands = [":help", ":h", ":quit", ":q", ":exit", ":x", ":clear", ":c"];
	reedline::Reedline::create()
		.with_edit_mode(Box::new(reedline::Vi::new(
			reedline::default_vi_insert_keybindings(),
			reedline::default_vi_normal_keybindings(),
		)))
		.with_highlighter(Box::new(reedline::ExampleHighlighter::new(
			commands.map(String::from).to_vec(),
		)))
		.with_mouse_click(reedline::MouseClickMode::EnabledWithOsc133)
		.use_bracketed_paste(true)
}

// The source of `line`'s definitions, one per line. Ran every subsequent turn.
fn defs(line: &str) -> String {
	parse_file(line, 0)
		.unwrap_or_default()
		.iter()
		.filter(|(e, _)| is_def(e))
		.map(|(_, s)| format!("{}\n", &line[s.start..s.end]))
		.collect()
}

fn is_def(e: &Expr) -> bool {
	match e {
		Expr::Pub(b) | Expr::Annotated(_, b) => is_def(&b.0),
		Expr::Bind { .. } | Expr::Use { .. } | Expr::Claim { .. } | Expr::MacroDef { .. } => true,
		_ => e.def_name().is_some(),
	}
}
