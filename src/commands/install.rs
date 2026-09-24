use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use oi::Reported;
use oi::driver::{DebugOpts, build_source};
use oi::loader::home;

use crate::commands::run;

/// Install a binary, module, or shared library into OI_HOME.
pub fn install(path: Option<&Path>, prefix: Option<&Path>, link: bool) -> Result<(), Reported> {
	let url = path
		.and_then(Path::to_str)
		.filter(|s| s.contains("://") || s.starts_with("git@"));
	let cloned = url.map(|u| fetch(u, prefix)).transpose()?;
	let path = cloned.as_deref().or(path).unwrap_or(Path::new("."));
	let src = path.canonicalize().map_err(at(path))?;
	let entry = [src.join("src/main.oi"), src.join("main.oi")].into_iter().find(|p| p.exists());
	let kind = if entry.is_some() { "bin" } else { "lib" };
	let dest = prefix
		.map_or_else(home, Path::to_path_buf)
		.join(kind)
		.join(src.file_name().unwrap_or_default());
	fs::create_dir_all(dest.parent().unwrap()).map_err(at(&dest))?;
	if dest.is_symlink() {
		fs::remove_file(&dest).map_err(at(&dest))?;
	} else if link && dest.is_dir() {
		fs::remove_dir_all(&dest).map_err(at(&dest))?;
	}
	let opts = DebugOpts::default();
	match entry {
		Some(e) => build_source(run::files(&e)?, run::root(&e), &run::stem(&e), &dest, false, opts)?,
		None if src.is_file() => fs::copy(&src, &dest).map(drop).map_err(at(&src))?,
		None if link => std::os::unix::fs::symlink(&src, &dest).map_err(at(&dest))?,
		None => copy(&src, &dest).map_err(at(&src))?,
	}
	println!("oi: installed {}", dest.display());
	Ok(())
}

// Shallow-clone (or fast-forward) a git URL into OI_HOME/src/<name>, returning that checkout.
fn fetch(url: &str, prefix: Option<&Path>) -> Result<PathBuf, Reported> {
	let name = url.trim_end_matches('/').rsplit('/').next().unwrap_or(url);
	let dir = prefix
		.map_or_else(home, Path::to_path_buf)
		.join("src")
		.join(name.trim_end_matches(".git"));
	let mut git = Command::new("git");
	if dir.exists() {
		git.arg("-C").arg(&dir).args(["pull", "-q", "--ff-only"]);
	} else {
		git.args(["clone", "-q", "--depth", "1", url]).arg(&dir);
	}
	if !git.status().map_err(at(&dir))?.success() {
		eprintln!("oi: {url}: git failed");
		return Err(Reported);
	}
	Ok(dir)
}

// Recursively copy Oi files and shared libs.
fn copy(src: &Path, dst: &Path) -> io::Result<()> {
	fs::create_dir_all(dst)?;
	for entry in fs::read_dir(src)?.flatten() {
		let (from, to) = (entry.path(), dst.join(entry.file_name()));
		let lib = from.to_str().is_some_and(|s| s.ends_with(std::env::consts::DLL_SUFFIX));
		if from.is_dir() {
			copy(&from, &to)?;
		} else if lib || from.extension().is_some_and(|e| e == "oi") {
			fs::copy(&from, &to)?;
		}
	}
	Ok(())
}

fn at(path: &Path) -> impl Fn(io::Error) -> Reported {
	let path = path.display().to_string();
	move |e| {
		eprintln!("oi: {path}: {e}");
		Reported
	}
}
