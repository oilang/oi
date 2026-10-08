use crate::helpers::*;
use indoc::indoc;

#[test]
fn set_get() {
	check(
		indoc! {r#"
			m: [string]int
			e: [string]int = []
			n: [int]string
			m["one"] = 1
			m["two"] = 2
			m["one"] = 10
			e["one"] = 1
			n[1] = "a"
			print(m["one"] + m["two"], e["one"], n[1])
		"#},
		"12 1 a",
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
		fail(
			indoc! {"
				Point :: (int, int)
				m: [Point]int
				m[(1, 2)] = 6
				m[(2, 1)] = 9
				m[(2, 1)]
			"},
			""
		)
		.contains("(int, int) cannot be used as a map key")
	);
}

#[test]
fn wrong_key_or_value_type() {
	fail(["m: [string]int", "m[1]"], "expected string key");
	fail(["m: [string]int", r#"m["a"] = "b""#], "type mismatch");
}

#[test]
fn bracket_literals() {
	check(
		indoc! {r#"
			f :: fn(m: [string]int) int { m["one"] }
			m :: [
				"one" = 1
				"two" = 2
			]
			t: [string]f64 : ["a" = 1.5]
			print(m["one"] + m["two"], t["a"], [1 = "one", 2 = "two"][1], [:ok = 200, :not_found = 404][:ok])
			print(f(["one" = 1]))
		"#},
		["3 1.5 one 200", "1"],
	);
}

#[test]
fn empty_array_against_array_target() {
	check(["a: []int = []", "a.len"], "0");
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
		fail_rt(
			indoc! {r#"
				k :: "one"
				m := [k = 1]
				m["k"]
			"#},
			""
		)
		.contains("key not found")
	);
}

#[test]
fn bracket_lit_undefined_ident_key_fails() {
	fail("[one = 1]", "undefined variable `one`");
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
			m.delete["missing"]
			(m.len, m["two"])
		"#},
		"(1, 2)",
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

#[test]
fn equality_is_order_independent() {
	check(
		indoc! {r#"
			a := ["x" = [1, 2], "y" = [3]]
			print a == ["y" = [3], "x" = [1, 2]]
			print a == ["x" = [1, 2]]
			print a == ["x" = [1, 2], "y" = [4]]
			print a == ["z" = [3], "x" = [1, 2]]
			print a != ["y" = [3], "x" = [1, 2]]
		"#},
		["true", "false", "false", "false", "false"],
	);
}

#[test]
fn equality_rejects_a_missing_key() {
	check(
		indoc! {r#"
			a := ["p" = "v", "q" = "v", "r" = "v", "s" = "v"]
			print a == ["p" = "v", "q" = "v", "r" = "v", "z" = "v"]
			print a == ["p" = "v", "q" = "v", "r" = "v", "s" = "v"]
		"#},
		["false", "true"],
	);
}

#[test]
fn dot_literals() {
	check(
		indoc! {r#"
			M :: [string]int
			a: M = .["a" = 1]
			b :: M.["b" = 2]
			e: M = .[]
			a["a"] + b["b"] + e.len
		"#},
		"3",
	);
}
