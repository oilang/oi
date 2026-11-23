pub mod exec;
pub mod fetch;
pub mod init;
pub mod install;
pub mod lsp;
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
		Command::Build { file, out, lib, target, timings } => {
			run::build(&run::entry(file), out.as_deref(), lib, target.as_deref(), timings)
		}
		Command::Exec { source, opts } => exec::run(source, opts),
		Command::Test { file, patterns } => run::test(&run::entry(file), &patterns),
		Command::Repl => repl::run(),
		Command::Lsp => lsp::serve(),
		Command::Install { path, prefix, link } => install::install(path.as_deref(), prefix.as_deref(), link),
		Command::Fetch => fetch::fetch(),
	}
}
