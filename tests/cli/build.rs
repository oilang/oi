use std::env::consts::{DLL_PREFIX, DLL_SUFFIX};
use std::process::Command;

use indoc::indoc;

use crate::common::{Project, Run, oi, ok, trim};

/// Build main.oi with, and run the binary.
fn build_and_run(src: &str, args: &[&str], bin: &str) -> String {
	let dir = Project::new().file("main.oi", src);
	let out = oi(&[&["build"][..], args].concat()).current_dir(&dir).run(None);
	assert!(
		out.status.success(),
		"build failed:\n{}",
		String::from_utf8_lossy(&out.stderr)
	);
	let run = std::process::Command::new(dir.as_ref().join(bin)).output().unwrap();
	String::from_utf8_lossy(&run.stdout).trim().to_string()
}

#[test]
fn hello_world() {
	assert_eq!(build_and_run(r#"print("hi")"#, &["main.oi", "-o", "hi"], "hi"), "hi");
}

#[test]
fn macros_comp_foreign_and_leak_check() {
	let main = indoc! {r#"
		use util
		sq! :: fn(e: Ast) Ast { `%e * %e` }
		print(sq!(comp 2 + 3))
		print(util.cube(3))
	"#};
	let util = indoc! {"
		module util
		pub cube :: fn(n: isize) isize { unsafe oi_pow_int(n, 3) }
		oi_pow_int : fn(base: isize, exp: isize) isize : foreign
	"};
	let dir = Project::new().file("main.oi", main).file("util.oi", util);
	ok(oi(&["build"]).current_dir(&dir).run(None));
	let mut bin = Command::new(dir.as_ref().join("main"));
	let out = bin.env("OI_LEAK_CHECK", "1").output().unwrap();
	assert_eq!(trim(&out.stdout), "25\n27");
	assert_eq!(trim(&out.stderr), "leaked allocations: 0");
}

#[test]
fn link_annotation_adds_lib_to_link_line() {
	let main = indoc! {r#"
		use cext
		print(unsafe { cext.zlibVersion().str() }[0..1])
	"#};
	let cext = indoc! {r#"
		module cext
		@link.{"z"}
		pub zlibVersion : fn() cstr : foreign
	"#};
	let dir = Project::new().file("main.oi", main).file("cext.oi", cext);
	ok(oi(&["build"]).current_dir(&dir).run(None));
	let run = Command::new(dir.as_ref().join("main")).output().unwrap();
	assert_eq!(trim(&run.stdout), "1");
}

#[test]
fn link_annotation_accepts_a_file_path() {
	let file = format!("{DLL_PREFIX}dep.x86_64{DLL_SUFFIX}");
	let link = format!(r#"@link.{{"./{file}"}}"#);
	let dir = Project::new().file("dep.c", "long oi_dep(void) { return 42; }").file(
		"main.oi",
		[&link[..], "oi_dep : fn() int : foreign", "print(unsafe oi_dep())"],
	);
	let cc = Command::new("cc")
		.args(["-shared", "-fPIC", &format!("-Wl,-soname,{file}"), "dep.c", "-o", &file])
		.current_dir(&dir)
		.output()
		.unwrap();
	assert!(cc.status.success());
	ok(oi(&["build"]).current_dir(&dir).run(None));
	let run = Command::new(dir.as_ref().join("main")).output().unwrap();
	assert_eq!(trim(&run.stdout), "42");
}

#[test]
fn export_annotation_marks_c_abi_fns() {
	let src = indoc! {r#"
		use cext

		@export
		add :: fn(a: int, b: int) int { a + b }

		@export.{"mul2"}
		pub mul :: fn(a: int, b: int) int { a * b }

		print("lib up")
	"#};
	let cext = indoc! {r#"
		module cext
		Cfg :: struct { pub scale: int }
		cfg := Cfg.{ 3 }
		@export
		pub triple :: fn(n: int) int { n * cfg.scale }
	"#};
	let caller = indoc! {r#"
		#include <stdio.h>
		extern long add(long, long);
		extern long mul2(long, long);
		extern long triple(long);
		int main(void) { printf("%ld %ld %ld\n", add(2, 3), mul2(2, 3), triple(3)); return 0; }
	"#};
	let dir = Project::new()
		.file("main.oi", src)
		.file("cext.oi", cext)
		.file("caller.c", caller);
	ok(oi(&["build", "--lib"]).current_dir(&dir).run(None));
	let lib = dir.as_ref().join(format!("{DLL_PREFIX}main{DLL_SUFFIX}"));
	let mut cc = Command::new("cc");
	cc.arg("caller.c").arg(lib).args(["-o", "caller"]).current_dir(&dir);
	assert!(cc.output().unwrap().status.success());
	let run = Command::new(dir.as_ref().join("caller")).output().unwrap();
	assert_eq!(trim(&run.stdout), "lib up\n5 6 9");
}

#[test]
fn structs_traits_and_generics_reach_the_linker() {
	let src = indoc! {r#"
		Animal :: trait { speak : fn(self) string }
		Dog :: struct { name: string }
		Dog : Animal < { speak :: fn(self) string { self.name + " woofs" } }
		Opt[T] :: enum { nope, some(T) }
		wrap[T] :: fn(v: T) Opt[T] { .some(v) }
		match wrap(Dog.{ "Rex" }) {
			.some(d) => print(d.speak()),
			.nope => print("none"),
		}
	"#};
	assert_eq!(build_and_run(src, &[], "main"), "Rex woofs");
}

#[test]
fn link_searches_module_dir_then_roots() {
	let so = format!("{DLL_PREFIX}dep{DLL_SUFFIX}");
	let main = ["use cext", "print(unsafe cext.oi_dep())"];
	let dir = Project::new()
		.file(
			"cext/cext.oi",
			["module cext", r#"@link.{"dep"}"#, "pub oi_dep : fn() int : foreign"],
		)
		.file("cext/dep.c", "long oi_dep(void) { return 42; }")
		.file("main.oi", main);
	let cc = Command::new("cc")
		.args(["-shared", "-fPIC", &format!("-Wl,-soname,{so}"), "dep.c", "-o", &so])
		.current_dir(dir.as_ref().join("cext"))
		.output()
		.unwrap();
	assert!(cc.status.success());
	ok(oi(&["build"]).current_dir(&dir).run(None));
	assert_eq!(
		trim(&Command::new(dir.as_ref().join("main")).output().unwrap().stdout),
		"42"
	);

	let home = Project::new();
	ok(oi(&["install", "cext"])
		.current_dir(&dir)
		.env("OI_HOME", home.as_ref())
		.run(None));
	let other = Project::new().file("main.oi", main);
	let run = oi(&["run", "main.oi"])
		.current_dir(&other)
		.env("OI_HOME", home.as_ref())
		.run(None);
	assert_eq!(ok(run), "42");
}
