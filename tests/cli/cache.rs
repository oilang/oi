use std::path::Path;
use std::time::SystemTime;

use crate::common::{Project, Run, oi, ok};

#[test]
fn warm_run_reuses_cache() {
	let dir = Project::new().file("main.oi", "1 + 2");
	assert_eq!(ok(dir.run()), "3");
	let root: &Path = dir.as_ref();
	let entries = std::fs::read_dir(root.join(".oi/cache")).unwrap().count();
	assert!(entries > 0);
	assert_eq!(ok(dir.run()), "3");
	assert_eq!(std::fs::read_dir(root.join(".oi/cache")).unwrap().count(), entries);
}

#[test]
fn edited_comp_input_is_not_stale() {
	let src = indoc::indoc! {r#"
		use fs
		use os
		name := comp fs.read("build.toml") or { "none" }
		flag := comp os.sh("test -f flag")
		print(name, flag)
	"#};
	let dir = Project::new().file("main.oi", src).file("build.toml", "one");
	assert_eq!(ok(dir.run()), "one 1");
	let root: &Path = dir.as_ref();
	assert!(root.join(".oi/cache").is_dir());
	std::fs::write(root.join("build.toml"), "two").unwrap();
	std::fs::write(root.join("flag"), "").unwrap();
	assert_eq!(ok(dir.run()), "two 0");
}

#[test]
fn disabled_cache_writes_nothing() {
	let dir = Project::new().file("main.oi", "1 + 2");
	let root: &Path = dir.as_ref();
	let out = oi(&["run", "main.oi"]).current_dir(root).env("OI_CACHE", "0").run(None);
	assert_eq!(ok(out), "3");
	assert!(!root.join(".oi").exists());
}

#[test]
fn run_sweeps_stale_entries() {
	let dir = Project::new().file("main.oi", "1 + 2");
	let root: &Path = dir.as_ref();
	let stale = root.join(".oi/cache/stale");
	std::fs::create_dir_all(root.join(".oi/cache")).unwrap();
	std::fs::File::create(&stale)
		.unwrap()
		.set_modified(SystemTime::UNIX_EPOCH)
		.unwrap();
	assert_eq!(ok(dir.run()), "3");
	assert!(!stale.exists());
}
