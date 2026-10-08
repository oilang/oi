use crate::helpers::*;
use indoc::indoc;

#[test]
fn basics() {
	check(
		indoc! {"
			o: ?int
			Box :: struct { val: ?int }
			print(?int.(42), ?int.(none), o, Box.{ val = ?int.(42) }.val, ord(?int.(42)), ord(?int.(none)))
			print(?int.(42) == ?int.(42), ?int.(42) == ?int.(7), ?int.(none) == ?int.(42), ?int.(none) != ?int.(42))
		"},
		["some.(42) none none some.(42) 1 0", "true false false true"],
	);
}

#[test]
fn rejections() {
	fail("none", "cannot infer the type");
	fail("int.(?int.(42))", "no backing value");
	fail("?int.(3.0)", "cannot cast float to ?int");
	fail("?int.(1) < ?int.(2)", "only `==` and `!=`");
}

#[test]
fn match_arms() {
	let src = indoc! {"
		unwrap_or :: fn(o: ?int, fallback: int) int {
			match o {
				.some.(n) => n,
				.none => fallback,
			}
		}
		print(unwrap_or(?int.(42), 0), unwrap_or(?int.(none), -1))
	"};
	check(src, "42 -1");
}

#[test]
fn match_binder_narrows_some() {
	let src = indoc! {"
		Node :: struct { val: int }
		f :: fn(o: ?Node) int {
			match o {
				n @ Node => n.val,
				_ => -1,
			}
		}
		print(f(?Node.(Node.{ val = 7 })))
		print(f(?Node.(none)))
	"};
	check(src, ["7", "-1"]);
}

#[test]
fn match_non_exhaustive_errors() {
	fail(
		indoc! {r"
			o :: ?int.(42)
			match o {
				.some.(n) => n,
			}
		"},
		"non-exhaustive match, missing: none",
	);
}

#[test]
fn bare_returns_wrap() {
	let src = indoc! {"
		find :: fn(x: int) ?int {
			if x > 0 { return x }
			return none
		}
		print(find(5), find(0))
	"};
	check(src, "some.(5) none");
}

#[test]
fn long_form_matches_shorthand() {
	let src = indoc! {"
		find :: fn(id: int) Option[int] {
			if id == 7 { return 42 }
			return none
		}
		print(find(7) or { -1 }, find(1) or { -1 })
	"};
	check(src, "42 -1");
}

#[test]
fn generic_fn_infers_through_option() {
	let src = indoc! {r#"
		unwrap[T] :: fn(o: Option[T], fallback: T) T {
			match o {
				.some.(v) => v,
				.none => fallback,
			}
		}
		print(unwrap(?int.(5), 0), unwrap(?string.(none), "hi"))
	"#};
	check(src, "5 hi");
}

#[test]
fn array_payload_is_independent_copy() {
	let src = indoc! {"
		wrap :: fn(a: []int) ?[]int { return a }
		a := [1]
		o :: wrap(a)
		a << 2
		match o { .some.(v) => v, .none => [0] }
	"};
	check(src, "[1]");
}

#[test]
fn values_coerce_into_options() {
	let src = indoc! {r#"
		T :: enum { nil, int, float }
		S :: struct { ret: ?T, d: ?int }
		mk :: fn(ret: ?T, d: ?int = none) S { S.{ ret, d } }
		pick :: fn(b: bool) ?T { if b { .int } else { none } }
		a: ?int = 3
		b: ?T = .int
		s := S.{ .float, 4 }
		print("{a or 0} {b or .nil} {s.ret or .nil} {s.d or 0}")
		print("{mk(.int).ret or .nil} {mk(T.int, a).d or 0} {mk(none).ret == none}")
		print("{pick(true) or .nil} {pick(false) or .nil}")
	"#};
	check(src, ["3 int float 4", "int 3 true", "int nil"]);
}

#[test]
fn zeroed_fn_options_are_none() {
	let src = indoc! {"
		S :: struct { f: ?fn(n: int) int, g: ?@c fn(n: i32) i32 }
		buf: [16]u8
		s := unsafe S.(buf.ptr)
		inc :: fn(n: int) int { n + 1 }
		print(s.f, s.g, match S.{f = inc}.f { .some.(p) => p(41), .none => 0 })
	"};
	check(src, "none none 42");
}

#[test]
fn anon_none_infers_for_fn_options() {
	let src = indoc! {"
		S :: struct { f: ?fn(n: int) int = .none }
		g: ?fn(n: int) int = .none
		print(S.{}.f == none, g == none)
	"};
	check(src, "true true");
}
