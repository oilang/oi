use indoc::indoc;

use crate::helpers::{check, fail};

#[test]
fn scalars_cross_ptr() {
	let src = indoc! {"
		buf: []u8 = .[0, 0, 0, 0, 0, 0, 0, 0]
		unsafe buf.ptr.write(42)
		print(unsafe buf.ptr.read[i32]())
		unsafe buf.ptr.write(buf.ptr)
		print(unsafe buf.ptr.read[ptr]().is_null())
	"};
	check(src, ["42", "false"]);
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
