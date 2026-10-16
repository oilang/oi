use std::collections::HashSet;
use std::io::IsTerminal as _;

use indoc::eprintdoc;
use oi::Reported;
use oi::ast::Expr;
use oi::driver::{DebugOpts, run_snippet};
use oi::lexer::{lex_at, prescan};
use oi::loader::{core_source, parse_file};

use crate::cli::VERSION;

const HELP: &str = indoc::indoc! {"
	The Oi REPL.

	Runs code you input as if it were running a script.
	Definitions and bindings persist across lines, plain statements run once.
	If you run into any issues `:clear` your session.

	Commands:
		:h, :help -> help
		:h <name> -> docs of an item, e.g. :h math.sqrt
		:q, :quit -> quit
		:x, :exit -> quit
		:c, :clear -> clear session context
"};

pub fn run() -> Result<(), Reported> {
	eprintdoc! {r"

    ███████     ███  ███  |
  ███░░░░░███  ░░░  ░███  |
 ███     ░░███ ████ ░███  |  Oi {VERSION}
░███      ░███░░███ ░███  |
░███      ░███ ░███ ░███  |  Docs: https://oilang.github.io/oi
░░███     ███  ░███ ░░░   |  Type :help for help, :quit to quit.
 ░░░███████░   █████ ███  |
   ░░░░░░░    ░░░░░ ░░░   |
	"}

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
			topic if topic.starts_with(":h ") => help_topic(topic[3..].trim(), &session),
			":help" | ":h" => eprint!("oi {VERSION}\n{HELP}"),
			":quit" | ":q" | ":exit" | ":x" => break,
			":clear" | ":c" => session.clear(),
			_ => {
				let src = format!("{session}{line}\n");
				let raw = prescan(&lex_at(&src, 0)).0.into_iter().collect();
				if run_snippet("<repl>", src, DebugOpts::default()).is_ok() {
					session.push_str(&defs(&line, &raw));
				}
			}
		}
	}
	eprintln!("goodbye");
	Ok(())
}

// Prints docs and signature for a name, from the session or a core module.
fn help_topic(name: &str, session: &str) {
	let (module, item) = name.rsplit_once('.').unwrap_or(("", name));
	let source = if module.is_empty() {
		session
	} else {
		core_source(module).unwrap_or_default()
	};
	let lines: Vec<_> = source.lines().collect();
	let is_def = |line: &&str| {
		let rest = line.trim_start_matches("pub ").strip_prefix(item);
		rest.is_some_and(|rest| rest.trim_start().starts_with("::"))
	};
	let Some(at) = lines.iter().rposition(is_def) else {
		return eprintln!("no docs for `{name}`");
	};
	let docs = lines[..at].iter().rev().take_while(|line| line.starts_with("##")).count();
	eprintln!("{}", lines[at - docs..=at].join("\n"));
}

fn reedline() -> reedline::Reedline {
	let commands = [":help", ":h", ":quit", ":q", ":exit", ":x", ":clear", ":c"];
	let mut insert = reedline::default_vi_insert_keybindings();
	insert.add_binding(
		reedline::KeyModifiers::CONTROL,
		reedline::KeyCode::Char('u'),
		reedline::ReedlineEvent::Edit(vec![reedline::EditCommand::CutFromLineStart]),
	);
	reedline::Reedline::create()
		.with_edit_mode(Box::new(reedline::Vi::new(
			insert,
			reedline::default_vi_normal_keybindings(),
		)))
		.with_highlighter(Box::new(reedline::ExampleHighlighter::new(
			commands.map(String::from).to_vec(),
		)))
		.with_mouse_click(reedline::MouseClickMode::EnabledWithOsc133)
		.use_bracketed_paste(true)
}

// The source of `line`'s definitions, one per line. Ran every subsequent turn.
fn defs(line: &str, raw: &HashSet<String>) -> String {
	parse_file(line, 0, raw)
		.unwrap_or_default()
		.iter()
		.filter(|(e, _)| is_def(e))
		.map(|(_, s)| format!("{}\n", &line[s.start..s.end]))
		.collect()
}

fn is_def(e: &Expr) -> bool {
	match e {
		Expr::Pub(_, b) | Expr::Annotated(_, b) => is_def(&b.0),
		Expr::Bind { .. } | Expr::Use { .. } | Expr::Claim { .. } | Expr::MacroDef { .. } => true,
		_ => e.def_name().is_some(),
	}
}
