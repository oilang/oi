use std::io::{IsTerminal as _, Read as _};

use oi::Reported;
use oi::driver::{DebugOpts, run_source};

/// Compile and run source from the argument, piped stdin, or both concatenated.
/// With no arg, stdin is the program.
/// With an arg, stdin is an optional preamble.
pub fn run(source: Option<String>, opts: DebugOpts) -> Result<(), Reported> {
	let stdin = std::io::stdin();
	let mut src = String::new();
	if source.is_none() || (!stdin.is_terminal() && stdin_has_data()) {
		stdin.lock().read_to_string(&mut src).map_err(|e| {
			eprintln!("oi: cannot read stdin: {e}");
			Reported
		})?;
	}
	let name = if source.is_some() { "<exec>" } else { "<stdin>" };
	if let Some(arg) = source {
		if !src.is_empty() && !src.ends_with('\n') {
			src.push('\n');
		}
		src.push_str(&arg);
	}
	run_source(vec![(name.to_string(), src)], std::path::Path::new("."), opts)
}

/// Whether stdin has bytes waiting.
#[cfg(unix)]
fn stdin_has_data() -> bool {
	use std::os::fd::AsRawFd as _;
	let mut fd = libc::pollfd {
		fd: std::io::stdin().as_raw_fd(),
		events: libc::POLLIN,
		revents: 0,
	};
	unsafe { libc::poll(&mut fd, 1, 50) > 0 && fd.revents & libc::POLLIN != 0 }
}

#[cfg(not(unix))]
fn stdin_has_data() -> bool {
	true
}
