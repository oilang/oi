use crate::common::{Run, oi, oi_files, ok};

/// Every `examples/*.oi` carries its own `# expect: <line>` comments.
/// Runs each and collect every mismatch instead of failing on the first.
#[test]
fn run_examples() {
	let mut failures = Vec::new();

	for path in oi_files("examples") {
		let src = std::fs::read_to_string(&path).unwrap();
		let expected = (src.lines())
			.filter_map(|l| l.trim().strip_prefix("# expect: "))
			.collect::<Vec<_>>()
			.join("\n");

		let out = ok(oi(&["run", path.to_str().unwrap()]).run(None));
		if out != expected {
			let name = path.file_name().unwrap().to_string_lossy();
			failures.push(format!("{name}: expected {expected:?}, got {out:?}"));
		}
	}

	assert!(failures.is_empty(), "{}", failures.join("\n"));
}
