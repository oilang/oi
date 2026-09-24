use crate::helpers::*;

#[test]
fn print_string() {
	check(r#"print("hello")"#, "hello");
}

#[test]
fn print_int() {
	check("print(42)", "42");
}

#[test]
fn print_bool() {
	check("print(true)", "true");
}

#[test]
fn print_multiple() {
	check(r#"print("a", "b", "c")"#, "a b c");
}

#[test]
fn print_as_statement() {
	check(
		r#"print("hello")
42"#,
		"hello\n42",
	);
}

#[test]
fn print_no_args() {
	check("print()", "");
}

#[test]
fn write_no_newline() {
	check(
		r#"write("hi")
42"#,
		"hi42",
	);
}

#[test]
fn write_no_args() {
	check("write()", "");
}

#[test]
fn eprint_goes_to_stderr() {
	let (stdout, stderr) = run_streams(
		r#"eprint("err")
42"#,
	);
	assert_eq!(stdout, "42");
	assert_eq!(stderr, "err");
}

#[test]
fn ewrite_goes_to_stderr() {
	let (stdout, stderr) = run_streams(
		r#"ewrite("err")
42"#,
	);
	assert_eq!(stdout, "42");
	assert_eq!(stderr, "err");
}

#[test]
fn eprint_no_args() {
	let (stdout, stderr) = run_streams("eprint()");
	assert_eq!(stdout, "");
	assert_eq!(stderr, "");
}

#[test]
fn ewrite_no_args() {
	check("ewrite()", "");
}
