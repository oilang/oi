use std::path::Path;
use std::process::Command;

use indoc::indoc;

use oi::Reported;

const MAIN: &str = indoc! {r#"
	main :: fn() {
		who :: "Mom"
		print("Hi {who}!")
	}
"#};

/// Scaffold a project in the current directory.
pub fn init() -> Result<(), Reported> {
	scaffold(Path::new("."))
}

/// Scaffold a project in a new `name` directory.
pub fn new(name: &str) -> Result<(), Reported> {
	let dir = Path::new(name);
	if dir.exists() {
		eprintln!("oi: {name} already exists");
		return Err(Reported);
	}
	scaffold(dir)
}

fn scaffold(dir: &Path) -> Result<(), Reported> {
	let entry = dir.join("src/main.oi");
	if entry.exists() {
		eprintln!("oi: {} already exists", entry.display());
		return Err(Reported);
	}

	write(&entry, MAIN)?;
	let ignore = dir.join(".gitignore");
	if !ignore.exists() {
		write(&ignore, ".oi/\n")?;
	}
	if !dir.canonicalize().is_ok_and(|p| p.ancestors().any(|a| a.join(".git").exists())) {
		Command::new("git").args(["init", "--quiet"]).arg(dir).status().ok();
	}

	println!("oi: created {}", entry.display());
	Ok(())
}

/// Write a file, creating parent directories.
fn write(path: &Path, content: &str) -> Result<(), Reported> {
	std::fs::create_dir_all(path.parent().unwrap_or(path))
		.and_then(|()| std::fs::write(path, content))
		.map_err(|e| {
			eprintln!("oi: cannot write {}: {e}", path.display());
			Reported
		})
}
