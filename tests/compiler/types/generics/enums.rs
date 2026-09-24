use crate::helpers::*;

#[test]
fn shorthand_round_trip() {
	let src = indoc! {"
		Opt[T] :: enum { nope, some(T) }
		get :: fn() Opt[int] { .some.(5) }
		match get() {
			.some.(n) => n,
			.nope => -1,
		}
	"};
	check(src, "5");
}

#[test]
fn nope_arm() {
	let src = indoc! {"
		Opt[T] :: enum { nope, some(T) }
		get :: fn() Opt[int] { .nope }
		match get() {
			.some.(n) => n,
			.nope => -1,
		}
	"};
	check(src, "-1");
}

#[test]
fn generic_fn_round_trip() {
	let src = indoc! {"
		Opt[T] :: enum { nope, some(T) }
		wrap[T] :: fn(v: T) Opt[T] { .some.(v) }
		match wrap(9) {
			.some.(n) => n,
			.nope => -1,
		}
	"};
	check(src, "9");
}

#[test]
fn two_instances_coexist() {
	let src = indoc! {r#"
		Opt[T] :: enum { nope, some(T) }
		geti :: fn() Opt[int] { .some.(1) }
		gets :: fn() Opt[string] { .some.("hi") }
		match geti() { .some.(n) => print(n), .nope => {} }
		match gets() { .some.(s) => print(s), .nope => {} }
	"#};
	check(src, ["1", "hi"]);
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
fn bare_name_needs_type_arguments() {
	fail(
		indoc! {"
			Opt[T] :: enum { nope, some(T) }
			f :: fn(o: Opt) int { 0 }
			0
		"},
		"needs type arguments",
	);
}

#[test]
fn wrong_arity() {
	fail(
		indoc! {"
			Opt[T] :: enum { nope, some(T) }
			f :: fn() Opt[int, string] { .nope }
			0
		"},
		"expects 1 type argument(s), got 2",
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
fn bare_path_infers_from_the_payload() {
	let src = indoc! {r#"
		Opt[T] :: enum { nope, yep(T) }
		print(Opt.yep("hi"))
		x: ?int = Option.some(9)
		print(x?)
	"#};
	check(src, [r#"yep.("hi")"#, "9"]);
}

#[test]
fn a_generic_struct_head_is_not_a_variant_path() {
	fail(
		indoc! {"
			Pair[A, B] :: struct { a: A, b: B }
			print(Pair[int, string].a)
		"},
		"is a type, not a value",
	);
}
