use indoc::indoc;

use crate::common::Project;
use crate::helpers::{check, fail};

#[test]
fn static_is_shared_by_every_fn() {
	let src = indoc! {"
		counter := 0

		bump :: fn() { counter = counter + 1 }

		main :: fn() {
			bump()
			bump()
			print(counter)
		}
	"};
	check(src, "2");
}

#[test]
fn struct_static_is_assignable_whole_and_by_field() {
	let src = indoc! {r#"
		Godot :: struct { ptr: int, name: string }

		handle := Godot.{}

		boot :: fn() { handle = Godot.{ 7, "godot" } }

		main :: fn() {
			print(handle.ptr)
			boot()
			print("{handle.ptr} {handle.name}")
			handle.ptr = 9
			print(handle.ptr)
		}
	"#};
	check(src, ["0", "7 godot", "9"]);
}

#[test]
fn pub_static_crosses_modules() {
	Project::new()
		.file(
			"main.oi",
			[
				"use mem",
				"main :: fn() { mem.track(3); mem.track(4); print(mem.used) }",
			],
		)
		.file(
			"mem.oi",
			[
				"module mem",
				"pub used := 0",
				"pub track :: fn(n: int) { used = used + n }",
			],
		)
		.check("7");
}

#[test]
fn static_is_read_only_outside_its_module() {
	Project::new()
		.file("main.oi", ["use mem", "main :: fn() { mem.used = 9 }"])
		.file("mem.oi", ["module mem", "pub used := 0"])
		.fail_with("cannot assign to `mem.used` outside module `mem`");
}

#[test]
fn static_needs_a_comptime_initializer() {
	Project::new()
		.file("main.oi", ["use thing", "main :: fn() { print(thing.n) }"])
		.file(
			"thing.oi",
			["module thing", "pub n := compute()", "compute :: fn() int { 5 }"],
		)
		.fail_with("a static needs a comptime initializer");
}

#[test]
fn pure_fns_cannot_touch_statics() {
	let src = indoc! {"
		total := 42

		@pure
		peek :: fn() int { total }

		main :: fn() { print(peek()) }
	"};
	fail(src, "`total` isn't allowed in a `@pure` fn");
}

#[test]
fn top_level_const_works_alongside_main() {
	let src = indoc! {r#"
		N :: 3
		A :: 1 + 2
		GREETING :: "hi"
		E :: enum { red green }
		RED :: E.green

		main :: fn() {
			buf : [N]int
			print("{GREETING} {buf.len} {A}")
			print(match RED {
				E.red => "r",
				E.green => "g",
			})
		}
	"#};
	check(src, ["hi 3 3", "g"]);
}

#[test]
fn typed_static_without_init_zeroes() {
	let src = indoc! {"
		foo: []int
		main :: fn() { print(foo.len) }
	"};
	check(src, "0");
}

#[test]
fn const_enum_variant_crosses_modules() {
	Project::new()
		.file("main.oi", ["use mem", "main :: fn() { print(mem.RED == mem.E.green) }"])
		.file(
			"mem.oi",
			["module mem", "pub E :: enum { red green }", "pub RED :: E.green"],
		)
		.check("true");
}
