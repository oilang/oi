use indoc::indoc;

use crate::common::Project;
use crate::helpers::{check, fail};

#[test]
fn static_is_shared_by_every_fn() {
	let src = indoc! {r#"
		counter := 0
		foo: []int
		idk := [ "foo" "bar" ]

		bump :: fn() { counter = counter + 1 }
		peek :: fn() { print(idk) }

		main :: fn() {
			bump()
			bump()
			peek()
			idk << "baz"
			print(counter, foo.len, idk.len)
		}
	"#};
	check(src, [r#"["foo", "bar"]"#, "2 0 3"]);
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
				"main :: fn() { mem.track(3); mem.track(4); print(mem.used, mem.RED == mem.E.green) }",
			],
		)
		.file(
			"mem.oi",
			[
				"module mem",
				"pub used := 0",
				"pub track :: fn(n: int) { used = used + n }",
				"pub E :: enum { red green }",
				"pub RED :: E.green",
			],
		)
		.check("7 true");
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
fn array_static_rejects_mixed_element_types() {
	let src = indoc! {r#"
		idk := [ "foo" 3 ]
		main :: fn() { print(idk) }
	"#};
	fail(src, "array elements are string and int");
}

#[test]
fn empty_array_static_needs_an_annotation() {
	let src = indoc! {"
		idk := []
		main :: fn() { print(idk) }
	"};
	fail(src, "an empty array needs a type annotation");
}
