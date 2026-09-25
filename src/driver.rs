use std::ffi::{CString, c_char};
use std::io::Write as _;
use std::path::Path;
use std::time::Instant;

use crate::Reported;
use crate::compiler::Compiler;
use crate::lexer::lex_at;
use crate::loader::{self, Entry};

/// A compilation stage to dump to stderr instead of running.
#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq)]
pub enum Emit {
	Tokens,
	Ast,
	Clif,
}

/// Flags that toggle introspection.
#[derive(Default, Clone, Copy)]
pub struct DebugOpts {
	pub timings: bool,
	pub emit: Option<Emit>,
	pub check: bool,
}

/// Compile and run a program from its entry files.
pub fn run_source(entry: Entry, root: &Path, args: &[String], opts: DebugOpts) -> Result<(), Reported> {
	let mut compiler = Compiler::default();
	compiler.emit_clif = opts.emit == Some(Emit::Clif);
	if opts.emit == Some(Emit::Tokens) {
		for (name, src) in &entry {
			eprintln!("-- {name} --");
			for (tok, span) in lex_at(src, 0) {
				eprintln!("{span:?} {tok:?}");
			}
		}
	}

	let t = Instant::now();
	let program = loader::load(entry, root)?;
	compiler.timings.push(("load", t.elapsed()));

	if opts.emit == Some(Emit::Ast) {
		for m in program.modules.iter().filter(|m| m.name == "main") {
			eprintln!("{:#?}", m.items);
		}
	}

	let code = match compiler.compile(&program) {
		Ok(code) => code,
		Err(error) => {
			error.report_mapped(&program.map);
			return Err(Reported);
		}
	};
	if opts.check {
		return Ok(());
	}

	// run
	let t = Instant::now();
	let cstrs: Vec<CString> = args.iter().filter_map(|a| CString::new(a.as_str()).ok()).collect();
	let argv: Vec<*const c_char> = cstrs.iter().map(|a| a.as_ptr()).collect();
	// SAFETY: `argv` holds `cstrs`'s pointers, alive for the call.
	unsafe { crate::runtime::set_args(argv.len() as i32, argv.as_ptr()) };
	// SAFETY: `code` is the finalized `__oi_main` entrypoint emitted by `compile`.
	let f = unsafe { std::mem::transmute::<*const u8, fn()>(code) };
	f();
	compiler.timings.push(("run", t.elapsed()));
	if opts.timings {
		compiler.report_timings();
	}
	crate::runtime::epilogue();
	Ok(())
}

/// The static runtime, embedded so the compiler is a single self-contained binary.
static RUNTIME: &[u8] = include_bytes!(env!("CARGO_STATICLIB_FILE_OI_RUNTIME_oi_runtime"));

// What the runtime staticlib needs linked in.
// `rustc --print native-static-libs`.
#[cfg(target_os = "linux")]
const LIBS: &[&str] = &["-lgcc_s", "-lutil", "-lrt", "-lpthread", "-lm", "-ldl"];
#[cfg(target_os = "android")]
const LIBS: &[&str] = &["-ldl", "-lm"];
#[cfg(target_os = "macos")]
const LIBS: &[&str] = &["-lSystem", "-lc", "-lm"];
#[cfg(windows)]
const LIBS: &[&str] = &[
	"-lkernel32",
	"-ladvapi32",
	"-lbcrypt",
	"-lntdll",
	"-luserenv",
	"-lws2_32",
];

/// Compile a program to a native executable at `out`, linked against the static runtime.
pub fn build_source(
	entry: Entry,
	root: &Path,
	stem: &str,
	out: &Path,
	lib: bool,
	opts: DebugOpts,
) -> Result<(), Reported> {
	let mut compiler = Compiler::object(stem, lib);
	let t = Instant::now();
	let program = loader::load(entry, root)?;
	compiler.timings.push(("load", t.elapsed()));
	let (obj, link_libs) = compiler.compile_object(&program, opts.timings).map_err(|e| {
		e.report_mapped(&program.map);
		Reported
	})?;
	let tmp = std::env::temp_dir().join(format!("oi_{}", std::process::id()));
	let write = |ext: &str, bytes: &[u8]| {
		let path = tmp.with_extension(ext);
		std::fs::write(&path, bytes)
			.map(|_| path)
			.map_err(|e| fail(format!("cannot write {}: {e}", tmp.display())))
	};
	let libs = link_libs.iter().flat_map(|l| match Path::new(l).is_absolute() {
		true => vec![
			l.clone(),
			format!("-Wl,-rpath,{}", Path::new(l).parent().unwrap().display()),
		],
		false => vec![format!("-l{l}")],
	});
	let cc = std::process::Command::new("cc")
		.args([write("o", &obj)?, write("a", RUNTIME)?])
		.args(if lib { &["-shared"][..] } else { &[] })
		.args(LIBS)
		.args(libs)
		.arg("-o")
		.arg(out)
		.output();
	for ext in ["o", "a"] {
		std::fs::remove_file(tmp.with_extension(ext)).ok();
	}
	let cc = cc.map_err(|e| fail(format!("cc: {e}")))?;
	if !cc.status.success() {
		std::io::stderr().write_all(&cc.stderr).ok();
		return Err(Reported);
	}
	Ok(())
}

fn fail(msg: impl std::fmt::Display) -> Reported {
	eprintln!("oi: {msg}");
	Reported
}

/// Compile a program in test mode and run every `@test` fn in the main module.
pub fn test_source(entry: Entry, root: &Path, pattern: Option<&str>) -> Result<(), Reported> {
	let program = loader::load(entry, root)?;
	let mut compiler = Compiler::default();
	compiler.include_tests = true;
	if let Err(error) = compiler.compile(&program) {
		error.report_mapped(&program.map);
		return Err(Reported);
	}
	let total = compiler.tests.len();
	if let Some(pattern) = pattern {
		compiler.tests.retain(|(_, display, _)| display.contains(pattern));
	}
	let filtered = total - compiler.tests.len();
	let mut passed = 0;
	let mut skipped = 0;
	for (fn_name, display, skip) in &compiler.tests {
		print!("test {display} ... ");
		std::io::Write::flush(&mut std::io::stdout()).ok();
		if *skip {
			println!("skipped");
			skipped += 1;
			continue;
		}
		// SAFETY: there are no other threads, the child only runs the fn and exits
		let ok = match unsafe { libc::fork() } {
			0 => {
				compiler.finalized_test(fn_name)();
				std::process::exit(0)
			}
			-1 => {
				eprintln!("oi: fork failed");
				return Err(Reported);
			}
			pid => {
				let mut status = 0;
				// SAFETY: `pid` is our own child, `status` is a valid out-pointer
				unsafe { libc::waitpid(pid, &mut status, 0) };
				libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
			}
		};
		println!("{}", if ok { "ok" } else { "FAILED" });
		passed += ok as usize;
	}
	let failed = compiler.tests.len() - passed - skipped;
	let tail: String = [(failed, "failed"), (skipped, "skipped"), (filtered, "filtered out")]
		.iter()
		.filter(|(n, _)| *n > 0)
		.map(|(n, what)| format!("; {n} {what}"))
		.collect();
	println!("{passed} passed{tail}");
	if failed == 0 { Ok(()) } else { Err(Reported) }
}
