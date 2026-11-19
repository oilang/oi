//! Package manifests, `oi.toml`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::Reported;
use crate::driver::fail;

#[derive(Deserialize)]
pub struct Manifest {
	pub name: Option<String>,
	#[serde(default)]
	pub root: PathBuf,
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
