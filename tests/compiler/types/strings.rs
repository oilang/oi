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
	check(r#"print("\u{41}\u{1F600}\x41\e")"#, "A\u{1F600}A\u{1b}");
}

#[test]
fn unknown_escape_fails() {
	fail(r#"print("\z")"#, "");
	fail(r#"print("\u{41")"#, "");
	fail(r#"print("\xff")"#, "");
}

#[test]
fn runes() {
	check(r"print('a' + 1, '\n', '\u{2603}')", "98 10 9731");
	check(r#"print("aloha"[0] == 'a')"#, "true");
	check(["c : rune : 'z'", "print(c)"], "z");
	fail("r : rune : 1114112", "out of range for rune");
	fail("print('')", "");
	fail("print('ab')", "");
	fail(r"print('\z')", "");
}

#[test]
fn string_iterates_by_rune() {
	check(
		indoc! {r#"
			s :: "h\u{2603}"
			loop r in s { print(r) }
			loop i in 0..s.len { print(s[i]) }
		"#},
		["h", "\u{2603}", "104", "226", "152", "131"],
	);
}

#[test]
fn raw_strings() {
	check(r#"print(r"no\nescape")"#, r"no\nescape");
	check(r#"print(r"C:\Users\{who}")"#, r"C:\Users\{who}");
	check(r#"r"a\b" + "!""#, r"a\b!");
}

#[test]
fn triple_quoted() {
	check(r#"who := "mom"; print("""say "hi" to {who}""")"#, r#"say "hi" to mom"#);
	check(r#"print(r"""raw "q" {who} \n""")"#, r#"raw "q" {who} \n"#);
	check(r#"print("""ends in \"""")"#, r#"ends in ""#);
	check(r#"print(""""holds a """ run"""")"#, r#"holds a """ run"#);
}

#[test]
fn dedent() {
	check("print(\"\"\"\n\t\tone\n\t\t  two\n\t\t\"\"\")", ["one", "  two"]);
	check("print(\"\"\"\n\t\t\\tone\n\t\t\"\"\")", "\tone");
	check("print(\"\"\"one\n\t\ttwo\"\"\")", ["one", "\t\ttwo"]);
	check("print(\"\n\tone\n\" == \"\\n\\tone\\n\")", "true");
	check("print(\"\"\"\n\t\tone\n\t\"\"\")", "\tone");
	check("print(\"\"\"\n\t\"\"\" == \"\")", "true");
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
