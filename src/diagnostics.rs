use std::cell::RefCell;
use std::fmt::Display;
use std::io::IsTerminal;
use std::ops::Range;
use std::sync::OnceLock;

use ariadne::{Color, Config, IndexType, Label, Report, ReportKind, Source};
use chumsky::error::{Rich, RichReason};
use serde_json::json;

use crate::Reported;
use crate::ast::Span;
use crate::lexer::Token;

/// `--color`, set once by the CLI.
pub static COLOR: OnceLock<ColorMode> = OnceLock::new();

#[derive(clap::ValueEnum, Clone, Copy, Default)]
pub enum ColorMode {
	#[default]
	Auto,
	Always,
	Never,
}

/// `--error-format`, set once by the CLI.
pub static ERROR_FORMAT: OnceLock<ErrorFormat> = OnceLock::new();

#[derive(clap::ValueEnum, Clone, Copy, Default, PartialEq)]
pub enum ErrorFormat {
	#[default]
	Human,
	Json,
}

// Get the 1-based line and column a source prefix ends at.
fn line_col(before: &str) -> (usize, usize) {
	let line = before.rsplit('\n').next().unwrap_or_default();
	(before.matches('\n').count() + 1, line.chars().count() + 1)
}

// A source file at its program-wide byte offset.
#[derive(Clone)]
struct File {
	base: usize,
	name: String,
	src: String,
}

thread_local! {
	// Sources parse reads at comptime.
	static FRAGMENTS: RefCell<Vec<&'static File>> = const { RefCell::new(Vec::new()) };
}

// Lay out a comptime-parsed source past any loaded file, returning its base.
pub fn push_fragment(src: String) -> usize {
	FRAGMENTS.with_borrow_mut(|frags| {
		let base = frags.last().map_or(1 << 48, |f| f.base + f.src.len() + 1);
		let name = "<parse>".into();
		frags.push(Box::leak(Box::new(File { base, name, src })));
		base
	})
}

// Source files laid out at increasing byte offsets so one span space covers the whole program.
#[derive(Clone, Default)]
pub struct SourceMap {
	files: Vec<File>,
}

impl SourceMap {
	// Append a file, returning its base offset.
	pub fn push(&mut self, name: String, src: String) -> usize {
		let base = self.files.last().map_or(0, |f| f.base + f.src.len() + 1);
		self.files.push(File { base, name, src });
		base
	}

	// Print the loaded file names as a JSON line, under `--error-format json`.
	pub fn report_files(&self) {
		if ERROR_FORMAT.get() == Some(&ErrorFormat::Json) {
			let names: Vec<_> = self.files.iter().map(|file| &file.name).collect();
			eprintln!("{}", json!({ "files": names }));
		}
	}

	pub fn src(&self, base: usize) -> &str {
		&self.files[self.files.partition_point(|f| f.base < base)].src
	}

	// Map a program-wide span back to its file and local range.
	fn locate(&self, span: &Range<usize>) -> (&File, Range<usize>) {
		let frag = FRAGMENTS.with_borrow(|frags| frags.iter().rev().find(|f| f.base <= span.start).copied());
		let i = self.files.partition_point(|f| f.base <= span.start).saturating_sub(1);
		let f = frag.unwrap_or(&self.files[i]);
		(f, span.start - f.base..span.end - f.base)
	}

	// Get actual source/metadata for a program-wide span.
	pub fn locate_span(&self, span: Range<usize>) -> (&str, usize, usize, &str) {
		let (file, local) = self.locate(&span);
		let (line, col) = line_col(&file.src[..local.start]);
		(&file.name, line, col, &file.src[local])
	}

	// The source of `at`'s file up to `at`.
	pub fn before(&self, at: usize) -> &str {
		let (file, local) = self.locate(&(at..at));
		&file.src[..local.start]
	}

	// The `##` block before a given line.
	pub fn docs(&self, at: usize) -> Vec<&str> {
		// block docs
		let before = self.before(at).trim_end();
		if let Some(block) = before.strip_suffix("}##")
			&& let Some((_, body)) = block.rsplit_once("##{")
		{
			return body.trim().lines().map(str::trim).collect();
		}

		// normal docs
		let (above, lead) = self.before(at).rsplit_once('\n').unwrap_or_default();
		let above = above.lines().rev().map(str::trim_start).skip_while(|l| l.starts_with('@'));
		let docs = above.take_while(|l| l.starts_with("##") && !lead.contains([':', ',', '{', ';']));
		let mut docs: Vec<_> = docs.map(|l| l.get(3..).unwrap_or("")).collect();
		docs.reverse();
		docs
	}
}

// A user-facing error rendered with ariadne.
pub struct Diagnostic {
	message: String,
	span: Range<usize>,
	label: Option<String>,
	note: Option<String>,
	also: Option<Box<(Range<usize>, String)>>,
}

impl Diagnostic {
	pub fn new(message: impl Into<String>, span: Range<usize>) -> Self {
		Self {
			message: message.into(),
			span,
			label: None,
			note: None,
			also: None,
		}
	}

	pub fn message(&self) -> &str {
		&self.message
	}

	pub fn with_label(mut self, label: impl Into<String>) -> Self {
		self.label = Some(label.into());
		self
	}

	pub fn with_note(mut self, note: impl Into<String>) -> Self {
		self.note = Some(note.into());
		self
	}

	pub fn with_also(mut self, span: Range<usize>, label: impl Into<String>) -> Self {
		self.also = Some(Box::new((span, label.into())));
		self
	}

	// Build a diagnostic from a chumsky parse error.
	pub fn from_rich(err: &Rich<'_, Token>) -> Self {
		let label = match err.reason() {
			RichReason::Custom(_) => "here",
			RichReason::ExpectedFound { found: None, .. } => "unexpected end of input",
			RichReason::ExpectedFound { .. } => "unexpected token",
		};
		Self::new(err.reason().to_string(), err.span().into_range()).with_label(label)
	}

	// Render to stderr, rebasing the span into its owning file.
	pub fn report_mapped(&self, map: &SourceMap) -> Reported {
		let (file, span) = map.locate(&self.span);
		// one diagnostic per line, its span in bytes and 1-based lines and columns of `text`
		if ERROR_FORMAT.get() == Some(&ErrorFormat::Json) {
			let src = &file.src;
			let (start, end) = (span.start.min(src.len()), span.end.min(src.len()));
			let first = src[..start].rfind('\n').map_or(0, |i| i + 1);
			let last = src[end..].find('\n').map_or(src.len(), |i| end + i);
			let point = |offset: usize| {
				let (line, col) = line_col(&src[..offset]);
				json!({ "offset": offset, "line": line, "col": col })
			};

			let diag = json!({
				"file": file.name,
				"span": { "start": point(start), "end": point(end) },
				"text": src[first..last],
				"message": self.message,
				"label": self.label,
				"note": self.note,
			});
			eprintln!("{diag}");
			return Reported;
		}

		let id = file.name.clone();
		let color = match COLOR.get().copied().unwrap_or_default() {
			ColorMode::Always => true,
			ColorMode::Never => false,
			ColorMode::Auto => {
				std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
			}
		};
		let config = Config::default().with_color(color).with_index_type(IndexType::Byte);

		let mut builder = Report::build(ReportKind::Error, (id.clone(), span.clone()))
			.with_config(config)
			.with_message(&self.message)
			.with_label(
				Label::new((id.clone(), span))
					.with_message(self.label.as_deref().unwrap_or("here"))
					.with_color(Color::Red),
			);
		if let Some((at, label)) = self.also.as_deref() {
			let (_, local) = map.locate(at);
			builder = builder.with_label(Label::new((id.clone(), local)).with_message(label).with_color(Color::Blue));
		}
		if let Some(note) = &self.note {
			builder = builder.with_note(note);
		}
		let _ = builder.finish().eprint((id, Source::from(&file.src)));
		Reported
	}
}

// An error with a labeled span.
pub fn fail<T>(msg: impl Into<String>, span: Span, label: impl Into<String>) -> Result<T, Diagnostic> {
	Err(Diagnostic::new(msg, span.into_range()).with_label(label))
}

pub fn arity_err<T>(what: &str, want: impl Display, got: usize, noun: &str, span: Span) -> Result<T, Diagnostic> {
	fail(
		format!("{what} expects {want} {noun}(s), got {got}"),
		span,
		format!("wrong number of {noun}s"),
	)
}

// Report a missing member.
pub fn unknown_member(owner: impl Display, kind: &str, name: impl Display, span: Span) -> Diagnostic {
	Diagnostic::new(format!("{owner} has no {kind} `{name}`"), span.into_range()).with_label(format!("no such {kind}"))
}
