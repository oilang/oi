use crate::common::{Project, stderr};

#[test]
fn always_forces_ansi_codes() {
	let dir = Project::new().file("main.oi", "foo");
	// NOTE: piped stderr is colorless, so this tests `always`
	let out = dir.oi(&["--color", "always", "run"]);
	let stderr = stderr(&out);
	assert!(stderr.contains("\x1b["), "stderr was:\n{stderr}");
}
