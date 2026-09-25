use indoc::indoc;

use crate::common::{Project, Run, oi, ok, trim};

#[test]
fn version_reports_pkg_version_sha_and_target() {
	let out = ok(oi(&["--version"]).run(None));
	assert!(out.starts_with("oi v0.1.0 ("), "version was:\n{out}");
}

#[test]
fn missing_file_errors() {
	let out = oi(&["run", "definitely-missing.oi"]).run(None);
	assert!(!out.status.success());

	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(stderr.contains("cannot read"), "stderr was:\n{stderr}");
}

#[test]
fn default_file_is_main_oi_in_cwd() {
	let dir = Project::new().file("main.oi", "1 + 2");
	assert_eq!(ok(oi(&["run"]).current_dir(&dir).run(None)), "3");
}

#[test]
fn timings_prints_phases_to_stderr() {
	let dir = Project::new().file("main.oi", "1 + 2");
	let out = oi(&["run", "--timings"]).current_dir(&dir).run(None);
	assert!(out.status.success());

	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(stderr.contains("codegen"), "stderr was:\n{stderr}");
	assert!(stderr.contains("run"), "stderr was:\n{stderr}");
}

#[test]
fn directory_entry_runs_all_its_files_as_one_main() {
	let dir = Project::new()
		.file("src/a.oi", "f :: fn() int { 42 }")
		.file("src/b.oi", "print(f())");
	assert_eq!(ok(oi(&["run", "src"]).current_dir(&dir).run(None)), "42");
}

#[test]
fn emit_tokens_dumps_the_lexer_output() {
	let dir = Project::new().file("main.oi", "1 + 2");
	let out = oi(&["run", "--emit", "tokens"]).current_dir(&dir).run(None);
	assert!(out.status.success());

	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(stderr.contains("Plus"), "stderr was:\n{stderr}");
	assert_eq!(trim(&out.stdout), "3");
}

#[test]
fn emit_ast_dumps_the_parsed_tree() {
	let dir = Project::new().file("main.oi", "1 + 2");
	let out = oi(&["run", "--emit", "ast"]).current_dir(&dir).run(None);
	assert!(out.status.success());

	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(stderr.contains("Binary"), "stderr was:\n{stderr}");
}

#[test]
fn emit_clif_dumps_cranelift_ir() {
	let dir = Project::new().file("main.oi", "1 + 2");
	let out = oi(&["run", "--emit", "clif"]).current_dir(&dir).run(None);
	assert!(out.status.success());

	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(stderr.contains("function u0:0"), "stderr was:\n{stderr}");
}

#[test]
fn check_type_checks_without_running() {
	let dir = Project::new().file("main.oi", r#"print("should not run")"#);
	assert_eq!(ok(oi(&["run", "--check"]).current_dir(&dir).run(None)), "");

	let dir = Project::new().file("main.oi", "x + 1");
	let out = oi(&["run", "--check"]).current_dir(&dir).run(None);
	assert!(!out.status.success());

	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(stderr.contains("undefined variable"), "stderr was:\n{stderr}");
}

#[test]
fn trailing_args_reach_os_args() {
	let src = indoc! {"
		use os
		print(os.args()[1..])
	"};
	let dir = Project::new().file("main.oi", src);
	let out = oi(&["run", "main.oi", "a", "-b"]).current_dir(&dir).run(None);
	assert_eq!(ok(out), r#"["a", "-b"]"#);
}
