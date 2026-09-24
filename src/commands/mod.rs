pub mod exec;
pub mod init;
pub mod install;
pub mod repl;
pub mod run;

use oi::Reported;
use oi::driver::DebugOpts;

use crate::cli::Command;

/// Route a parsed command to its handler.
pub fn dispatch(cmd: Command) -> Result<(), Reported> {
	match cmd {
		Command::Init => init::init(),
		Command::New { name } => init::new(&name),
		Command::Run {
			file,
			timings,
			emit,
			check,
		} => run::run(&run::entry(file), DebugOpts { timings, emit, check }),
		Command::Build {
			file,
			out,
			lib,
			timings,
		} => run::build(&run::entry(file), out.as_deref(), lib, timings),
		Command::Exec {
			source,
			timings,
			emit,
			check,
		} => exec::run(source, DebugOpts { timings, emit, check }),
		Command::Test { file, pattern } => run::test(&run::entry(file), pattern.as_deref()),
		Command::Repl => repl::run(),
		Command::Install { path, prefix, link } => install::install(path.as_deref(), prefix.as_deref(), link),
	}
}
