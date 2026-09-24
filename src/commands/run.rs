use std::path::{Path, PathBuf};

use oi::Reported;
use oi::driver::{DebugOpts, build_source, run_source, test_source};
use oi::loader::Entry;

/// Resolve the entry file.
pub fn entry(file: Option<PathBuf>) -> PathBuf {
	if let Some(file) = file {
		return file;
	}
	let cwd_path = PathBuf::from("main.oi");
	if cwd_path.exists() {
		cwd_path
	} else {
		PathBuf::from("src/main.oi")
	}
}

/// Run a source file, or a directory's `.oi` files.
pub fn run(file: &Path, opts: DebugOpts) -> Result<(), Reported> {
	run_source(files(file)?, root(file), opts)
}

/// Compile a source file to a native executable or shared library.
pub fn build(file: &Path, out: Option<&Path>, lib: bool, timings: bool) -> Result<(), Reported> {
	let stem = stem(file);
	let default = match lib {
		true => format!("{}{stem}{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX).into(),
		false => PathBuf::from(&stem),
	};
	let opts = DebugOpts {
		timings,
		..Default::default()
	};
	build_source(files(file)?, root(file), &stem, out.unwrap_or(&default), lib, opts)
}

/// Compile a source file and call its `@test` fns.
pub fn test(file: &Path, pattern: Option<&str>) -> Result<(), Reported> {
	test_source(files(file)?, root(file), pattern)
}

pub fn files(file: &Path) -> Result<Entry, Reported> {
	if !file.is_dir() {
		return Ok(vec![(file.display().to_string(), read(file)?)]);
	}
	let mut paths: Vec<PathBuf> = std::fs::read_dir(file)
		.map_err(unreadable(file))?
		.flatten()
		.map(|e| e.path())
		.filter(|p| p.extension().is_some_and(|x| x == "oi"))
		.collect();
	paths.sort();
	paths.into_iter().map(|p| Ok((p.display().to_string(), read(&p)?))).collect()
}

/// Read a source file.
pub fn read(file: &Path) -> Result<String, Reported> {
	std::fs::read_to_string(file).map_err(unreadable(file))
}

fn unreadable(file: &Path) -> impl Fn(std::io::Error) -> Reported + '_ {
	move |e| {
		eprintln!("oi: cannot read {}: {e}", file.display());
		Reported
	}
}

pub fn root(file: &Path) -> &Path {
	if file.is_dir() {
		return file;
	}
	file.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

/// The file's name or dirname.
pub fn stem(file: &Path) -> String {
	let full = file.canonicalize().unwrap_or_default();
	full.file_stem().and_then(|s| s.to_str()).unwrap_or("main").into()
}
