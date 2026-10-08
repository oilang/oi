use crate::helpers::*;

#[test]
fn round_trips() {
	let src = indoc! {r#"
		Opt[T] :: enum { nope, some(T) }
		get :: fn() Opt[int] { .some.(5) }
		none :: fn() Opt[int] { .nope }
		wrap[T] :: fn(v: T) Opt[T] { .some.(v) }
		gets :: fn() Opt[string] { .some.("hi") }
		val :: fn(o: Opt[int]) int {
			match o {
				.some.(n) => n,
				.nope => -1,
			}
		}
		print(val(get()), val(none()), val(wrap(9)))
		match gets() { .some.(s) => print(s), .nope => {} }
	"#};
	check(src, ["5 -1 9", "hi"]);
}

#[test]
fn infers_params_from_instance() {
	let src = indoc! {r#"
		Opt[T] :: enum { nope, some(T) }
		again[T] :: fn(o: Opt[T]) Opt[T] { o }
		i : Opt[int] = .some.(7)
		s : Opt[string] = .some.("hi")
		match again(i) { .some.(n) => print(n), .nope => {} }
		match again(s) { .some.(v) => print(v), .nope => {} }
	"#};
	check(src, ["7", "hi"]);

	let src = indoc! {"
		Either[L, R] :: enum { left(L), right(R) }
		swap[L, R] :: fn(e: Either[L, R]) Either[R, L] {
			match e {
				.left.(v) => .right.(v),
				.right.(v) => .left.(v),
			}
		}
		e : Either[int, string] = .left.(1)
		match swap(e) { .left.(a) => print(a), .right.(b) => print(b) }
	"};
	check(src, "1");
}

#[test]
fn rejections() {
	fail(
		["Opt[T] :: enum { nope, some(T) }", "f :: fn(o: Opt) int { 0 }", "0"],
		"needs type arguments",
	);
	fail(
		[
			"Opt[T] :: enum { nope, some(T) }",
			"f :: fn() Opt[int, string] { .nope }",
			"0",
		],
		"expects 1 type argument(s), got 2",
	);
	fail(
		["Pair[A, B] :: struct { a: A, b: B }", "print(Pair[int, string].a)"],
		"is a type, not a value",
	);
}

#[test]
fn recursive_payload() {
	let src = indoc! {"
		Tree[T] :: enum { leaf(T), node(Tree[T]) }
		f :: fn() Tree[int] { .node.(.leaf.(5)) }
		match f() {
			.leaf.(v) => v,
			.node.(inner) => match inner { .leaf.(v) => v, .node.(x) => -1, },
		}
	"};
	check(src, "5");
}

#[test]
fn qualified_variant_path() {
	let src = indoc! {r#"
		Opt[T] :: enum { nope, yep(T) }
		print(Opt[int].nope)
		print(Opt[int].yep(3))
		print(Result[int, string].err("nope"))
		xs := [10, 20, 30]
		i := 1
		print(xs[i])
	"#};
	check(src, ["nope", "yep.(3)", r#"err.("nope")"#, "20"]);
}

#[test]
fn bare_path_infers() {
	let src = indoc! {r#"
		Opt[T] :: enum { nope, yep(T) }
		print(Opt.yep("hi"))
		w: ?int = Option.some(9)
		print(w?)
		x: ?int = Option.none
		y: !int = Result.ok(7)
		z: string!int = Result.err("no")
		o: Opt[int] = Opt.nope
		print(x, y, z, o)
	"#};
	check(src, [r#"yep.("hi")"#, "9", r#"none ok.(7) err.("no") nope"#]);
}

#[test]
fn a_variant_path_on_an_enum_bound_param() {
	let src = indoc! {r#"
		Sig :: enum { idle, busy(int) }
		start[T] :: fn() T { T.idle }
		load[T] :: fn(n: int) T { T.busy(n) }
		print(start[Sig]())
		print(load[Sig](3))
	"#};
	check(src, ["idle", "busy.(3)"]);
}

#[test]
fn instance_in_a_struct_field() {
	let src = indoc! {"
		Opt[T] :: enum { nope, yep(T) }
		Slot :: struct { v: Opt[int] }
		s :: Slot.{ v = Opt[int].yep(4) }
		match s.v { .nope => print(0), .yep.(n) => print(n), }
	"};
	check(src, "4");
}
