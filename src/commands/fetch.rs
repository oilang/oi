use std::fs;
use std::path::Path;
use std::process::Command;

use oi::Reported;
use oi::driver::fail;
use oi::manifest::Manifest;
use toml_edit::{DocumentMut, value};

/// Fetch the manifest's git deps, pinning unpinned ones to their remote HEAD.
pub fn fetch() -> Result<(), Reported> {
	let Some((dir, manifest)) = Manifest::find(Path::new("."))? else {
		return Err(fail("no oi.toml found"));
	};

	let path = dir.join("oi.toml");
	let at = |e: &dyn std::fmt::Display| fail(format!("{}: {e}", path.display()));
	let mut doc: DocumentMut = fs::read_to_string(&path).map_err(|e| at(&e))?.parse().map_err(|e| at(&e))?;

	for (key, mut dep) in manifest.deps {
		let Some(url) = &dep.git else { continue };

		if dep.rev.is_none() {
			let git = Command::new("git").args(["ls-remote", url, "HEAD"]).output();
			let out = git.map_err(|e| fail(format!("git: {e}")))?;
			let sha = String::from_utf8_lossy(&out.stdout).split_whitespace().next().map(String::from);
			let sha = sha.ok_or_else(|| fail(format!("{url}: no HEAD to pin")))?;

			let entry = &mut doc["deps"][&key];
			entry["rev"] = value(&sha);

			if let Some(table) = entry.as_inline_table_mut() {
				table.fmt();
			}

			println!("oi: pinned {key} at {sha}");
			dep.rev = Some(sha);
		}

		dep.source(&key, &dir)?;
	}

	fs::write(&path, doc.to_string()).map_err(|e| at(&e))
}
