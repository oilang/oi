use std::io::IsTerminal;
use std::ops::Range;
use std::sync::OnceLock;

use ariadne::{Color, Config, IndexType, Label, Report, ReportKind, Source};
use chumsky::error::{Rich, RichReason};

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

// A source file at its program-wide byte offset.
#[derive(Clone)]
struct File {
	base: usize,
	name: String,
	src: String,
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

	pub fn src(&self, base: usize) -> &str {
		&self.files[self.files.partition_point(|f| f.base < base)].src
	}

	// Map a program-wide span back to its file and local range.
	fn locate(&self, span: &Range<usize>) -> (&File, Range<usize>) {
		let i = self.files.partition_point(|f| f.base <= span.start).saturating_sub(1);
		let f = &self.files[i];
		(f, span.start - f.base..span.end - f.base)
	}

	// Get actual source/metadata for a program-wide span.
	pub fn locate_span(&self, span: Range<usize>) -> (&str, usize, &str) {
		let (file, local) = self.locate(&span);
		let line = file.src[..local.start].matches('\n').count() + 1;
		(&file.name, line, &file.src[local])
	}
}

// A user-facing error rendered with ariadne.
pub struct Diagnostic {
	message: String,
	span: Range<usize>,
	label: Option<String>,
	note: Option<String>,
}

impl Diagnostic {
	pub fn new(message: impl Into<String>, span: Range<usize>) -> Self {
		Self {
			message: message.into(),
			span,
			label: None,
			note: None,
		}
	}

	pub fn with_label(mut self, label: impl Into<String>) -> Self {
		self.label = Some(label.into());
		self
	}

	pub fn with_note(mut self, note: impl Into<String>) -> Self {
		self.note = Some(note.into());
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

	// Render through a source map, rebasing the span into its owning file.
	pub fn report_mapped(&self, map: &SourceMap) {
		let (file, span) = map.locate(&self.span);
		Diagnostic {
			message: self.message.clone(),
			span,
			label: self.label.clone(),
			note: self.note.clone(),
		}
		.report(&file.name, &file.src);
	}

	// Render span to stderr.
	pub fn report(&self, filename: &str, src: &str) {
		let id = filename.to_string();
		let color = match COLOR.get().copied().unwrap_or_default() {
			ColorMode::Always => true,
			ColorMode::Never => false,
			ColorMode::Auto => {
				std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
			}
		};
		let config = Config::default().with_color(color).with_index_type(IndexType::Byte);

		let mut builder = Report::build(ReportKind::Error, (id.clone(), self.span.clone()))
			.with_config(config)
			.with_message(&self.message)
			.with_label(
				Label::new((id.clone(), self.span.clone()))
					.with_message(self.label.as_deref().unwrap_or("here"))
					.with_color(Color::Red),
			);
		if let Some(note) = &self.note {
			builder = builder.with_note(note);
		}
		let _ = builder.finish().eprint((id, Source::from(src)));
	}
}
