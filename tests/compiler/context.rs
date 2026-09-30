use crate::common::Project;
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

#[test]
fn an_installed_allocator_sees_every_allocation() {
	let src = indoc! {"
		count := 0

		@c counting :: fn(data: ptr, mode: int, size: int, align: int, old: ptr, old_size: int) ptr {
			if mode == ALLOC { count = count + 1 }
			sys :: system_allocator()
			sys.proc(sys.data, mode, size, align, old, old_size)
		}

		main :: fn() {
			xs := [ 1 2 3 ]
			ctx :: Context.{ ..ctx, alloc = Alloc.{ proc = counting, data = ptr(0) } }
			xs << 4
			print(xs, count > 0)
		}
	"};
	check(src, "[1, 2, 3, 4] true");
}

#[test]
fn an_allocator_claimer_installs_as_ctx_alloc() {
	let src = indoc! {"
		Counter :: struct { hits: int }

		Counter : Allocator < {
			alloc :: fn(mut self, size: int, align: int) ptr {
				self.hits = self.hits + 1
				sys :: system_allocator()
				sys.proc(sys.data, ALLOC, size, align, ptr(0), 0)
			}
		}

		c :: Counter.{ 0 }
		xs := [ 1 2 3 ]
		ctx :: Context.{ ..ctx, alloc = c }
		xs << 4
		print(c.hits > 0)
	"};
	check(src, "true");
}

#[test]
fn an_arena_rewinds_to_its_first_chunk() {
	for a in ["ctx.temp", "arena()"] {
		check(
			[
				&format!("ctx :: Context.{{ ..ctx, alloc = {a} }}"),
				"xs := [1 2 3]",
				"at :: xs.ptr",
				"ctx.alloc.free_all()",
				"ys := [4 5 6]",
				"print(ys.ptr == at)",
			],
			"true",
		);
	}
}

#[test]
fn modules_amend_the_context() {
	Project::new()
		.file("physics.oi", ["module physics", "Context :< { dt: float = 0.5 }"])
		.file("game.oi", ["module game", "Context :< { level: int = 1 }"])
		.file("main.oi", ["use physics", "use game", "print(ctx.dt, ctx.level)"])
		.check("0.5 1");
}
