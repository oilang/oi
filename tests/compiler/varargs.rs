use crate::helpers::*;

#[test]
fn vararg_rejections() {
	for src in [
		"f :: fn(x: ?..int) {}",
		"f :: fn(x: []..int) {}",
		"S :: struct { xs: ..int }",
		"f :: fn() ..int { [] }",
	] {
		fail(src, "`..T` is only allowed as a parameter type");
	}
	fail("f :: fn(a: ..int, b: ..int) {}", "`f` has more than one vararg");
	fail(
		["f :: fn(a: ..int, b: int) {}", "f()"],
		"`f` expects 1.. argument(s), got 0",
	);
	fail(["f :: fn(xs: ..int) {}", "a :: [1]", "f(a)"], "expected int, got []int");
}

#[test]
fn varargs() {
	let src = indoc! {r#"
		sum :: fn(xs: ..int) int {
			t := 0
			loop x in xs { t = t + x }
			t
		}
		between :: fn(open: string, items: ..string, close: string) string { "{open}{items.len}{close}" }
		join :: fn(parts: ..string, sep := ",") string { "{parts.len}{sep}" }
		count :: fn(args: ..any) int { args.len }
		s :: sum
		ps :: ["a", "b"]
		print(sum(1, 2, 3), sum(), s(4, 5), count(1, "a", true))
		print(between("[", "a", "b", "]"), between("[", "]"), join("a", "b", sep = "/"))
		print(join(..ps), join("pre", ..ps, "post"), sum(..[1, 2]))
	"#};
	check(src, ["6 0 9 3", "[2] [0] 2/", "2, 4, 3"]);
}

#[test]
fn generic_varargs() {
	let src = indoc! {r#"
		count[T] :: fn(xs: ..T) int { xs.len }
		first[T] :: fn(xs: ..T) T { xs[0] }
		print(count(1, 2, 3), count("a", "b"))
		print(first(1, 2, 3), first("a", "b"))
	"#};
	check(src, ["3 2", "1 a"]);
	fail(
		["first[T] :: fn(xs: ..T) T { xs[0] }", r#"first(1, "a")"#],
		"array elements must share a type",
	);
}

#[test]
fn spread_args() {
	let src = indoc! {r#"
		divmod :: fn(a: int, b: int) (int, int) { (a / b, a % b) }
		show :: fn(q: int, r: int) { print(q, r) }
		sum :: fn(xs: ..int) int {
			t := 0
			loop x in xs { t = t + x }
			t
		}
		fwd :: fn(a: int, b: int) { show(..$) }
		pair: [2]int = .[4 5]
		show(..divmod(10, 3))
		fwd(7, 2)
		show(..pair)
		print(sum(9, ..(1, 2)))
	"#};
	check(src, ["3 1", "7 2", "4 5", "12"]);
	let show = "show :: fn(q: int, r: int) { print(q, r) }";
	fail([show, "show(..[1, 2])"], "a `[]T` spread may only feed the vararg slot");
	fail([show, r#"show(.."ab")"#], "cannot spread string");
}
