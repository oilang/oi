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
fn a_ctx_shadow_needs_a_ctx_type() {
	fail("ctx :: 1", "`ctx` can't be int");
}

#[test]
fn a_ctx_write_stays_in_the_callee() {
	let src = indoc! {"
		set :: fn() {
			ctx.thread = 3
			ctx = Context.{ ..ctx, alloc = ctx.temp }
			print(ctx.thread)
		}
		set()
		xs := [ 1 2 3 ]
		print(ctx.thread, xs)
	"};
	check(src, ["3", "0 [1, 2, 3]"]);
	assert_clean(src);
}

#[test]
fn a_ctx_fn_reads_its_ctx_type() {
	let src = indoc! {"
		GameCtx :: struct { Context, dt: float }
		@ctx(none)
		half :: fn(x: float) float { x / 2.0 }
		@ctx(GameCtx)
		step :: fn() { print(half(ctx.dt), ctx.thread) }
		ctx :: GameCtx.{ Context = ctx, dt = 0.5 }
		step()
		f := step
		f()
	"};
	check(src, ["0.25 0", "0.25 0"]);
}

#[test]
fn a_contextless_fn_cant_pass_ctx_on() {
	let src = indoc! {"
		show :: fn() {}
		@ctx(none)
		f :: fn() { show() }
		f()
	"};
	fail(src, "a `@ctx(none)` fn has no `ctx`");
}

#[test]
fn an_installed_allocator_sees_every_allocation() {
	let src = indoc! {"
		count := 0

		@c counting :: fn(data: ptr, mode: AllocMode, size: int, align: int, old: ptr, old_size: int) ptr {
			if mode == .alloc { count = count + 1 }
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
				sys.proc(sys.data, .alloc, size, align, ptr(0), 0)
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
fn a_shadow_frees_its_copy_but_not_what_it_allocated_from() {
	assert_clean(indoc! {"
		I :: struct { a: int, b: int }
		J :: struct { i: I, k: I, l: I, m: I }
		f :: fn() []int {
			ctx :: Context.{ ..ctx, thread = 7 }
			[ 1 2 3 ]
		}
		g :: fn() { j :: J.{ i = I.{ 9, 9 } } }
		h :: fn() {
			xs :: f()
			g()
			print(xs[0])
		}
		h()
	"});
}

#[test]
fn an_arena_rewinds_to_its_first_chunk() {
	for a in ["ctx.temp", "arena()"] {
		check(
			[
				&format!("ctx :: Context.{{ ..ctx, alloc = {a} }}"),
				"xs := [ 1 2 3 ]",
				"at :: xs.ptr",
				"ctx.alloc.free_all()",
				"ys := [ 4 5 6 ]",
				"print(ys.ptr == at)",
			],
			"true",
		);
	}
}

#[test]
fn an_arena_grows_past_its_first_chunk() {
	let src = indoc! {"
		sum :: fn(a: Alloc, n: int) out: int {
			ctx :: Context.{ ..ctx, alloc = a }
			xs := [ 0 ]
			loop i in 1..n { xs << i }
			loop x in xs { out += x }
			a.free_all()
			out
		}
		a :: arena()
		print(sum(a, 100000), sum(a, 10), sum(a, 200000))
	"};
	check(src, "4999950000 45 19999900000");
}
