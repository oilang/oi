use crate::helpers::*;

#[test]
fn prints() {
	check(
		indoc! {r#"
			print("hello")
			print(42, true)
			print("a", "b", "c")
			print()
			write()
			write("hi")
			42
		"#},
		["hello", "42 true", "a b c", "", "hi42"],
	);
}

#[test]
fn stderr() {
	let (stdout, stderr) = run_streams(indoc! {r#"
		eprint("err")
		ewrite("x")
		ewrite()
		eprint()
		42
	"#});
	assert_eq!(stdout, "42");
	assert_eq!(stderr, "err\nx");
}
