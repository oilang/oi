use std::collections::HashSet;
use std::io::IsTerminal as _;
use std::path::Path;

use indoc::eprintdoc;
use oi::Reported;
use oi::ast::{Expr, Span, Spanned};
use oi::driver::{DebugOpts, run_snippet};
use oi::lexer::{lex_at, prescan};
use oi::loader::{core_source, load, parse_file};

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

// Prints docs and signature for a name, from the session or a module it names.
fn help_topic(name: &str, session: &str) {
	let module = name.split_once('.').map(|(head, _)| head).filter(|head| {
		core_source(head).is_some() || Path::new(head).is_dir() || Path::new(&format!("{head}.oi")).is_file()
	});
	let src = module.map_or(session.into(), |m| format!("{session}use {m}\n"));
	let Ok(program) = load(vec![("<repl>".into(), src)], Path::new(".")) else {
		return;
	};
	let Some(span) = program.modules.iter().rev().find_map(|m| find_def(&m.items, "", name)) else {
		return eprintln!("no docs for `{name}`");
	};
	let (.., text) = program.map.locate_span(span.into_range());
	let (above, indent) = program.map.before(span.start).rsplit_once('\n').unwrap_or_default();
	let docs = above.lines().rev().map(str::trim_start).skip_while(|l| l.starts_with('@'));
	let docs: Vec<_> = docs.take_while(|l| l.starts_with("##")).collect();
	docs.iter().rev().for_each(|doc| eprintln!("{doc}"));
	eprintln!("{}{}", indent.trim_start(), text.lines().next().unwrap_or_default());
}

// The last definition at the given dotted path.
fn find_def(items: &[Spanned<Expr>], prefix: &str, path: &str) -> Option<Span> {
	items.iter().rev().find_map(|item| {
		let item = match &item.0 {
			Expr::MacroCall { args, .. } => args.first()?,
			_ => item,
		};
		let def = Expr::peel_meta(item).2;
		let (name, members): (&str, &[_]) = match &def.0 {
			Expr::Claim { typ, fills, .. } => return find_def(fills, &format!("{prefix}{typ}."), path),
			Expr::StructDef { name, fills, .. }
			| Expr::EnumDef { name, fills, .. }
			| Expr::TraitDef {
				name, methods: fills, ..
			} => (name, fills),
			Expr::Bind { name, .. } | Expr::MacroDef { name, .. } => (name, &[]),
			e => (e.def_name()?, &[]),
		};
		let full = format!("{prefix}{name}").replace("::", ".");
		match full.trim_start_matches("core.") == path {
			true => Some(def.1),
			false => find_def(members, &format!("{full}."), path),
		}
	})
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
