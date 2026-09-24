use crate::helpers::*;

#[test]
fn annotation_on_fn() {
	check(
		indoc! {r#"
			deprecated :: struct { reason: string }
			@:awesome
			@deprecated.{reason = "use speak()"}
			pub yell :: fn() string { "AAA" }
			print(yell())
		"#},
		"AAA",
	);
}

#[test]
fn field_annotation_trailing_form() {
	check(
		indoc! {r#"
			Player :: struct { name: string @:required }
			p := Player.{ name = "Todd" }
			print(p.name)
		"#},
		"Todd",
	);
}

#[test]
fn bare_const_marker_from_core() {
	check(
		indoc! {r#"
			User :: struct { name: string @required }
			u := User.{ name = "Todd" }
			print(u.name)
		"#},
		"Todd",
	);
}

#[test]
fn main_file_const_as_annotation() {
	check(
		indoc! {r#"
			opt :: ()
			deprecated :: struct { reason: string }
			warn :: deprecated.{reason = "old"}
			@warn
			speak :: fn() string { "hi" }
			User :: struct { name: string @opt }
			u := User.{ name = "Todd" }
			print("{speak()} {u.name}")
		"#},
		"hi Todd",
	);
}

#[test]
fn builtin_marker_is_a_real_value() {
	check("print(required)", "()");
}

#[test]
fn unknown_annotation_is_not_a_struct() {
	fail(
		indoc! {"
			@bogus.{}
			thing :: fn() int { 1 }
		"},
		"`bogus` is not a struct",
	);
}

#[test]
fn annotation_arg_must_be_literal() {
	fail(
		indoc! {r#"
			deprecated :: struct { reason: string }
			x :: "hi"
			@deprecated.{x}
			thing :: fn() int { 1 }
		"#},
		"annotation arguments must be literal values",
	);
}

#[test]
fn annotation_field_set_twice() {
	fail(
		indoc! {r#"
			deprecated :: struct { reason: string }
			@deprecated.{"a", reason = "b"}
			thing :: fn() int { 1 }
		"#},
		"`reason` was already set positionally",
	);
}

#[test]
fn annotation_only_goes_on_definitions() {
	fail(
		indoc! {"
			Foo :: struct {}
			@:awesome
			Foo :< Eq
		"},
		"annotations only attach to definitions",
	);
}

#[test]
fn bare_struct_name_is_its_zero_value() {
	check(
		indoc! {r#"
			deprecated :: struct { reason: string }
			@deprecated
			thing :: fn() string { "ok" }
			print(thing())
		"#},
		"ok",
	);
}

#[test]
fn required_field_omitted() {
	fail(
		indoc! {"
			Foo :: struct { n: int @required }
			f := Foo.{}
		"},
		"`Foo.n` is required",
	);
}

#[test]
fn required_field_provided() {
	check(
		indoc! {r#"
			Foo :: struct { n: int @required }
			a := Foo.{n = 1}
			b := Foo.{2}
			c := Foo.{..a}
			print("{a.n} {b.n} {c.n}")
		"#},
		"1 2 1",
	);
}

#[test]
fn generic_required_omitted() {
	fail(
		indoc! {"
			Box[T] :: struct { v: T, n: int @required }
			b := Box.{v = 1}
		"},
		"is required",
	);
}

#[test]
fn params_omits_trailing_struct() {
	check(
		indoc! {"
			@params
			Settings :: struct { idk: int = 7 }
			User :: struct {}
			User :< {
				with_settings :: fn(self, settings: Settings) { print(settings.idk) }
			}
			take :: fn(settings: Settings) { print(settings.idk) }
			u := User.{}
			u.with_settings()
			take()
			take(.{idk = 1})
		"},
		["7", "7", "1"],
	);
}

#[test]
fn params_only_on_marked_struct() {
	fail(
		indoc! {"
			Settings :: struct { idk: int = 7 }
			take :: fn(settings: Settings) { print(settings.idk) }
			take()
		"},
		"expects 1 argument",
	);
}

#[test]
fn annotation_fn_call_is_folded() {
	check(
		indoc! {r#"
			deprecated :: struct { reason: string }
			mark :: fn(who: string) deprecated { deprecated.{ "use " + who } }
			@mark("speak")
			yell :: fn() string { "AAA" }
			print(yell())
		"#},
		"AAA",
	);
}

#[test]
fn params_synthesized_literal_checks_required() {
	fail(
		indoc! {"
			@params
			Settings :: struct { idk: int @required }
			take :: fn(settings: Settings) { print(settings.idk) }
			take()
		"},
		"is required",
	);
}

#[test]
fn bare_attr_macro_is_identity() {
	check(
		indoc! {"
			keep! :: fn(input: Ast) Ast { `%input` }
			@keep!
			f :: fn() int { 42 }
			print(f())
		"},
		"42",
	);
}

#[test]
fn attr_macro_over_annotated_def() {
	check(
		indoc! {"
			deprecated :: struct { reason: string }
			keep! :: fn(input: Ast) Ast {
				print(input.notes[0].str())
				`%input`
			}
			@keep!
			@deprecated
			f :: fn() int { 42 }
			print(f())
		"},
		["deprecated", "42"],
	);
}

// binds, struct fills and claim fills
#[test]
fn attr_macro_positions() {
	check(
		indoc! {"
			keep! :: fn(input: Ast) Ast { `%input` }
			Show :: trait { show : fn(self) int }
			Point :: struct {
				x: int
				@keep! double :: fn(self) int { self.x * 2 }
			}
			Point : Show < {
				@keep! show :: fn(self) int { self.x }
			}
			@keep! answer :: 5
			print(Point.{ x = 7 }.show() + Point.{ x = 1 }.double() + answer)
		"},
		"14",
	);
}

#[test]
fn attr_macro_args_arrive_as_one_list() {
	check(
		indoc! {"
			def! :: fn(input: Ast, args: Ast) Ast {
				a := args.items
				name := a[0]
				`
				%name :: %{a[1]}
				%input
				`
			}
			@def!(VERSION, 3)
			compute :: fn() int { 39 }
			print(VERSION + compute())
		"},
		"42",
	);
}

#[test]
fn qualified_attr_macro() {
	crate::common::Project::new()
		.file(
			"main.oi",
			[
				"module main",
				"use util",
				"@util.keep!",
				"f :: fn() int { 42 }",
				"print(f())",
			],
		)
		.file(
			"util/lib.oi",
			["module util", "pub keep! :: fn(input: Ast) Ast { `%input` }"],
		)
		.check("42");
}

#[test]
fn qualified_field_annotation() {
	crate::common::Project::new()
		.file(
			"main.oi",
			[
				"module main",
				"use godot",
				"read_export! :: fn(input: Ast) Ast {",
				"	n := input.items[0].notes[0]",
				"	print(n.items[0])",
				"	print(n.items[1])",
				"	`%input`",
				"}",
				"@read_export!",
				"Thing :: struct { x: int @godot.export.{ lo = 0, hi = 10 } }",
			],
		)
		.file(
			"godot/lib.oi",
			["module godot", "pub export :: struct { lo: int, hi: int }"],
		)
		.check(["0", "10"]);
}

#[test]
fn export_rejects_non_c_repr_params() {
	fail(
		indoc! {r#"
			@export
			greet :: fn(name: string) int { 1 }
		"#},
		"has no C representation",
	);
}

#[test]
fn plain_fn_with_fn_pointer_param_needs_c_marker() {
	fail(
		indoc! {r#"
			@export
			call_with_getter_cb :: fn(cb: fn(get: fn() i32) i32) i32 { 42 }
			my_cb :: fn(get: fn() i32) i32 { get() }
			print(call_with_getter_cb(my_cb))
		"#},
		"mark it with `@c`",
	);
}

#[test]
fn c_marked_fn_crosses_the_c_abi_without_a_symbol() {
	check(
		indoc! {r#"
			@export
			call_with_getter_cb :: fn(cb: fn(get: fn() i32) i32) i32 { 42 }
			@c
			my_cb :: fn(get: fn() i32) i32 { get() }
			print(call_with_getter_cb(my_cb))
		"#},
		"42",
	);
}

#[test]
fn c_fn_types() {
	check(
		indoc! {"
			GetProc :: @c fn(name: cstr) ptr
			@c
			Iface :: struct { init: @c fn() bool, tick: @c fn(dt: f32) }
			print(Iface.size)
			print(@c fn (n: i32) bool { n > 0 })
		"},
		["16", "<fn>"],
	);
	check(
		indoc! {"
			C :: @c fn(i32) i32
			f: C = @c fn(n) { n + 1 }
			print(f(1))
		"},
		"2",
	);
}

#[test]
fn annotated_types_are_checked() {
	fail(["f :: fn(cb: @c fn(s: string)) {}"], "`@c fn` can't cross the C ABI");
	fail(
		["n := 3", "h := @c fn (x: i32) i32 { x + n }"],
		"`@c` fns can't capture",
	);
	fail(
		["f :: fn(cb: @stdcall fn(x: i32)) {}"],
		"`@stdcall fn(i32) ()` isn't a type",
	);
	fail(["x := @c 5"], "`@c int` isn't a type");
}

#[test]
fn pure_fn_calls_pure_fn() {
	check(
		indoc! {"
			@pure
			square :: fn(x: int) int { x * x }
			@pure
			sum_of_squares :: fn(a: int, b: int) int { square(a) + square(b) }
			print(sum_of_squares(2, 3))
		"},
		"13",
	);
}

#[test]
fn pure_fn_rejects_impure_calls() {
	fail(
		["@pure", "f :: fn() { print(1) }", "f()"],
		"isn't allowed in a `@pure` fn",
	);
	fail(
		["g :: fn() int { 1 }", "@pure", "f :: fn() int { g() }", "f()"],
		"isn't allowed in a `@pure` fn",
	);
}

#[test]
fn unknown_attr_macro_errors() {
	fail(
		indoc! {"
			@nope!
			f :: fn() int { 1 }
		"},
		"no macro named",
	);
}
