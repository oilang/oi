use crate::helpers::*;
use indoc::indoc;

#[test]
fn string_concat() {
	check(r#""foo" + "bar""#, "foobar");
}

#[test]
fn string_eq_true() {
	check(r#""foo" == "foo""#, "true");
}

#[test]
fn string_ne_true() {
	check(r#""foo" != "bar""#, "true");
}

#[test]
fn string_in_found() {
	check(r#""foo" in "foobar""#, "true");
}

#[test]
fn string_in_not_found() {
	check(r#""baz" in "foobar""#, "false");
}

#[test]
fn string_in_exact_match() {
	check(r#""foo" in "foo""#, "true");
}

#[test]
fn string_in_empty_value() {
	// empty string is always a substring
	check(r#""" in "foo""#, "true");
}

#[test]
fn string_in_type_mismatch_error() {
	fail(r#"42 in "foo""#, "type mismatch");
}

#[test]
fn string_from_bytes() {
	check(
		indoc! {"
			out: []u8 = []
			out << 104
			out << 105
			print(string.(out))
		"},
		"hi",
	);
}

#[test]
fn cstr_from_slice_copies() {
	// data[5] is a space, not a NUL, which forces the copy branch
	check(r#"print(unsafe "hello world"[0..5].cstr().str())"#, "hello");
}

#[test]
fn cstr_from_literal_is_zero_cost() {
	check(
		indoc! {r#"
			f :: fn(p: cstr) string { unsafe p.str() }
			print(f("hey"))
		"#},
		"hey",
	);
}

#[test]
fn string_from_ptr_copies() {
	check(r#"print(unsafe "hello".ptr.string(4))"#, "hell");
	check(r#"print(unsafe "hello".ptr.offset(1).string(2))"#, "el");
	check(
		r#"print(ptr(0).is_null(), unsafe { ptr(0).string(4) } == "")"#,
		"true true",
	);
}

#[test]
fn escapes() {
	check(r#"print("a\nb\tc")"#, ["a", "b\tc"]);
	check(r#"print("q: \" back: \\")"#, r#"q: " back: \"#);
}

#[test]
fn unknown_escape_fails() {
	fail(r#"print("\z")"#, "");
}

#[test]
fn raw_strings() {
	check(r#"print(r"no\nescape")"#, r"no\nescape");
	check(r#"print(r"C:\Users\{who}")"#, r"C:\Users\{who}");
	check(r#"r"a\b" + "!""#, r"a\b!");
}

#[test]
fn len_and_index() {
	check(r#"print("hello".len)"#, "5");
	check(r#"print("abc"[1])"#, "98");
	fail_rt(r#"print("abc"[9])"#, "out of range");
}

#[test]
fn slices() {
	check(r#"print("hello"[1..3])"#, "el");
	check(r#"print("hello"[..2])"#, "he");
	check(r#"print("hello"[2..])"#, "llo");
	fail_rt(r#""abc"[1..9]"#, "out of bounds");
}

#[test]
fn immutable() {
	fail([r#"a := "abc""#, "a[0] = 1"], "strings are immutable");
}
