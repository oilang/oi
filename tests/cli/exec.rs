use crate::common::{Run, oi, ok, stderr};

#[test]
fn exec_source_from_arg_or_stdin() {
	let cases: [(&[&str], Option<&str>, &str); 6] = [
		(&["exec", "2 + 3 * 4"], None, "14"),
		(&["exec", r#""a" + "b""#], None, "ab"),
		(&["exec", "-5 + 8"], None, "3"),
		(&["exec"], Some("1 + 2"), "3"),
		(&["exec", "2 + 2"], Some("not valid oi"), "4"),
		(&["exec", "-"], Some("1 + 2"), "3"),
	];
	for (args, stdin, expected) in cases {
		assert_eq!(ok(oi(args).run(stdin)), expected, "{args:?}");
	}
}

#[test]
fn error_names_exec_source() {
	let out = oi(&["exec", "2 +"]).run(None);
	assert!(!out.status.success());
	let stderr = stderr(&out);
	assert!(stderr.contains("<exec>"), "stderr was:\n{stderr}");
}
