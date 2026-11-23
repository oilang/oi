use std::process::{Command, Stdio};

use crate::common::{Project, Run, err, oi, ok};

#[test]
fn installs_module_into_home() {
	let home = Project::new();
	let p = Project::new()
		.file("pkg/oi.toml", [r#"name = "greet""#, r#"root = "src""#])
		.file("pkg/src/lib.oi", ["module greet", "pub hi :: fn() int { 42 }"])
		.file("other/oi.toml", [r#"name = "greet""#])
		.file("main.oi", ["module main", "use greet", "print(greet.hi())"]);
	let run = |args: &[&str]| oi(args).current_dir(&p).env("OI_HOME", home.as_ref()).run(None);
	ok(run(&["install", "pkg", "--link"]));
	assert!(home.as_ref().join("lib/greet").is_symlink());

	ok(run(&["install", "pkg"]));
	assert!(!home.as_ref().join("lib/greet").is_symlink());

	ok(run(&["install", "pkg"]));
	assert!(err(run(&["install", "other"])).contains("taken"));
	assert_eq!(ok(run(&["run", "main.oi"])), "42");
}

#[test]
fn installs_binary_into_home() {
	let home = Project::new();
	let p = Project::new()
		.file("oi.toml", ["[bins]", r#"hello = "src/main.oi""#])
		.file("src/main.oi", [r#"main :: fn() { print("hi") }"#]);
	ok(oi(&["install"]).current_dir(&p).env("OI_HOME", home.as_ref()).run(None));

	let bin = home.as_ref().join("bin/hello");
	assert_eq!(ok(Command::new(bin).stdout(Stdio::piped()).run(None)), "hi");
}

#[test]
fn installs_from_git_url() {
	let repo = Project::new().file("src/main.oi", [r#"main :: fn() { print("hi") }"#]);
	let git = |args: &[&str]| assert!(Command::new("git").args(args).current_dir(&repo).status().unwrap().success());
	git(&["init", "-q"]);
	git(&["add", "."]);
	git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);

	let home = Project::new();
	let url = format!("file://{}", repo.as_ref().display());

	let install = || ok(oi(&["install", &url]).env("OI_HOME", home.as_ref()).run(None));
	install();
	// ensure re-install doesn't choke on the existing clone
	install();

	let bin = home.as_ref().join("bin").join(repo.as_ref().file_name().unwrap());
	assert_eq!(ok(Command::new(bin).stdout(Stdio::piped()).run(None)), "hi");
}
