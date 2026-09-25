use crate::helpers::*;
use indoc::indoc;

#[test]
fn array_literal() {
	check("[1, 2, 3]", "[1, 2, 3]");
}

#[test]
fn fn_param_type() {
	let src = indoc! {"
		first :: fn(xs: []int) int {
			xs.0
		}
		first([9, 8, 7])
	"};
	check(src, "9");
}

#[test]
fn array_of_strings() {
	check(r#"["a", "b"]"#, r#"["a", "b"]"#);
}

#[test]
fn trailing_comma() {
	check("[1, 2,]", "[1, 2]");
}

#[test]
fn no_comma_ints() {
	check("[2 4 6]", "[2, 4, 6]");
}

#[test]
fn index_literal() {
	check(["a :: [10, 20, 30]", "a[1]"], "20");
}

#[test]
fn index_variable() {
	check(["a :: [10, 20, 30]", "i :: 2", "a[i]"], "30");
}

#[test]
fn dot_index() {
	check(["a :: [10, 20, 30]", "a.0"], "10");
}

#[test]
fn len_field() {
	check(["a :: [10, 20, 30]", "a.len"], "3");
}

#[test]
fn nested_in_tuple() {
	check(r#"(1, [2, 3], "x")"#, r#"(1, [2, 3], "x")"#);
}

#[test]
fn array_of_tuples() {
	check("[(1, 2), (3, 4)]", "[(1, 2), (3, 4)]");
}

#[test]
fn nested_arrays() {
	check(["a :: [10, 20]", "b :: [30, 40]", "[a, b]"], "[[10, 20], [30, 40]]");
}

#[test]
fn index_into_nested() {
	check(["a :: [10, 20]", "b :: [30, 40]", "[a, b][1]"], "[30, 40]");
}

#[test]
fn mixed_types() {
	fail(r#"[1, "two"]"#, "must share a type");
}

#[test]
fn spread() {
	let src = indoc! {"
		a :: [2, 3]
		b :: [5, 6]
		print([..a])
		[1, ..a, ..b, 7]
	"};
	check(src, ["[2, 3]", "[1, 2, 3, 5, 6, 7]"]);
}

#[test]
fn spread_type_mismatch() {
	fail([r#"a :: ["x", "y"]"#, "[1, ..a]"], "must share a type");
}

#[test]
fn empty_unsupported() {
	fail("[]", "empty array");
}

#[test]
fn index_non_array() {
	fail(["x :: 5", "x[0]"], "cannot index");
}

#[test]
fn non_int_index() {
	fail(r#"a :: [1, 2]; a["x"]"#, "index must be Int");
}

#[test]
fn index_out_of_range() {
	fail_rt(["a :: [1, 2]", "a[5]"], "out of range");
}

#[test]
fn unknown_named_field() {
	fail(["a :: [1, 2]", "a.foo"], "no field `foo`");
}

// slices

#[test]
fn slice_middle() {
	check(["even :: [0, 2, 4, 6, 8]", "even[1..3]"], "[2, 4]");
}

#[test]
fn slice_from_start() {
	check(["even :: [0, 2, 4, 6, 8]", "even[..3]"], "[0, 2, 4]");
}

#[test]
fn slice_to_end() {
	check(["even :: [0, 2, 4, 6, 8]", "even[1..]"], "[2, 4, 6, 8]");
}

#[test]
fn slice_full() {
	check(["even :: [0, 2, 4, 6, 8]", "even[..]"], "[0, 2, 4, 6, 8]");
}

#[test]
fn slice_empty() {
	check(["even :: [0, 2, 4, 6, 8]", "even[2..2]"], "[]");
}

#[test]
fn slice_variable_bounds() {
	check(["a :: [0, 2, 4, 6, 8]", "lo :: 1", "hi :: 4", "a[lo..hi]"], "[2, 4, 6]");
}

#[test]
fn slice_is_an_array() {
	check(["a :: [0, 2, 4, 6, 8]", "assert!(a[1..][0] == 2)"], "");
}

#[test]
fn slice_out_of_bounds() {
	fail_rt(["a :: [1, 2, 3]", "a[1..9]"], "out of bounds");
}

#[test]
fn slice_reversed_range() {
	fail_rt(["a :: [1, 2, 3]", "a[3..1]"], "out of bounds");
}

#[test]
fn slice_non_array() {
	fail(["x :: 5", "x[0..1]"], "cannot slice");
}

#[test]
fn slice_non_int_bound() {
	fail(r#"a :: [1, 2, 3]; a[true..2]"#, "must be Int");
}

// index assignment

#[test]
fn index_assign_basic() {
	check(["a := [1, 2, 3]", "a[1] = 99", "a"], "[1, 99, 3]");
}

#[test]
fn index_assign_variable_index() {
	check(["a := [1, 2, 3]", "i := 2", "a[i] = 7", "a"], "[1, 2, 7]");
}

#[test]
fn index_assign_immutable_error() {
	fail(["a :: [1, 2]", "a[0] = 5"], "immutable");
}

#[test]
fn index_assign_non_array_error() {
	fail(["x := 5", "x[0] = 1"], "not an array");
}

#[test]
fn index_assign_type_mismatch_error() {
	fail(r#"a := [1, 2]; a[0] = "hi""#, "type mismatch");
}

#[test]
fn index_assign_oob_error() {
	fail_rt(["a := [1, 2]", "a[5] = 9"], "out of range");
}

// append

#[test]
fn append_basic() {
	check(["a := [1, 2, 3]", "a << 4", "a"], "[1, 2, 3, 4]");
}

#[test]
fn append_grows_past_initial_cap() {
	// initial cap == len == 2
	check(["a := [1, 2]", "a << 3", "a << 4", "a << 5", "a"], "[1, 2, 3, 4, 5]");
}

#[test]
fn append_slice_copies_buffer() {
	check(["a :: [1, 2, 3]", "b := a[1..]", "b << 99", "b"], "[2, 3, 99]");
}

#[test]
fn append_in_expression_position() {
	check(["a := [1]", "(a << 2 << 3).len"], "3");
}

#[test]
fn append_immutable_error() {
	fail(["a :: [1, 2]", "a << 3"], "immutable");
}

#[test]
fn append_non_array_error() {
	fail([r#"x := "hi""#, "x << 1"], "cannot apply `<<`");
}

#[test]
fn append_type_mismatch_error() {
	fail(r#"a := [1, 2]; a << "hi""#, "type mismatch");
}

// array extend (<<)

#[test]
fn extend_basic() {
	check(["odd := [1, 3, 5]", "odd << [9, 11]", "odd"], "[1, 3, 5, 9, 11]");
}

#[test]
fn extend_empty_src() {
	check(["a := [1, 2, 3]", "b :: a[0..0]", "a << b", "a"], "[1, 2, 3]");
}

#[test]
fn extend_into_empty_ish() {
	check(["a :: [1, 2]", "b := a[0..0]", "b << [3, 4]", "b"], "[3, 4]");
}

#[test]
fn extend_type_mismatch_error() {
	fail(r#"a := [1, 2]; b :: ["x"]; a << b"#, "type mismatch");
}

// value semantics (COW)

#[test]
fn copy_is_independent_on_append() {
	check(
		indoc! {"
		a := [1, 2, 3]
		b :: a
		a << 4
		print(a)
		b
	"},
		["[1, 2, 3, 4]", "[1, 2, 3]"],
	);
}

#[test]
fn appended_element_is_independent_copy() {
	check(
		indoc! {"
		inner := [1]
		outer := [[9]]
		outer << inner
		inner << 2
		outer[1]
	"},
		"[1]",
	);
}

#[test]
fn index_assign_copy_forward() {
	check(["a := [1, 2, 3]", "b :: a", "a[0] = 99", "b"], "[1, 2, 3]");
}

#[test]
fn index_assign_copy_backward() {
	check(["a :: [1, 2, 3]", "b := a", "b[0] = 99", "a"], "[1, 2, 3]");
}

#[test]
fn chain_of_copies_is_independent() {
	check(
		indoc! {"
		a := [1, 2, 3]
		b :: a
		c :: b
		a << 4
		print(b)
		c
	"},
		["[1, 2, 3]", "[1, 2, 3]"],
	);
}

#[test]
fn copy_of_zero_value_appends() {
	check(["a: []int", "b := a", "b << 1", "b"], "[1]");
}

#[test]
fn slice_independent_from_parent() {
	check(["a := [1, 2, 3]", "b :: a[..]", "a[0] = 99", "b"], "[1, 2, 3]");
}

#[test]
fn parent_independent_from_slice() {
	check(["a :: [1, 2, 3]", "b := a[..]", "b[0] = 99", "a"], "[1, 2, 3]");
}

#[test]
fn returned_param_independent_from_arg() {
	check(
		indoc! {"
		id :: fn(a: []int) []int { a }
		a :: [1, 2, 3]
		r := id(a)
		r << 4
		a
	"},
		"[1, 2, 3]",
	);
}

// in operator

#[test]
fn in_found() {
	check(["even :: [0, 2, 4, 6, 8]", "6 in even"], "true");
}

#[test]
fn in_not_found() {
	check(["even :: [0, 2, 4, 6, 8]", "5 in even"], "false");
}

#[test]
fn in_after_append() {
	check(["a := [1, 2]", "a << 3", "3 in a"], "true");
}

#[test]
fn in_non_array_error() {
	fail("5 in 10", "not an array");
}

#[test]
fn in_type_mismatch_error() {
	fail(r#"a :: [1, 2]; "x" in a"#, "type mismatch");
}

#[test]
fn fn_returns_array_annotation() {
	let src = indoc! {"
		nums :: fn() []int { [1, 2, 3] }
		nums()
	"};
	check(src, "[1, 2, 3]");
}

#[test]
fn fn_returns_array_field() {
	let src = indoc! {"
		nums :: fn() []int { [10, 20, 30] }
		a :: nums()
		a[1]
	"};
	check(src, "20");
}

#[test]
fn fn_return_type_mismatch_array() {
	let src = indoc! {"
		bad :: fn() []int { 42 }
		bad()
	"};
	fail(src, "wrong return type");
}

#[test]
fn if_no_else_array_zero() {
	let src = indoc! {"
		a :: if false { [1, 2, 3] }
		a.len
	"};
	check(src, "0");
}

// fixed-size arrays

#[test]
fn fixed_zeroed() {
	check(["a: [3]int", "a"], "[0, 0, 0]");
}

#[test]
fn fixed_len() {
	check(["a: [4]int", "a.len"], "4");
	check(["N :: 3", "a: [N]int", "a.len"], "3");
}

#[test]
fn fixed_index_write_read() {
	check(
		r#"three: [3]string
		three[0] = "larry"
		three[1] = "curly"
		three"#,
		r#"["larry", "curly", ""]"#,
	);
}

#[test]
fn fixed_dot_index() {
	check(["a: [2]int", "a[0] = 9", "a.0"], "9");
}

#[test]
fn fixed_value_semantics() {
	check(["a: [2]int", "b := a", "a[0] = 9", "b[0]"], "0");
}

#[test]
fn fixed_index_out_of_range() {
	fail_rt(["a: [2]int", "a[5]"], "out of range");
}

#[test]
fn fixed_field_survives_return() {
	check(
		indoc! {"
			T :: struct { w: [4]u64 }
			make :: fn() T { T.{ w = .[1 2 3 4] } }
			t := make()
			print(t.w[0])
			print(t.w[3])
		"},
		["1", "4"],
	);
}

#[test]
fn empty_dynamic_via_init() {
	check(["a: []int", "a.len"], "0");
}

#[test]
fn typed_literal() {
	check("int.[1, 2]", "[1, 2]");
}

#[test]
fn typed_literal_mismatch() {
	fail(r#"a :: int.[1, "x"]"#, "must share a type");
}

// anon array literals

#[test]
fn anon_against_fixed_target() {
	check(["a: [3]int = .[1 2 3]", "a"], "[1, 2, 3]");
}

#[test]
fn anon_fixed_count_mismatch() {
	fail("a: [3]int = .[1 2]", "expected 3 elements, got 2");
}

#[test]
fn anon_against_dynamic_target() {
	check(["a: []int = .[1 2]", "a"], "[1, 2]");
}

#[test]
fn anon_as_call_arg() {
	let src = indoc! {"
		three :: fn(xs: [3]int) int {
			xs.0
		}
		three(.[1 2 3])
	"};
	check(src, "1");
}

#[test]
fn anon_no_annotation_infers_fixed() {
	check(["primes := .[2 3 5 7]", "primes.len"], "4");
}

#[test]
fn anon_fixed_value_semantics() {
	check(["a: [2]int = .[1 2]", "b := a", "a[0] = 9", "b[0]"], "1");
}

#[test]
fn anon_empty_no_context_fails() {
	fail("a := .[]", "cannot infer the element type");
}

// fixed <-> dynamic

#[test]
fn typed_literal_is_fixed_value_semantics() {
	check(["a := int.[1, 2]", "b := a", "a[0] = 9", "b[0]"], "1");
}

#[test]
fn typed_literal_empty_fails() {
	fail("int.[]", "an exact array literal needs elements");
}

#[test]
fn array_type_literal_is_whole_type() {
	check(["a := [2]int.[3, 4]", "a"], "[3, 4]");
	check(["b := []int.[3, 4]", "b << 5", "b"], "[3, 4, 5]");
	check("[]int.[]", "[]");
}

#[test]
fn array_type_literal_count_mismatch() {
	fail("[2]int.[3]", "expected 2 elements, got 1");
}

#[test]
fn fixed_coerces_to_dynamic_binding() {
	check(["a := i32.[1, 2]", "b: []i32 = a", "b << 3", "b"], "[1, 2, 3]");
}

#[test]
fn fixed_coerce_leaves_original_unchanged() {
	check(["a := i32.[1, 2]", "b: []i32 = a", "b << 3", "a"], "[1, 2]");
}

#[test]
fn fixed_coerces_as_call_arg() {
	let src = indoc! {"
		total :: fn(xs: []int) int { xs.len }
		total(int.[1, 2, 3])
	"};
	check(src, "3");
}

#[test]
fn in_compares_structurally() {
	check(
		indoc! {"
			P :: struct { x: int }
			print(P.{1} in [P.{2}, P.{1}])
			print((1, 2) in [(3, 4)])
		"},
		["true", "false"],
	);
}

#[test]
fn equality_is_structural() {
	check(
		indoc! {"
			print([1, 2, 3] == [1, 2, 3])
			print([1, 2] == [1, 2, 3])
			print([[1, 2], [3]] == [[1, 2], [3]])
		"},
		["true", "false", "true"],
	);
}

#[test]
fn ordering_rejected() {
	fail("[1, 2] < [1, 3]", "only `==` and `!=`");
}
