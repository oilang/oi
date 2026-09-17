use std::path::Path;

use crate::common::{Project, ok};

#[test]
fn warm_run_reuses_cache() {
	let dir = Project::new().file("main.oi", "1 + 2");
	assert_eq!(ok(dir.run()), "3");
	let root: &Path = dir.as_ref();
	assert!(std::fs::read_dir(root.join(".oi/cache")).unwrap().count() > 0);
	assert_eq!(ok(dir.run()), "3");
}

#[test]
fn edited_comp_input_is_not_stale() {
	let src = indoc::indoc! {r#"
		use fs
		name := comp fs.read("build.toml") or { "none" }
		print(name)
	"#};
	let dir = Project::new().file("main.oi", src).file("build.toml", "one");
	assert_eq!(ok(dir.run()), "one");
	let root: &Path = dir.as_ref();
	assert!(root.join(".oi/cache").is_dir());
	std::fs::write(root.join("build.toml"), "two").unwrap();
	assert_eq!(ok(dir.run()), "two");
}
