use std::process::Command;

// Export `oi_*` functions for `dlsym`.
fn main() {
	println!("cargo:rustc-link-arg-bins=-rdynamic");

	let sha = Command::new("git")
		.args(["rev-parse", "--short", "HEAD"])
		.output()
		.ok()
		.filter(|o| o.status.success())
		.and_then(|o| String::from_utf8(o.stdout).ok())
		.map(|s| s.trim().to_string())
		.unwrap_or_else(|| "unknown".to_string());
	println!("cargo:rustc-env=OI_GIT_SHA={sha}");
	println!("cargo:rustc-env=OI_TARGET={}", std::env::var("TARGET").unwrap());
	println!("cargo:rerun-if-changed=.git/HEAD");
}
