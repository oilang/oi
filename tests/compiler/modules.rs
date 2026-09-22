use indoc::indoc;

use crate::common::Project;
use crate::helpers::{check, fail_with};

#[test]
fn pub_fn_runs() {
	check(r#"pub foo :: fn() { print("hi") } foo()"#, "hi");
}

#[test]
fn module_decl() {
	check(["module main", r#"print("ok")"#], "ok");
	fail_with("module other", "the entry file is module `main`");
	fail_with(["x :: 1", "module main"], "`module` must come first");
}

#[test]
fn import_missing() {
	fail_with("use nope", "cannot find module `nope`");
}

#[test]
fn import_std() {
	check(["use core", r#"print("ok")"#], "ok");
}

#[test]
fn import_trait() {
	check(["use core.{ Ord }", "print(int is Ord)"], "true");
	check(["use core.{ Order :: Ord }", "print(int is Order)"], "true");
}

#[test]
fn import_nested_path() {
	fail_with("use a.b.c", "nested module paths aren't supported yet");
	fail_with("x :: use a.b.{ c }", "nested module paths aren't supported yet");
}

#[test]
fn rt_is_internal_to_core() {
	fail_with(["use rt"], "internal to core");
}

#[test]
fn foreign_fn() {
	let src = indoc! {r#"
		abs : fn(x: i32) i32 : foreign
		main :: fn() { print(unsafe abs(-5)) }
	"#};
	check(src, "5");
}

#[test]
fn foreign_resolves_process_symbols() {
	Project::new()
		.file("main.oi", ["use cext", "print(unsafe cext.abs(-5))"])
		.file("cext.oi", ["module cext", "pub abs : fn(x: i32) i32 : foreign"])
		.check("5");
}

#[test]
fn foreign_cstr_param_calls_strlen() {
	Project::new()
		.file("main.oi", ["use cext", r#"print(unsafe cext.strlen("hi!".cstr()))"#])
		.file("cext.oi", ["module cext", "pub strlen : fn(s: cstr) usize : foreign"])
		.check("3");
}

#[test]
fn foreign_ptr_roundtrips() {
	Project::new()
		.file(
			"main.oi",
			[
				"use cext",
				"p := unsafe cext.malloc(16)",
				"unsafe cext.free(p)",
				":done",
			],
		)
		.file(
			"cext.oi",
			[
				"module cext",
				"pub malloc : fn(size: usize) ptr : foreign",
				"pub free : fn(p: ptr) : foreign",
			],
		)
		.check(":done");
}

#[test]
fn foreign_writes_through_array_ptr() {
	Project::new()
		.file(
			"main.oi",
			[
				"use cext",
				"buf := [1, 2, 3]",
				"unsafe cext.memset(buf.ptr, 0, 4)",
				"print(buf)",
			],
		)
		.file(
			"cext.oi",
			["module cext", "pub memset : fn(p: ptr, c: int, n: usize) : foreign"],
		)
		.check("[0, 2, 3]");
}

#[test]
fn foreign_typed_read_copies_out() {
	Project::new()
		.file(
			"main.oi",
			[
				"use cext",
				"buf: []i32 = .[7, 8, 9]",
				"unsafe cext.memset(buf.ptr, 0, 4)",
				"print(unsafe buf.ptr.array[i32](3))",
			],
		)
		.file(
			"cext.oi",
			["module cext", "pub memset : fn(p: ptr, c: int, n: usize) : foreign"],
		)
		.check("[0, 8, 9]");
}

#[test]
fn fn_type_alias_casts_a_ptr() {
	Project::new()
		.file(
			"main.oi",
			[
				"use cext",
				"Abs :: fn(n: i32) i32",
				r#"abs := unsafe { Abs(cext.dlsym(ptr(0), "abs")) }"#,
				"print(abs(-5))",
			],
		)
		.file(
			"cext.oi",
			["module cext", "pub dlsym : fn(handle: ptr, name: cstr) ptr : foreign"],
		)
		.check("5");
}

#[test]
fn a_c_fn_sheds_its_cell_and_takes_it_back() {
	Project::new()
		.file(
			"main.oi",
			[
				"use cext",
				"Abs :: @c fn(n: i32) i32",
				"on_init :: fn(get: Abs) i32 { get(-21) * 2 }",
				r#"run :: fn(init: @c fn(get: Abs) i32) i32 { init(unsafe Abs(cext.dlsym(ptr(0), "abs"))) }"#,
				"print(run(on_init))",
				r#"boxed: fn(n: i32) i32 = unsafe Abs(cext.dlsym(ptr(0), "abs"))"#,
				"print(boxed(-5))",
			],
		)
		.file(
			"cext.oi",
			["module cext", "pub dlsym : fn(handle: ptr, name: cstr) ptr : foreign"],
		)
		.check(["42", "5"]);
}

#[test]
fn foreign_takes_an_oi_fn_as_a_callback() {
	Project::new()
		.file(
			"main.oi",
			[
				"use cext",
				"cmp :: fn(a: ptr, b: ptr) i32 { unsafe { a.array[i32](1)[0] - b.array[i32](1)[0] } }",
				"buf: []i32 = .[3, 1, 2]",
				"unsafe cext.qsort(buf.ptr, 3, 4, cmp)",
				"print(buf)",
			],
		)
		.file(
			"cext.oi",
			[
				"module cext",
				"pub qsort : fn(base: ptr, n: usize, size: usize, cmp: fn(a: ptr, b: ptr) i32) : foreign",
			],
		)
		.check("[1, 2, 3]");
}

#[test]
fn foreign_takes_an_inline_callback() {
	Project::new()
		.file(
			"main.oi",
			[
				"use cext",
				"buf: []i32 = .[3, 1, 2]",
				"unsafe {",
				"cext.qsort(buf.ptr, 3, 4, fn(a: ptr, b: ptr) i32 { unsafe { a.array[i32](1)[0] - b.array[i32](1)[0] } })",
				"}",
				"print(buf)",
			],
		)
		.file(
			"cext.oi",
			[
				"module cext",
				"pub qsort : fn(base: ptr, n: usize, size: usize, cmp: fn(a: ptr, b: ptr) i32) : foreign",
			],
		)
		.check("[1, 2, 3]");
}

#[test]
fn foreign_callback_rejects_a_closure() {
	Project::new()
		.file(
			"main.oi",
			[
				"use cext",
				"k := 1",
				"bad := fn(s: i32) () { print(k) }",
				"unsafe cext.signal(2, bad)",
			],
		)
		.file(
			"cext.oi",
			[
				"module cext",
				"pub signal : fn(sig: i32, handler: fn(s: i32)) ptr : foreign",
			],
		)
		.fail_with("`@c` fns can't capture");
}

#[test]
fn raw_memory_needs_unsafe() {
	fail_with(
		["buf: []u8 = .[1]", "buf.ptr.array[u8](1)"],
		"`ptr.array` needs `unsafe`",
	);
	fail_with(
		["abs : fn(x: i32) i32 : foreign", "print(abs(-5))"],
		"`abs` needs `unsafe`",
	);
	fail_with(["@unsafe peek :: fn() {}", "peek()"], "`peek` needs `unsafe`");
}

#[test]
fn unsafe_fn_value_needs_unsafe() {
	fail_with(["@unsafe peek :: fn() {}", "f := peek", "f()"], "`peek` needs `unsafe`");
	check(
		["@unsafe peek :: fn() int { 1 }", "f := unsafe peek", "print(f())"],
		"1",
	);
}

#[test]
fn fn_ptr_cast_needs_a_c_signature() {
	fail_with(
		["Bad :: fn(s: string) int", "f := unsafe Bad(ptr(0))"],
		"can't cross the C ABI",
	);
}

#[test]
fn foreign_unknown_symbol_fails() {
	Project::new()
		.file("main.oi", ["use cext"])
		.file(
			"cext.oi",
			["module cext", "zzz_definitely_not_a_symbol : fn() int : foreign"],
		)
		.fail_with("unknown foreign symbol");
}

#[test]
fn link_dlopens_named_library() {
	Project::new()
		.file(
			"main.oi",
			["use cext", "print(unsafe { cext.zlibVersion().str() }[0..1])"],
		)
		.file(
			"cext.oi",
			["module cext", r#"@link.{"z"}"#, "pub zlibVersion : fn() cstr : foreign"],
		)
		.check("1");
}

#[test]
fn link_renames_the_symbol() {
	Project::new()
		.file("main.oi", ["use cext", "print(unsafe cext.magnitude(-5))"])
		.file(
			"cext.oi",
			[
				"module cext",
				r#"@link.{name = "abs"}"#,
				"pub magnitude : fn(x: i32) i32 : foreign",
			],
		)
		.check("5");
}

#[test]
fn c_struct_writes_at_c_offsets() {
	let src = indoc! {"
		@c
		S :: struct { a: u8, ok: bool, b: u32 }
		buf: []u8 = .[9, 9, 9, 9, 9, 9, 9, 9]
		unsafe { buf.ptr.write(S.{ a = 1, ok = true, b = 2 }) }
		print(buf)
	"};
	check(src, "[1, 1, 9, 9, 2, 0, 0, 0]");
}

#[test]
fn c_struct_roundtrips_nested() {
	let src = indoc! {"
		@c Inner :: struct { x: u16, y: u16 }
		@c Outer :: struct { tag: u8, on: bool, inner: Inner }
		buf: []u8 = .[0, 0, 0, 0, 0, 0]
		unsafe { buf.ptr.write(Outer.{ tag = 7, on = true, inner = Inner.{ x = 1, y = 2 } }) }
		print(buf)
		o := unsafe { buf.ptr.read[Outer]() }
		print(o.on)
		print(o.inner.y)
	"};
	check(src, ["[7, 1, 1, 0, 2, 0]", "true", "2"]);
}

#[test]
fn c_struct_roundtrips_fixed_array() {
	let src = indoc! {"
		@c Xform :: struct { m: [4]f32, id: i32 }
		buf: []u8 = .[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
		unsafe { buf.ptr.write(Xform.{ m = .[1.5, 2.5, 3.5, 4.5], id = 7 }) }
		print(unsafe { buf.ptr.offset(16).array[i32](1) })
		x := unsafe { buf.ptr.read[Xform]() }
		print(x.m)
		print(Xform.size)
		print(comp Xform.size)
		sized: [Xform.size]u8
		print(sized.len)
	"};
	check(src, ["[7]", "[1.5, 2.5, 3.5, 4.5]", "20", "20", "20"]);
}

#[test]
fn c_struct_rejects_missing_c_repr() {
	fail_with(["@c Bad :: struct { s: string }"], "`Bad.s` has no C representation");
	// 1 vs. 8 bytes
	fail_with(
		["@c Wide :: struct { flags: [4]bool }"],
		"`Wide.flags` has no C representation",
	);
	let src = indoc! {r#"
		buf: []u8 = .[0]
		unsafe { buf.ptr.write("hi") }
	"#};
	fail_with(src, "`string` has no C layout");
}

#[test]
fn c_struct_roundtrips_fn_field() {
	// TODO: revisit buf with a repeat fn or something sane
	let src = indoc! {"
		@c Init :: struct { level: i32, cb: fn(n: i32) i32 }
		double :: fn(n: i32) i32 { n * 2 }
		buf: []u8 = .[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
		unsafe { buf.ptr.write(Init.{ level = 1, cb = double }) }
		i := unsafe { buf.ptr.read[Init]() }
		f := i.cb
		print(f(21))
	"};
	check(src, "42");
}

#[test]
fn c_struct_fn_field_rejects_a_closure() {
	let src = indoc! {"
		@c Init :: struct { cb: fn(n: i32) i32 }
		k := 2
		Init.{ cb = fn [move k] (n: i32) i32 { n * k } }
	"};
	fail_with(src, "`@c` fns can't capture");
}

#[test]
fn c_struct_rejects_bad_fn_field() {
	fail_with(["@c Bad :: struct { cb: fn(s: string) }"], "has no C representation");
}

#[test]
fn qualified_type_in_signature() {
	Project::new()
		.file(
			"main.oi",
			[
				"use shapes",
				"f :: fn(p: shapes.Point) int { p.x + p.y }",
				"print(f(shapes.make(1, 2)))",
			],
		)
		.file(
			"shapes.oi",
			[
				"module shapes",
				"pub Point :: struct { pub x: int, pub y: int }",
				"pub make :: fn(x: int, y: int) Point { Point.{ x = x, y = y } }",
			],
		)
		.check("3");
}

#[test]
fn qualified_static_access() {
	Project::new()
		.file(
			"main.oi",
			indoc! {"
				use shapes
				p := shapes.Point.of(1, 2)
				print(p.x + p.y)
				print(shapes.Kind.circle.str())
				print(shapes.Point.ORIGIN.x)
			"},
		)
		.file(
			"shapes.oi",
			indoc! {"
				module shapes
				pub Point :: struct { pub x: int, pub y: int }
				Point :< {
					pub of :: fn(x: int, y: int) Point { Point.{ x = x, y = y } }
					pub ORIGIN :: Point.{ x = 0, y = 0 }
				}
				pub Kind :: enum { circle, square }
			"},
		)
		.check("3\ncircle\n0");
}

#[test]
fn const_exprs() {
	Project::new()
		.file(
			"main.oi",
			["module main", "use util", "print(util.half)", "print(util.low)"],
		)
		.file(
			"util/lib.oi",
			["module util", "pub half :: 10 / 2", "pub low :: -2147483647 - 1"],
		)
		.check(["5", "-2147483648"]);
}

#[test]
fn imported_alias_body_resolves_in_its_module() {
	Project::new()
		.file("main.oi", ["use m", "print(m.get())"])
		.file(
			"m.oi",
			[
				"module m",
				"B :: bool",
				"pub J :: :null | B",
				"pub get :: fn() J { true }",
			],
		)
		.check("true");
}
