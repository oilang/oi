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
	check(src, "1\nhi");
}

#[test]
fn bare_name_needs_type_arguments() {
	fail_with(
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
	fail_with(
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
