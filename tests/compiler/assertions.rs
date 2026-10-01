use crate::helpers::*;

#[test]
fn asserts() {
	check("assert!(true)", "");
	check("assert!(2 > 1)", "");
	fail_rt("assert!(false)", "assertion failed: false");
	fail_rt(r#"assert!(false, "bad value")"#, "bad value");
	fail("assert!()", "1 or 2 arguments");
	fail(r#"assert!(true, "a", "b")"#, "1 or 2 arguments");
	fail("assert!(1)", "must be Bool");
	fail("assert!(false, 42)", "must be Str");
}
#[test]
fn panics() {
	fail_rt(r#"panic!("uh oh")"#, "panic: uh oh");
	fail_rt("panic!()", "panic: panicked");
	fail(r#"panic!("a", "b")"#, "0 or 1 arguments");
	fail("panic!(42)", "must be Str");
}

#[test]
fn panic_hooks() {
	// runs first
	let src = indoc! {r#"
		hook :: fn(msg: string, at: Src) { eprint("caught: {msg}") }
		ctx :: Context.{ ..ctx, panic = hook }
		panic!("uh oh")
	"#};
	fail_rt(src, "caught: uh oh");

	// oob
	let src = indoc! {r#"
		hook :: fn(msg: string, at: Src) { eprint("caught at {at.line}: {msg}") }
		ctx :: Context.{ ..ctx, panic = hook }
		a := [1, 2, 3]
		a[5]
	"#};
	fail_rt(src, "caught at 4:");
}
