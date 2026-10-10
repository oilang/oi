use crate::helpers::*;

#[test]
fn unwraps_or_falls_back() {
	let src = indoc! {r#"
		print(?int.(none) or { -1 })
		print(!int.(error("oops")) or { -1 })
		print(?int.(42) or { -1 })
		x :: !int.(42) or {
			print("ran")
			9
		}
		x
	"#};
	check(src, ["-1", "-1", "42", "42"]);
}

#[test]
fn dollar_is_error_message() {
	let src = indoc! {r#"
		!int.(error("boom")) or {
			print($)
			9
		}
	"#};
	check(src, ["boom", "9"]);
}

#[test]
fn fallback_can_diverge() {
	let src = indoc! {"
		unwrap_or_bail :: fn(o: ?int) int {
			v :: o or { return -1 }
			v
		}
		f :: fn(o: ?int) int {
			y := o or return 0
			z := match o { .none => return 0, .some.(v) => v }
			y + z
		}
		print(unwrap_or_bail(?int.(none)))
		print(unwrap_or_bail(?int.(42)))
		f(?int.(none)) + f(?int.(2))
	"};
	check(src, ["-1", "42", "4"]);
}

#[test]
fn diverging_fallback_ends_at_semicolon() {
	let src = indoc! {"
		f :: fn(o: ?int) {
			loop { y := o or break; print(y); break }
			y := o or return; print(y)
		}
		f(?int.(1))
		f(?int.(none))
	"};
	check(src, ["1", "1"]);
}

#[test]
fn fallback_can_panic() {
	check(r#"!string.("hi") or { panic!("boom") }"#, "hi");
	fail_rt(r#"!string.(error("boom")) or { panic!("boom") }"#, "panic: boom");
	fail_rt(r#"!string.(error("boom")) or panic!("boom")"#, "panic: boom");
}

#[test]
fn type_errors() {
	fail(
		r#"?int.(42) or { "wrong" }"#,
		"or` branches have mismatched types: int and str",
	);
	fail("42 or { 0 }", "needs a `?T`/`!T` value");
}

#[test]
fn and_maps_the_happy_path() {
	let src = indoc! {r#"
		half :: fn(n: int) ?int { if n % 2 == 0 { return n / 2 } none }
		print(?int.(4) and $ * 10 or -1)
		print(?int.(4) and half($) and half($) or -1)
		print(?int.(6) and half($) and half($) or -1)
		print(!int.(error("boom")) and "{$}" or { $.message() })
	"#};
	check(src, ["40", "1", "-1", "boom"]);

	let src = indoc! {"
		NetError :: enum { timeout refused }
		fetch :: fn() Result[int, NetError] { 1 }
		load :: fn() !int { fetch() and !int.($) }
		load()
	"};
	fail(src, "does not claim Error");
	fail("?int.(1) and !int.($)", "cannot mix `?T` and `!T`");
}

#[test]
fn pipeline_is_a_try_scope() {
	let src = indoc! {r#"
		E1 :: enum { a }
		E1 :< Error
		f :: fn(x: int) E1!int { if x > 0 do return E1.a; x }
		h :: fn(x: int) !int { if x > 2 do return error("h"); x }
		print(5 |> h? |> f? or { print($.message()); 0 })
		print(1 |> h? |> f? or { print($.message()); 0 })
	"#};
	check(src, ["h", "0", "a", "0"]);

	let src = indoc! {"
		o :: fn(x: int) ?int { x }
		h :: fn(x: int) !int { x }
		0 |> o? |> h? or -1
	"};
	fail(src, "cannot catch both `?T` and `!T`");
}
