use crate::common::{Project, Run, oi};

#[test]
fn always_forces_ansi_codes() {
	let dir = Project::new().file("main.oi", "foo");
	// NOTE: piped stderr is colorless, so this tests `always`
	let out = oi(&["--color", "always", "run"]).current_dir(&dir).run(None);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(stderr.contains("\x1b["), "stderr was:\n{stderr}");
}
