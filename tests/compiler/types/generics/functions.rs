use crate::helpers::*;
use indoc::indoc;

#[test]
fn max_int() {
	let src = indoc! {"
		max[T] :: fn(a: T, b: T) T {
			if a > b { a } else { b }
		}
		max(3, 7)
	"};
	check(src, "7");
}

#[test]
fn max_float_instantiation_is_independent() {
	let src = indoc! {"
		max[T] :: fn(a: T, b: T) T {
			if a > b { a } else { b }
		}
		a := max(3, 7)
		max(3.5, 1.2)
	"};
	check(src, "3.5");
}

#[test]
fn self_recursive_generic() {
	let src = indoc! {"
		fact[T] :: fn(n: T) T {
			if n <= 1 { 1 } else { n * fact(n - 1) }
		}
		fact(5)
	"};
	check(src, "120");
}

#[test]
fn mutually_recursive_generics() {
	let src = indoc! {"
		is_even[T] :: fn(n: T) bool {
			if n == 0 { true } else { is_odd(n - 1) }
		}
		is_odd[T] :: fn(n: T) bool {
			if n == 0 { false } else { is_even(n - 1) }
		}
		is_even(10)
	"};
	check(src, "true");
}

#[test]
fn first_of_array() {
	let src = indoc! {"
		first[T] :: fn(xs: []T) ?T {
			if xs.len == 0 { ?T(none) } else { ?T(xs[0]) }
		}
		first([1, 2, 3])
	"};
	check(src, "some(1)");
}

#[test]
fn type_mismatch_across_args() {
	fail_with(
		indoc! {r#"
			max[T] :: fn(a: T, b: T) T { if a > b { a } else { b } }
			max(1, "a")
		"#},
		"bound to both",
	);
}

#[test]
fn omitted_return_type_is_unit() {
	let src = indoc! {"
		show[T] :: fn(x: T) { print(x) }
		show(1)
		show(2.5)
	"};
	check(src, ["1", "2.5"]);
}

#[test]
fn tuple_substitutions_monomorphize_separately() {
	let src = indoc! {r#"
		show[T] :: fn(x: T) { print("{x}") }
		show((1, "a"))
		show((true, false))
	"#};
	check(src, [r#"(1, "a")"#, "(true, false)"]);
}

#[test]
fn omitted_return_type_rejects_a_value() {
	fail_with(
		indoc! {"
			noret[T] :: fn(x: T) { x }
			noret(1)
		"},
		"expected ()",
	);
}

#[test]
fn explicit_type_arg_when_uninferable() {
	let src = indoc! {"
		none_of[T] :: fn() ?T {
			?T(none)
		}
		none_of[int]()
	"};
	check(src, "none");
}

#[test]
fn explicit_type_arg_redundant_with_inference() {
	let src = indoc! {"
		max[T] :: fn(a: T, b: T) T {
			if a > b { a } else { b }
		}
		max[int](3, 7)
	"};
	check(src, "7");
}

#[test]
fn explicit_type_arg_count_mismatch() {
	fail_with(
		indoc! {"
			max[T] :: fn(a: T, b: T) T { if a > b { a } else { b } }
			max[int, string](3, 7)
		"},
		"expects 1 type argument",
	);
}

#[test]
fn explicit_type_arg_on_non_generic_errors() {
	fail_with(
		indoc! {"
			add :: fn(a: int, b: int) int { a + b }
			add[int](3, 7)
		"},
		"is not generic",
	);
}

#[test]
fn bounded_type_param_parses_and_runs() {
	let src = indoc! {"
		Ord :: trait {}
		int :< Ord
		biggest[T: Ord] :: fn(a: T, b: T) T {
			if a > b { a } else { b }
		}
		biggest(3, 7)
	"};
	check(src, "7");
}

#[test]
fn std_bound_satisfied_by_builtin() {
	let src = indoc! {"
		biggest[T: Ord] :: fn(a: T, b: T) T {
			if a > b { a } else { b }
		}
		biggest(3, 7)
	"};
	check(src, "7");
}

#[test]
fn bound_violated() {
	fail_with(
		indoc! {"
			Ord :: trait {}
			biggest[T: Ord] :: fn(a: T, b: T) T {
				if a > b { a } else { b }
			}
			biggest(3, 7)
		"},
		"does not claim",
	);
}

#[test]
fn unknown_bound_trait() {
	fail_with(
		indoc! {"
			biggest[T: Odr] :: fn(a: T, b: T) T {
				if a > b { a } else { b }
			}
			biggest(3, 7)
		"},
		"unknown trait",
	);
}

#[test]
fn default_type_param_fills_when_uninferable() {
	let src = indoc! {"
		none_of[T = int] :: fn() ?T {
			?T(none)
		}
		none_of()
	"};
	check(src, "none");
}

#[test]
fn inference_and_explicit_args_beat_the_default() {
	let src = indoc! {r#"
		id[T = int] :: fn(x: T) T { x }
		none_of[T = int] :: fn() ?T { ?T(none) }
		print(id("a"))
		print(none_of[string]())
	"#};
	check(src, ["a", "none"]);
}
