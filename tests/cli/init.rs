use crate::common::{Project, Run, oi, ok};

#[test]
fn scaffolds_a_runnable_project() {
	let dir = Project::new();
	dir.ok(&["new", "hello"]);

	let hello = dir.as_ref().join("hello");
	assert!(hello.join(".gitignore").exists());
	assert!(hello.join(".git").exists());

	let manifest = std::fs::read_to_string(hello.join("oi.toml")).unwrap();
	assert_eq!(manifest, "name = \"hello\"\nroot = \"src\"\n");
	assert_eq!(ok(oi(&["run"]).current_dir(&hello).run(None)), "Hi Mom!");
}
