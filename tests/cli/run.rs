use indoc::indoc;

use crate::common::{Project, Run, oi, ok, stderr, trim};

#[test]
fn version_reports_pkg_version_sha_and_target() {
	let out = ok(oi(&["--version"]).run(None));
	assert!(out.starts_with("oi v0.1.0 ("), "version was:\n{out}");
}

#[test]
fn missing_file_errors() {
	let out = oi(&["run", "definitely-missing.oi"]).run(None);
	assert!(!out.status.success());

	let stderr = stderr(&out);
	assert!(stderr.contains("cannot read"), "stderr was:\n{stderr}");
}

#[test]
fn default_file_is_main_oi_in_cwd() {
	let dir = Project::new().file("main.oi", "1 + 2");
	assert_eq!(dir.ok(&["run"]), "3");
}

#[test]
fn failing_main_reports_where_the_error_came_from() {
	let src = indoc! {r#"
		main :: fn() ! {
			return error("boom")
		}
	"#};
	let dir = Project::new().file("main.oi", src);
	let out = dir.oi(&["run"]);
	assert_eq!(trim(&out.stderr), "error: boom  at main.oi:2");
}

#[test]
fn timings_prints_phases_to_stderr() {
	let dir = Project::new().file("main.oi", "1 + 2");
	let out = dir.oi(&["run", "--timings"]);
	assert!(out.status.success());

	let stderr = stderr(&out);
	assert!(stderr.contains("codegen"), "stderr was:\n{stderr}");
	assert!(stderr.contains("run"), "stderr was:\n{stderr}");
}

#[test]
fn directory_entry_runs_all_its_files_as_one_main() {
	let dir = Project::new()
		.file("src/a.oi", "f :: fn() int { 42 }")
		.file("src/b.oi", "print(f())");
	assert_eq!(dir.ok(&["run", "src"]), "42");
}

#[test]
fn emit_dumps_each_stage_to_stderr() {
	let dir = Project::new().file("main.oi", "1 + 2");
	for (stage, expected) in [("tokens", "Plus"), ("ast", "Binary"), ("clif", "function u0:0")] {
		let out = dir.oi(&["run", "--emit", stage]);
		assert!(out.status.success());
		let stderr = stderr(&out);
		assert!(stderr.contains(expected), "{stage} stderr was:\n{stderr}");
		assert_eq!(trim(&out.stdout), "3");
	}
}

#[test]
fn check_type_checks_without_running() {
	let dir = Project::new().file("main.oi", r#"print("should not run")"#);
	assert_eq!(dir.ok(&["run", "--check"]), "");

	let dir = Project::new().file("main.oi", "x + 1");
	let out = dir.oi(&["run", "--check"]);
	assert!(!out.status.success());

	let stderr = stderr(&out);
	assert!(stderr.contains("undefined variable"), "stderr was:\n{stderr}");
}

#[test]
fn trailing_args_reach_os_args() {
	let src = indoc! {"
		use os
		print(os.args()[1..])
	"};
	let dir = Project::new().file("main.oi", src);
	let out = dir.oi(&["run", "main.oi", "a", "-b"]);
	assert_eq!(ok(out), r#"["a", "-b"]"#);
}
