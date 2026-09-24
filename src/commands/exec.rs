use oi::Reported;
use oi::driver::{DebugOpts, run_source};

/// Compile and run source from the argument, or from stdin when the arg is absent or `-`.
pub fn run(source: Option<String>, opts: DebugOpts) -> Result<(), Reported> {
	let (name, src) = match source.filter(|s| s != "-") {
		Some(arg) => ("<exec>", arg),
		None => (
			"<stdin>",
			std::io::read_to_string(std::io::stdin()).map_err(|e| {
				eprintln!("oi: cannot read stdin: {e}");
				Reported
			})?,
		),
	};
	run_source(vec![(name.to_string(), src)], std::path::Path::new("."), opts)
}
