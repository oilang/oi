pub mod exec;
pub mod init;
pub mod install;
pub mod repl;
pub mod run;

use oi::Reported;

use crate::cli::Command;

/// Route a parsed command to its handler.
pub fn dispatch(cmd: Command) -> Result<(), Reported> {
	match cmd {
		Command::Init => init::init(),
		Command::New { name } => init::new(&name),
		Command::Run { file, args, opts } => run::run(&run::entry(file), args, opts),
		Command::Build {
			file,
			out,
			lib,
			timings,
		} => run::build(&run::entry(file), out.as_deref(), lib, timings),
		Command::Exec { source, opts } => exec::run(source, opts),
		Command::Test { file, pattern } => run::test(&run::entry(file), pattern.as_deref()),
		Command::Repl => repl::run(),
		Command::Install { path, prefix, link } => install::install(path.as_deref(), prefix.as_deref(), link),
	}
}
