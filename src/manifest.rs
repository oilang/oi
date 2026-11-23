//! Package manifests, `oi.toml`.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::Reported;
use crate::driver::fail;
use crate::loader::home;

#[derive(Deserialize, Default)]
pub struct Manifest {
	pub name: Option<String>,
	#[serde(default)]
	pub root: PathBuf,
	#[serde(default)]
	pub deps: BTreeMap<String, Dep>,
	#[serde(default)]
	pub bins: BTreeMap<String, PathBuf>,
}

#[derive(Deserialize)]
pub struct Dep {
	pub path: Option<PathBuf>,
	pub git: Option<String>,
	pub rev: Option<String>,
}

impl Dep {
	// Find the dep's dir from `dir`, fetching a git dep.
	pub fn source(&self, key: &str, dir: &Path) -> Result<PathBuf, Reported> {
		match (&self.git, &self.path) {
			(Some(url), _) => {
				if self.rev.is_none() {
					eprintln!("oi: warning: dep `{key}` has no `rev`, `oi fetch` pins it");
				}
				fetch(url, self.rev.as_deref(), &home())
			}
			(None, Some(path)) => {
				let at = |e| fail(format!("dep `{key}`: {}: {e}", path.display()));
				dir.join(path).canonicalize().map_err(at)
			}
			(None, None) => Err(fail(format!("dep `{key}` needs `path` or `git`"))),
		}
	}
}

// Clone a git URL to `<prefix>/src/<host>/<path>@<rev>`, or pull its tip if unpinned.
pub fn fetch(url: &str, rev: Option<&str>, prefix: &Path) -> Result<PathBuf, Reported> {
	let tail = url.split_once("://").map_or(url, |(_, tail)| tail);
	let tail = tail.trim_start_matches("git@").replacen(':', "/", 1);
	let pin = rev.map_or_else(String::new, |rev| format!("@{rev}"));
	let tail = format!("{}{pin}", tail.trim_end_matches(".git"));
	let normal = |part: &Component| matches!(part, Component::Normal(_));
	let mut dir = prefix.join("src");
	dir.extend(Path::new(&tail).components().filter(normal));

	let path = &*dir.to_string_lossy();
	let git = |args: &[&str]| Command::new("git").args(args).status().is_ok_and(|status| status.success());
	let fresh = !dir.exists();
	let ok = match rev {
		_ if !fresh => rev.is_some() || git(&["-C", path, "pull", "-q", "--ff-only"]),
		Some(rev) => git(&["clone", "-q", url, path]) && git(&["-C", path, "checkout", "-q", rev]),
		None => git(&["clone", "-q", "--depth", "1", url, path]) || git(&["clone", "-q", url, path]),
	};
	if !ok {
		if fresh {
			let _ = std::fs::remove_dir_all(&dir);
		}
		return Err(fail(format!("{url}: git failed")));
	}
	Ok(dir)
}

impl Manifest {
	// Read `dir/oi.toml`, if exists.
	pub fn read(dir: &Path) -> Result<Option<Manifest>, Reported> {
		let path = dir.join("oi.toml");
		let parsed = std::fs::read_to_string(&path).ok().map(|text| toml::from_str(&text));
		parsed.transpose().map_err(|e| fail(format!("{}: {e}", path.display())))
	}

	// Find the nearest manifest at or above `dir` with its dir, stopping at a repo root.
	// TODO: do this in a way that doesn't tightly couple to git or the fs
	pub fn find(dir: &Path) -> Result<Option<(PathBuf, Manifest)>, Reported> {
		let full = dir.canonicalize().unwrap_or_default();
		let marked = |dir: &&Path| dir.join("oi.toml").exists() || dir.join(".git").exists();
		let dir = full.ancestors().find(marked).unwrap_or(&full);
		Ok(Self::read(dir)?.map(|manifest| (dir.into(), manifest)))
	}

	// Name the package, defaulting to its dir leaf.
	pub fn name(&self, dir: &Path) -> String {
		let leaf = || dir.file_name().unwrap_or_default().to_string_lossy().into();
		self.name.clone().unwrap_or_else(leaf)
	}
}
