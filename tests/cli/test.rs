use indoc::indoc;

use crate::common::{Project, stderr};

fn project(src: &str) -> Project {
	Project::new().file("main.oi", src)
}

#[test]
fn run_strips_tests_before_typecheck() {
	let dir = project(indoc! {r#"
		@test bad :: fn() { 1 < "x" }
		main :: fn() { print("fine") }
	"#});
	assert_eq!(dir.ok(&["run"]), "fine");
}

#[test]
fn test_runs_all_in_order() {
	let dir = project(indoc! {r#"
		@test first :: fn() { assert!(1 + 1 == 2) }
		@test second :: fn() { assert!(2 + 2 == 4) }
	"#});
	let out = dir.ok(&["test"]);
	assert!(out.find("first").unwrap() < out.find("second").unwrap() && out.contains("2 passed"));
}

#[test]
fn test_macro_runs_under_test_and_drops_under_run() {
	let dir = project(indoc! {r#"
		test! "leading literal" { assert! true }
		main :: fn() { print("fine") }
	"#});
	let out = dir.ok(&["test"]);
	assert!(out.contains("leading literal ... ok") && out.contains("1 passed"));
	assert_eq!(dir.ok(&["run"]), "fine");
}

#[test]
fn test_payload_renames_and_skips() {
	let dir = project(indoc! {r#"
		@test.{"alt name"} first :: fn() { assert! true }
		@test.{skip = true} second :: fn() { assert! false }
	"#});
	let out = dir.ok(&["test"]);
	assert!(out.contains("alt name ... ok") && out.contains("second ... skipped"));
	assert!(out.contains("1 passed; 1 skipped"));
}

#[test]
fn each_test_starts_from_the_declared_statics() {
	let dir = project(indoc! {r#"
		main :: fn() {}
		n := 0
		@test first :: fn() { n = n + 1; assert!(n == 1) }
		@test second :: fn() { n = n + 1; assert!(n == 1) }
	"#});
	assert!(dir.ok(&["test"]).contains("2 passed"));
}

#[test]
fn module_tests_are_qualified_and_filterable() {
	let dir = Project::new()
		.file("main.oi", ["use greeter", "@test main_test :: fn() { assert! true }"])
		.file(
			"greeter.oi",
			["module greeter", "@test warmup :: fn() { assert! true }"],
		);
	let out = dir.ok(&["test"]);
	assert!(
		out.contains("greeter.warmup ... ok") && out.contains("2 passed"),
		"{out}"
	);
	let out = dir.ok(&["test", "main.oi", "warmup"]);
	assert!(
		!out.contains("main_test") && out.contains("1 passed; 1 filtered out"),
		"{out}"
	);
}

#[test]
fn failing_test_is_isolated() {
	let dir = project(indoc! {r#"
		@test first :: fn() { assert! false }
		@test second :: fn() { assert! true }
	"#});
	let out = dir.oi(&["test"]);
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(!out.status.success());
	assert!(stdout.contains("first ... FAILED") && stdout.contains("second ... ok"));
	assert!(stdout.contains("1 passed; 1 failed"));
	assert!(stderr(&out).contains("assertion failed"));
}
