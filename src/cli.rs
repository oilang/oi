use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// The Oi CLI.
#[derive(Parser)]
#[command(name = "oi", version, about)]
pub struct Cli {
	#[command(subcommand)]
	pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
	/// Scaffold a project in the current directory.
	Init,

	/// Scaffold a project in a new directory.
	New {
		/// Directory to create.
		name: String,
	},

	/// Compile and run an Oi file.
	Run {
		/// Path to the .oi source file.
		file: Option<PathBuf>,

		/// Print phase timings to stderr.
		#[arg(long)]
		timings: bool,
	},

	/// Compile an Oi file to a native executable.
	Build {
		/// Path to the source file.
		file: Option<PathBuf>,

		/// Output path.
		/// Defaults to the file stem in the current directory.
		#[arg(short, long)]
		out: Option<PathBuf>,

		/// Build a shared library instead of an executable.
		#[arg(long)]
		lib: bool,

		/// Print phase timings to stderr.
		#[arg(long)]
		timings: bool,
	},

	/// Compile and run an Oi script.
	Exec {
		/// Source to run, appended to piped stdin if any. If omitted, read from stdin.
		#[arg(allow_hyphen_values = true)]
		source: Option<String>,

		/// Print phase timings to stderr.
		#[arg(long)]
		timings: bool,
	},

	/// Compile and run a file's `@test` fns.
	Test {
		/// Path to the .oi source file.
		file: Option<PathBuf>,

		/// Only run tests whose name matches pattern.
		pattern: Option<String>,
	},

	/// Start an interactive Oi REPL.
	Repl,

	/// Install a binary or module into OI_HOME.
	Install {
		/// Path to install. Defaults to the current directory.
		path: Option<PathBuf>,

		/// Override OI_HOME.
		#[arg(long)]
		prefix: Option<PathBuf>,

		/// Symlink a module instead of copying it.
		#[arg(long)]
		link: bool,
	},
}
