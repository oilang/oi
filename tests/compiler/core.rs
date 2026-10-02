use crate::common::{Run, oi, oi_files, stderr, trim};

/// Self-hosted Oi tests.
/// Runs `oi test` over files in `tests/core/`.
#[test]
fn run_core_tests() {
	for path in oi_files("tests/core") {
		let name = path.file_name().unwrap().to_string_lossy().into_owned();
		let out = oi(&["test", path.to_str().unwrap()]).run(None);
		let report = trim(&out.stdout);
		assert!(out.status.success(), "{name}:\n{report}\n{}", stderr(&out));
		assert!(!report.starts_with("0 passed"), "{name}: no @test fns ran");
		println!("{name}\n{report}");
	}
}
