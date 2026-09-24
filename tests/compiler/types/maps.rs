use crate::helpers::*;
use indoc::indoc;

#[test]
fn declare_and_set_get() {
	check(
		indoc! {r#"
			m: [string]int
			m["one"] = 1
			m["one"]
		"#},
		"1",
	);
}

#[test]
fn init_expr_declare_and_set_get() {
	check(
		indoc! {r#"
			m: [string]int = []
			m["one"] = 1
			m["one"]
		"#},
		"1",
	);
}

#[test]
fn generic_fn_type_param_as_map_key() {
	check(
		indoc! {r#"
			get[K] :: fn(m: [K]int, k: K) int { m[k] }
			m: [string]int
			m["a"] = 5
			get(m, "a")
		"#},
		"5",
	);
}

#[test]
fn dot_brace_map_syntax_is_gone() {
	fail("Map.{ one = 1 }", "unknown struct");
	fail(r#"Map[string, int].{"x"}"#, "unknown type `Map`");
}

#[test]
fn overwrite_key() {
	check(
		indoc! {r#"
			m: [string]int
			m["a"] = 1
			m["a"] = 2
			m["a"]
		"#},
		"2",
	);
}

#[test]
fn multiple_keys() {
	check(
		indoc! {r#"
			m: [string]int
			m["one"] = 1
			m["two"] = 2
			m["one"] + m["two"]
		"#},
		"3",
	);
}

#[test]
fn int_keys() {
	check(
		indoc! {r#"
			m: [int]string
			m[1] = "a"
			m[2] = "b"
			m[1]
		"#},
		"a",
	);
}

#[test]
fn len_keys_values() {
	check(
		indoc! {r#"
			m: [string]int
			m["a"] = 1
			m["b"] = 2
			m["c"] = 3
			(m.len, m.keys.len, m.values.len, m.keys.contains("b"), m.values.contains(2))
		"#},
		"(3, 3, 3, true, true)",
	);
}

#[test]
fn tuple_keys_fail_for_now() {
	// TODO: actually implement complex keys and fix test
	assert!(
		fail(indoc! {"
			Point :: (int, int)
			m: [Point]int
			m[(1, 2)] = 6
			m[(2, 1)] = 9
			m[(2, 1)]
		"}, "")
		.contains("tuple cannot be used as a map key")
	);
}

#[test]
fn missing_key_panics() {
	assert!(
		fail_rt(indoc! {r#"
			m: [string]int
			m["missing"]
		"#}, "")
		.contains("key not found")
	);
}

#[test]
fn wrong_key_type() {
	assert!(
		fail(indoc! {r#"
			m: [string]int
			m[1]
		"#}, "")
		.contains("expected string key")
	);
}

#[test]
fn wrong_value_type() {
	assert!(
		fail(indoc! {r#"
			m: [string]int
			m["a"] = "b"
		"#}, "")
		.contains("type mismatch")
	);
}

#[test]
fn bracket_lit_string_keys() {
	check(
		indoc! {r#"
			m :: ["one" = 1, "two" = 2]
			m["one"] + m["two"]
		"#},
		"3",
	);
}

#[test]
fn bracket_lit_multiline() {
	check(
		indoc! {r#"
			m :: [
				"one" = 1
				"two" = 2
			]
			m["two"]
		"#},
		"2",
	);
}

#[test]
fn bracket_lit_typed_target() {
	check(
		indoc! {r#"
			m: [string]f64 : ["a" = 1.5]
			m["a"]
		"#},
		"1.5",
	);
}

#[test]
fn empty_array_against_array_target() {
	check(["a: []int = []", "a.len"], "0");
}

#[test]
fn bracket_lit_infers_from_first_entry() {
	check(r#"[1 = "one", 2 = "two"][1]"#, "one");
}

#[test]
fn bracket_lit_atom_keys() {
	check("[:ok = 200, :not_found = 404][:ok]", "200");
}

#[test]
fn bracket_lit_var_key_uses_value_not_name() {
	check(
		indoc! {r#"
			k :: "one"
			m := [k = 1, "two" = 2]
			m["one"]
		"#},
		"1",
	);
	assert!(
		fail_rt(indoc! {r#"
			k :: "one"
			m := [k = 1]
			m["k"]
		"#}, "")
		.contains("key not found")
	);
}

#[test]
fn bracket_lit_undefined_ident_key_fails() {
	fail("[one = 1]", "undefined variable `one`");
}

#[test]
fn bracket_lit_as_call_arg() {
	check(
		indoc! {r#"
			f :: fn(m: [string]int) int { m["one"] }
			f(["one" = 1])
		"#},
		"1",
	);
}

#[test]
fn bracket_lit_mixed_value_types_fail() {
	fail(r#"m :: ["a" = 1, "b" = "two"]"#, "expected int, got string");
}

#[test]
fn delete_key() {
	check(
		indoc! {r#"
			m: [string]int
			m["one"] = 1
			m["two"] = 2
			m.delete["one"]
			m["two"]
		"#},
		"2",
	);
}

#[test]
fn delete_missing_key_is_noop() {
	check(
		indoc! {r#"
			m: [string]int
			m.delete["missing"]
			1
		"#},
		"1",
	);
}

#[test]
fn deleted_key_then_lookup_panics() {
	fail_rt(
		indoc! {r#"
			m: [string]int
			m["one"] = 1
			m.delete["one"]
			m["one"]
		"#},
		"key not found",
	);
}

#[test]
fn delete_on_immutable_map_fails() {
	fail(
		indoc! {r#"
			f :: fn(m: [string]int) int {
				m.delete["one"]
				m["one"]
			}
			n: [string]int
			n["one"] = 1
			f(n)
		"#},
		"immutable",
	);
}

// value semantics (COW)

#[test]
fn index_assign_copy() {
	check(
		indoc! {r#"
			m: [string]int
			m["a"] = 1
			b :: m
			m["a"] = 99
			b["a"]
		"#},
		"1",
	);
	check(
		indoc! {r#"
			m: [string]int
			m["a"] = 1
			b := m
			b["a"] = 99
			m["a"]
		"#},
		"1",
	);
}

#[test]
fn independent_copies() {
	// delete copy
	check(
		indoc! {r#"
			m: [string]int
			m["a"] = 1
			m["b"] = 2
			n := m
			n.delete["a"]
			m["a"]
		"#},
		"1",
	);
	// chain of copies
	check(
		indoc! {r#"
			m: [string]int
			m["a"] = 1
			n :: m
			o := n
			o["a"] = 99
			n["a"]
		"#},
		"1",
	);
	// returned param vs arg
	check(
		indoc! {r#"
			id :: fn(m: [string]int) [string]int { m }
			a: [string]int
			a["a"] = 1
			r := id(a)
			r["a"] = 99
			a["a"]
		"#},
		"1",
	);
	// stored array value
	check(
		indoc! {r#"
			m: [string][]int
			arr := [1]
			m["a"] = arr
			arr << 2
			m["a"]
		"#},
		"[1]",
	);
}
