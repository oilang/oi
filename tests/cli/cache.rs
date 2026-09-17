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
