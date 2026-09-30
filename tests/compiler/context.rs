use crate::helpers::*;

#[test]
fn ctx_shadowing() {
	let src = indoc! {"
		show :: fn() { print(ctx.thread) }
		show()
		if true {
			ctx :: Context.{ ..ctx, thread = 7 }
			show()
		}
		show()
	"};
	check(src, ["0", "7", "0"]);
}

#[test]
fn c_fn_reads_the_root_ctx() {
	let src = indoc! {"
		@c
		tick :: fn(n: i32) { print(ctx.thread, n) }
		ctx :: Context.{ ..ctx, thread = 7 }
		tick(1)
	"};
	check(src, "0 1");
}

#[test]
fn ctx_cannot_be_rebound() {
	fail("ctx := 1", "`ctx` is reserved");
}
