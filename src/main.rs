mod cli;
mod commands;

use std::process::ExitCode;

use clap::Parser as _;

use crate::cli::Cli;

fn main() -> ExitCode {
	let cli = Cli::parse();
	let _ = oi::diagnostics::COLOR.set(cli.color);
	let _ = oi::diagnostics::ERROR_FORMAT.set(cli.error_format);

	if cli.overlay {
		let overlay = serde_json::from_reader(std::io::stdin()).map_err(|e| oi::driver::fail(format!("overlay: {e}")));
		let Ok(overlay) = overlay else { return ExitCode::FAILURE };
		let _ = oi::loader::OVERLAY.set(overlay);
	}

	match commands::dispatch(cli.command.unwrap_or(cli::Command::Repl)) {
		Ok(()) => ExitCode::SUCCESS,
		Err(_) => ExitCode::FAILURE,
	}
}
