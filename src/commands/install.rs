use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use oi::Reported;
use oi::driver::fail;
use oi::loader::home;
use oi::manifest::Manifest;

use crate::commands::run;

/// Install a binary, module, or shared library into OI_HOME.
pub fn install(path: Option<&Path>, prefix: Option<&Path>, link: bool) -> Result<(), Reported> {
	let url = path
		.and_then(Path::to_str)
		.filter(|s| s.contains("://") || s.starts_with("git@"));
	let cloned = url.map(|u| fetch(u, prefix)).transpose()?;
	let path = cloned.as_deref().or(path).unwrap_or(Path::new("."));
	let src = path.canonicalize().map_err(at(path))?;
	let manifest = Manifest::read(&src)?.unwrap_or_default();
	let (name, home) = (manifest.name(&src), prefix.map_or_else(home, Path::to_path_buf));
	let main = ["src/main.oi", "main.oi"].into_iter().find(|main| src.join(main).exists());

	let mut bins = manifest.bins;
	if bins.is_empty() {
		bins.extend(main.map(|main| (name.clone(), main.into())));
	}

	let dir = home.join(if bins.is_empty() { "lib" } else { "bin" });
	fs::create_dir_all(&dir).map_err(at(&dir))?;
	for (bin, main) in &bins {
		run::build(&src.join(main), Some(&dir.join(bin)), false, None, false)?;
		println!("oi: installed {}", dir.join(bin).display());
	}
	if !bins.is_empty() {
		return Ok(());
	}

	let (lib, dest) = (src.join(manifest.root), dir.join(name));
	let marker = dest.join(".source");
	let source = fs::read_link(&dest).or_else(|_| fs::read_to_string(&marker).map(PathBuf::from));
	if dest.is_dir() && !source.is_ok_and(|source| source == lib) {
		return Err(fail(format!("{} is taken by another package", dest.display())));
	}

	if dest.is_symlink() {
		fs::remove_file(&dest).map_err(at(&dest))?;
	} else if link && dest.is_dir() {
		fs::remove_dir_all(&dest).map_err(at(&dest))?;
	}

	if src.is_file() {
		fs::copy(&src, &dest).map_err(at(&src))?;
	} else if link {
		std::os::unix::fs::symlink(&lib, &dest).map_err(at(&dest))?;
	} else {
		copy(&lib, &dest).map_err(at(&lib))?;
		fs::write(&marker, lib.to_string_lossy().as_bytes()).map_err(at(&marker))?;
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
	let fresh = !dir.exists();
	let mut git = Command::new("git");
	if !fresh {
		git.arg("-C").arg(&dir).args(["pull", "-q", "--ff-only"]);
	} else {
		git.args(["clone", "-q", "--depth", "1", url]).arg(&dir);
	}
	let mut ok = git.status().map_err(at(&dir))?.success();
	// retry as full clone
	if !ok && fresh {
		let _ = fs::remove_dir_all(&dir);
		ok = Command::new("git")
			.args(["clone", "-q", url])
			.arg(&dir)
			.status()
			.map_err(at(&dir))?
			.success();
	}
	if !ok {
		return Err(fail(format!("{url}: git failed")));
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
	move |e| fail(format!("{path}: {e}"))
}
