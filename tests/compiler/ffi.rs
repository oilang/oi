use indoc::indoc;

use crate::helpers::{check, fail};

#[test]
fn scalars_cross_ptr() {
	let src = indoc! {r#"
		buf: []u8 = .[0, 0, 0, 0, 0, 0, 0, 0]
		unsafe buf.ptr.write(42)
		print(unsafe buf.ptr.read[i32]())
		unsafe buf.ptr.write(buf.ptr)
		print(unsafe buf.ptr.read[ptr]().is_null())
		unsafe buf.ptr.write(true)
		print("{unsafe buf.ptr.read[bool]()}")
	"#};
	check(src, ["42", "false", "true"]);
}

#[test]
fn fn_casts_to_ptr() {
	let src = indoc! {"
		cb :: fn(n: i32) i32 { n + 1 }
		Cb :: @c fn(n: i32) i32
		f := unsafe Cb.(ptr.(cb))
		print(f(41), ptr.(0).is_null())
	"};
	check(src, "42 true");
}

#[test]
fn oi_fn_crosses_into_c_through_a_shim() {
	let src = indoc! {"
		@c V :: struct { x: f32, y: f32 }
		@export
		apply :: fn(f: @c fn(v: V) V) f32 {
			f(.{ x = 1, y = 2 }).x
		}
		flip :: fn(v: V) V {
			.{ x = v.y, y = v.x }
		}
		print(apply(flip), apply(fn(v: V) V { v }), ptr.(flip) == ptr.(flip))
		C :: @c fn(v: V) V
		lit := unsafe C.(
			ptr.(fn(v: V) V {
				.{ x = v.x + v.y, y = 0 }
			})
		)
		print(lit(.{ x = 1, y = 2 }).x)
	"};
	check(src, ["2.0 1.0 true", "3.0"]);
	fail(["f :: fn() {}", "g := f", "ptr.(g)"], "a `fn` value has no C address");
}

#[test]
fn c_fn_ptr_takes_a_fn_arg() {
	let src = indoc! {"
		@export
		run :: fn(cb: fn(get: fn() i32) i32) i32 {
			cb(fn() i32 do 7)
		}
		six :: fn() i32 { 6 }
		G :: @c fn(get: fn() i32) i32
		g := unsafe G.(ptr.(fn(get: fn() i32) i32 do get() * 7))
		print(run(fn(get: fn() i32) i32 do get()), g(six))
	"};
	check(src, "7 42");
}

#[test]
fn struct_place_behind_ptr() {
	let src = indoc! {"
		S :: struct { n: int }
		S :< { bump :: fn(mut self) { self.n = self.n + 1 } }
		buf: []u8 = .[0, 0, 0, 0, 0, 0, 0, 0]
		unsafe buf.ptr.write(S.{n = 1})
		unsafe S.(buf.ptr).bump()
		print(unsafe buf.ptr.read[S]().n)
	"};
	check(src, "2");
}

#[test]
fn unowned_ref_does_not_become_owned() {
	let src = indoc! {"
		buf: []int = .[7, 0]
		r := unsafe ^int.(buf.ptr)
		%
	"};
	for (tail, at) in [
		("f :: fn(p: ^int) int { p^ }\nprint(f(r))", "owned"),
		("q: ^int = r", "owned"),
		(
			"g :: fn(b: []int) ^int { unsafe ^int.(b.ptr) }\nprint(g(buf)^)",
			"owned",
		),
	] {
		fail(&src.replace('%', tail), at);
	}
}
