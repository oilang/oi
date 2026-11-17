mod cli;
mod commands;

use std::process::ExitCode;

use clap::Parser as _;

use crate::cli::Cli;

fn main() -> ExitCode {
	let cli = Cli::parse();
	let _ = oi::diagnostics::COLOR.set(cli.color);
	let _ = oi::diagnostics::ERROR_FORMAT.set(cli.error_format);
	match commands::dispatch(cli.command.unwrap_or(cli::Command::Repl)) {
		Ok(()) => ExitCode::SUCCESS,
		Err(_) => ExitCode::FAILURE,
	}
}
