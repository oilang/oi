use std::env::consts::{DLL_PREFIX, DLL_SUFFIX};
use std::path::Path;
use std::process::Command;

use indoc::indoc;

use crate::common::{Project, Run, bin, oi, ok, stderr, trim};

fn cc_shared(dir: impl AsRef<Path>, so: &str) {
	let soname = format!("-Wl,-soname,{so}");
	let cc = Command::new("cc")
		.args(["-shared", "-fPIC", &soname, "dep.c", "-o", so])
		.current_dir(dir)
		.output()
		.unwrap();
	assert!(cc.status.success());
}

/// Build main.oi with, and run the binary.
fn build_and_run(src: &str, args: &[&str], name: &str) -> String {
	let dir = Project::new().file("main.oi", src);
	let out = dir.oi(&[&["build"][..], args].concat());
	assert!(out.status.success(), "build failed:\n{}", stderr(&out));
	trim(&bin(dir.as_ref().join(name)).stdout)
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
	dir.ok(&["build"]);
	let mut bin = Command::new(dir.as_ref().join("main"));
	let out = bin.env("OI_LEAK_CHECK", "1").output().unwrap();
	assert_eq!(trim(&out.stdout), "25\n27");
	assert_eq!(trim(&out.stderr), "leaked allocations: 0");
}

#[test]
fn ast_fns_stay_out_of_the_binary() {
	let src = indoc! {"
		width :: fn(p: (Ast, int)) int { p.0.items.len() + p.1 }
		print(1)
	"};
	assert_eq!(build_and_run(src, &["main.oi", "-o", "hi"], "hi"), "1");
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
	dir.ok(&["build"]);
	assert_eq!(trim(&bin(dir.as_ref().join("main")).stdout), "1");
}

#[test]
fn link_annotation_accepts_a_file_path() {
	let file = format!("{DLL_PREFIX}dep.x86_64{DLL_SUFFIX}");
	let link = format!(r#"@link.{{"./{file}"}}"#);
	let dir = Project::new().file("dep.c", "long oi_dep(void) { return 42; }").file(
		"main.oi",
		[&link[..], "oi_dep : fn() int : foreign", "print(unsafe oi_dep())"],
	);
	cc_shared(&dir, &file);
	dir.ok(&["build"]);
	assert_eq!(trim(&bin(dir.as_ref().join("main")).stdout), "42");
}

#[test]
fn c_structs_cross_by_value() {
	let dep = indoc! {"
		typedef struct { int a; double b; } Small;
		typedef struct { long n; struct { int k; _Bool on; } inner; double d[3]; } Big;
		double small(long pad, Small s) { return pad + s.a + s.b; }
		double big(Big b, long tail) { return b.n + b.inner.k + b.inner.on + b.d[0] + b.d[1] + b.d[2] + tail; }
		typedef struct { float x, y; } Vec2;
		Vec2 vec2(float x) { return (Vec2){ x, x * 2 }; }
		Small twice(Small s) { return (Small){ s.a * 2, s.b * 2 }; }
		Big bump(Big b) { b.inner.k++; b.d[2]++; return b; }
		double spill(long a, long b, long c, long d, long e, long f, Small s, double g) { return f + s.a * 10 + s.b * 100 + g * 1000; }
	"};
	let main = indoc! {r#"
		@c
		Small :: struct { a: i32, b: f64 }
		@c
		Inner :: struct { k: i32, on: bool }
		@c
		Big :: struct { n: int, inner: Inner, d: [3]f64 }
		@link.{"dep"}
		small : fn(pad: int, s: Small) f64 : foreign
		big : fn(b: Big, tail: int) f64 : foreign
		@c
		Vec2 :: struct { x: f32, y: f32 }
		vec2 : fn(x: f32) Vec2 : foreign
		twice : fn(s: Small) Small : foreign
		bump : fn(b: Big) Big : foreign
		spill : fn(a: int, b: int, c: int, d: int, e: int, f: int, s: Small, g: f64) f64 : foreign
		print(unsafe small(1, Small.{ a = 2, b = 0.5 }))
		print(unsafe big(Big.{ n = 1, inner = Inner.{ k = 2, on = true }, d = .[0.25 0.25 0.5] }, 10))
		v := unsafe vec2(1.5)
		s := unsafe twice(Small.{ a = 2, b = 0.5 })
		b := unsafe bump(Big.{ n = 1, inner = Inner.{ k = 2, on = true }, d = .[0.25 0.25 0.5] })
		print(v.x + v.y, s.a, s.b)
		print(b.n, b.inner.k, b.inner.on, b.d[2])
		print(unsafe spill(0, 0, 0, 0, 0, 1, Small.{ a = 2, b = 3.0 }, 4.0))
		f := unsafe small
		apply :: fn(g: fn(pad: int, s: Small) f64) f64 { g(2, Small.{ a = 2, b = 0.5 }) }
		print(f(1, Small.{ a = 2, b = 0.5 }), apply(unsafe small))
	"#};
	let dir = Project::new().file("dep.c", dep).file("main.oi", main);
	cc_shared(&dir, &format!("{DLL_PREFIX}dep{DLL_SUFFIX}"));
	dir.check(["3.5", "15.0", "4.5 4 1.0", "1 3 true 1.5", "4321.0", "3.5 4.5"]);
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
	dir.ok(&["build", "--lib"]);
	let lib = dir.as_ref().join(format!("{DLL_PREFIX}main{DLL_SUFFIX}"));
	let mut cc = Command::new("cc");
	cc.arg("caller.c").arg(lib).args(["-o", "caller"]).current_dir(&dir);
	assert!(cc.output().unwrap().status.success());
	assert_eq!(trim(&bin(dir.as_ref().join("caller")).stdout), "lib up\n5 6 9");
}

#[test]
fn c_calls_oi_with_structs_by_value() {
	let src = indoc! {"
		@c
		Vec2 :: struct { x: f32, y: f32 }
		@c
		Big :: struct { n: int, d: [3]f64 }
		@export
		sum :: fn(v: Vec2, b: Big) f64 { f64.(v.x + v.y) + f64.(b.n) + b.d[2] }
		@export
		grow :: fn(n: int, b: Big) Big { .{ n = b.n + n, d = b.d } }
		@export
		apply :: fn(f: @c fn(v: Vec2) Vec2, g: @c fn(n: int) Big, v: Vec2) Vec2 { .{ x = f(v).y, y = f32.(g(4).n) } }
	"};
	let caller = indoc! {r#"
		#include <stdio.h>
		typedef struct { float x, y; } Vec2;
		typedef struct { long n; double d[3]; } Big;
		extern double sum(Vec2, Big);
		extern Big grow(long, Big);
		extern Vec2 apply(Vec2 (*)(Vec2), Big (*)(long), Vec2);
		static Vec2 flip(Vec2 v) { return (Vec2){ v.y, v.x }; }
		static Big make(long n) { return (Big){ n, { 0.5, 1.5, 2.5 } }; }
		int main(void) {
			Vec2 v = { 1.5, 2 }, a = apply(flip, make, v);
			printf("%g %g %g\n", sum(v, grow(3, make(7))), a.x, a.y);
			return 0;
		}
	"#};
	let dir = Project::new().file("main.oi", src).file("caller.c", caller);
	dir.ok(&["build", "--lib"]);
	let lib = dir.as_ref().join(format!("{DLL_PREFIX}main{DLL_SUFFIX}"));
	let mut cc = Command::new("cc");
	cc.arg("caller.c").arg(lib).args(["-o", "caller"]).current_dir(&dir);
	assert!(cc.output().unwrap().status.success());
	assert_eq!(trim(&bin(dir.as_ref().join("caller")).stdout), "16 1.5 4");
}

#[test]
fn structs_traits_and_generics_reach_the_linker() {
	let src = indoc! {r#"
		Animal :: trait { speak : fn(self) string }
		Dog :: struct { name: string }
		Dog : Animal < { speak :: fn(self) string { self.name + " woofs" } }
		Opt[T] :: enum { nope, some(T) }
		wrap[T] :: fn(v: T) Opt[T] { .some.(v) }
		match wrap(Dog.{ "Rex" }) {
			.some.(d) => print(d.speak()),
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
	cc_shared(dir.as_ref().join("cext"), &so);
	dir.ok(&["build"]);
	assert_eq!(trim(&bin(dir.as_ref().join("main")).stdout), "42");

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
