use crate::helpers::*;

const FILE: &str = indoc! {r#"
	File :: struct { fd: int }
	File : Drop < { drop :: fn(mut self) { print("drop", self.fd) } }
"#};

#[test]
fn reverse_drop_order() {
	check(
		[FILE, "a :: File.{fd = 1}", "b :: File.{fd = 2}"],
		["File.{fd = 2}", "drop 2", "drop 1"],
	);
}

#[test]
fn bind_move_kills_source() {
	fail([FILE, "f :: File.{fd = 1}", "g :: f", "print(f)"], "undefined variable");
}

#[test]
fn arg_borrows_and_drops_once() {
	check(
		[
			FILE,
			"look :: fn(f: File) {}",
			"f :: File.{fd = 1}",
			"look(f)",
			r#"print("marker")"#,
		],
		["marker", "drop 1"],
	);
}

#[test]
fn callee_cannot_steal_a_borrowed_arg() {
	fail(
		[
			FILE,
			"steal :: fn(f: File) { g :: f }",
			"f :: File.{fd = 1}",
			"steal(f)",
		],
		"it is borrowed here",
	);
}

#[test]
fn returned_tuple_and_unbound_resources_drop_once() {
	check(
		[
			FILE,
			"open :: fn(n: int) File { File.{fd = n} }",
			"f :: open(3)",
			"t :: (File.{fd = 1}, 2)",
			"File.{fd = 5}",
			r#"print("end")"#,
		],
		["end", "drop 5", "drop 1", "drop 3"],
	);
}

#[test]
fn block_tail_moves_its_local_out() {
	check(
		[
			FILE,
			"g :: fn() File { f := File.{fd = 1}; { f } }",
			"x :: g()",
			"y :: { f := File.{fd = 2}; f }",
			"print(x.fd, y.fd)",
		],
		["1 2", "drop 2", "drop 1"],
	);
}

#[test]
fn resource_field_makes_its_owner_one() {
	let owner = "Handle :: struct { file: File }";
	check(
		[FILE, owner, "h :: Handle.{file = File.{fd = 1}}", r#"print("built")"#],
		["built", "drop 1"],
	);
	fail(
		[FILE, owner, "h :: Handle.{file = File.{fd = 1}}", "g :: h", "print(h)"],
		"undefined variable",
	);
	fail(
		[FILE, owner, "f :: File.{fd = 1}", "h :: Handle.{file = f}", "print(f)"],
		"undefined variable",
	);
}

#[test]
fn array_elements_drop_with_their_last_owner() {
	check(
		[
			FILE,
			"a :: [File.{fd = 1}, File.{fd = 2}]",
			"b :: a",
			r#"print("built")"#,
		],
		["built", "drop 1", "drop 2"],
	);
	fail(
		[FILE, "f :: File.{fd = 1}", "a :: [f]", "print(f)"],
		"undefined variable",
	);
}

#[test]
fn a_projected_resource_is_a_borrow() {
	let a = "a :: [File.{fd = 1}]";
	check(
		[FILE, a, "look :: fn(f: File) {}", "look(a[0])", "print(a[0].fd)"],
		["1", "drop 1"],
	);
	fail([FILE, a, "g :: a[0]"], "cannot move File out of its container");
}

#[test]
fn a_variant_path_is_a_fresh_value() {
	check(
		[FILE, "E :: enum { A(File), B }", "e := E.B", "f := e", r#"print("ok")"#],
		"ok",
	);
}

#[test]
fn map_values_drop_with_their_last_owner() {
	let f = "f :: File.{fd = 1}";
	check([FILE, f, r#"m :: ["a" = f]"#, r#"print("built")"#], ["built", "drop 1"]);
	check(
		[FILE, "m: [string]File", f, r#"m["a"] = f"#, r#"print("set")"#],
		["set", "drop 1"],
	);
}

#[test]
fn index_and_field_stores_move_and_drop_the_old_value() {
	check(
		[
			FILE,
			"Box :: struct { f: File }",
			"a := [File.{fd = 1}]",
			"b := Box.{f = File.{fd = 2}}",
			"f :: File.{fd = 3}",
			"g :: File.{fd = 4}",
			"a[0] = f",
			"b.f = g",
			r#"print("set")"#,
		],
		["drop 1", "drop 2", "set", "drop 4", "drop 3"],
	);
}

#[test]
fn overwrite_drops_the_old_value_and_moves_the_new() {
	check(
		[
			FILE,
			"f := File.{fd = 1}",
			"g :: File.{fd = 2}",
			"f = g",
			r#"print("set")"#,
		],
		["drop 1", "set", "drop 2"],
	);
	fail(
		[FILE, "f := File.{fd = 1}", "g :: File.{fd = 2}", "f = g", "print(g)"],
		"undefined variable",
	);
}

#[test]
fn a_move_arg_transfers_ownership() {
	let eat = r#"eat :: fn(move f: File) { print("ate", f.fd) }"#;
	let f = "f :: File.{fd = 1}";
	check(
		[FILE, eat, f, "eat(move f)", r#"print("after")"#],
		["ate 1", "drop 1", "after"],
	);
	fail([FILE, eat, f, "eat(move f)", "print(f.fd)"], "undefined variable");
	fail([FILE, eat, f, "eat(f)"], "missing `move` at the callsite");
	fail(["look :: fn(n: int) {}", "look(move 1)"], "not `move`");
}

#[test]
fn a_mutable_copy_of_a_resource_is_owned() {
	let f = "f :: fn(x: ?File = none) int { x = File.{fd = 9}  1 }";
	let g = "g :: File.{fd = 1}";
	check(
		[FILE, f, g, "print(f())", "print(f(move g))"],
		["drop 9", "1", "drop 1", "drop 9", "1"],
	);
	fail([FILE, f, g, "f(g)"], "missing `move` at the callsite");
	let t = "t[T] :: fn(x: ?T = none) int { 1 }";
	check(
		[FILE, t, g, "print(t[File]())", "print(t(move g))", "print(t(3))"],
		["1", "drop 1", "1", "1"],
	);
}

#[test]
fn a_branch_move_drops_at_scope_exit() {
	let go = indoc! {r#"
		eat :: fn(move f: File) { print("ate", f.fd) }
		go :: fn(n: int) {
			f :: File.{fd = n}
			if n == 1 { eat(move f) }
			print("end", n)
		}
	"#};
	check(
		[FILE, go, "go(1)", "go(3)"],
		["ate 1", "drop 1", "end 1", "end 3", "drop 3"],
	);
	fail(
		[FILE, go, "f :: File.{fd = 1}", "if true { eat(move f) }", "print(f.fd)"],
		"undefined variable",
	);
}

#[test]
fn move_self_consumes_the_receiver() {
	let close = r#"File :< { close :: fn(move self) { print("closing", self.fd) } }"#;
	check(
		[FILE, close, "f :: File.{fd = 1}", "f.close()", r#"print("after")"#],
		["closing 1", "drop 1", "after"],
	);
	fail(
		[FILE, close, "f :: File.{fd = 1}", "f.close()", "print(f)"],
		"undefined variable",
	);
}

#[test]
fn an_object_cannot_hand_over_what_it_borrows() {
	let sink = "Sink :: trait { swallow : fn(move self) }";
	let claim = "File : Sink < { swallow :: fn(move self) {} }";
	let d = "d : Sink : File.{fd = 1}";
	fail([FILE, sink, claim, d, "d.swallow()"], "only borrows its data");
	fail([FILE, "Sink :: trait { swallow : fn(self) }", claim], "wrong signature");
}

#[test]
fn fixed_array_elements_drop_with_their_last_owner() {
	let a = "a : [2]File : .[File.{fd = 1}, File.{fd = 2}]";
	check([FILE, a, "b :: a", r#"print("built")"#], ["built", "drop 1", "drop 2"]);
	fail([FILE, a, "b :: a", "print(a[0].fd)"], "undefined variable");
}

#[test]
fn boxed_payloads_drop_with_their_box() {
	let v = "V :: enum { Empty, Held(File) }";
	check(
		[
			FILE,
			v,
			"o: ?File = File.{fd = 1}",
			"h :: V.Held.(File.{fd = 2})",
			r#"print("built")"#,
		],
		["built", "drop 2", "drop 1"],
	);
	assert_clean([FILE, v, "o: ?File = File.{fd = 1}", "h :: V.Held.(File.{fd = 2})"]);
	fail(
		[FILE, v, "f :: File.{fd = 1}", "h :: V.Held.(f)", "print(f)"],
		"undefined variable",
	);
}

#[test]
fn a_generic_claim_drops_each_instance() {
	check(
		[
			"Box[T] :: struct { val: T }",
			r#"Box[T] : Drop < { drop :: fn(mut self) { print("drop", self.val) } }"#,
			"a :: Box[int].{val = 1}",
			r#"b :: Box[string].{val = "two"}"#,
			r#"print("built")"#,
		],
		["built", "drop two", "drop 1"],
	);
}

const REF: &str = indoc! {r#"
	Ref :: struct { id: int }
	Ref : Drop, Copy < {
		drop :: fn(mut self) { print("drop", self.id) }
		copy :: fn(mut self) { print("copy", self.id) }
	}
"#};

#[test]
fn a_copy_claim_binds_a_duplicate() {
	check(
		[REF, "a :: Ref.{id = 1}", "b :: a", r#"print("bound", a.id, b.id)"#],
		["copy 1", "bound 1 1", "drop 1", "drop 1"],
	);
}

#[test]
fn a_move_skips_the_copy_hook() {
	check(
		[
			REF,
			"eat :: fn(move r: Ref) {}",
			"r :: Ref.{id = 1}",
			"eat(move r)",
			r#"print("after")"#,
		],
		["drop 1", "after"],
	);
}

#[test]
fn a_container_copies_what_it_owns() {
	let owner = ["Box :: struct { r: Ref }", "b :: Box.{r = Ref.{id = 1}}", "c :: b"].join("\n");
	check(
		[REF, &owner, r#"print("held", c.r.id)"#],
		["copy 1", "held 1", "drop 1", "drop 1"],
	);
	check(
		[REF, "a :: [Ref.{id = 2}]", "g :: a[0]", r#"print("held", g.id)"#],
		["copy 2", "held 2", "drop 2", "drop 2"],
	);
}

#[test]
fn a_generic_claim_copies_each_instance() {
	check(
		[
			"Box[T] :: struct { val: T }",
			indoc! {r#"
				Box[T] : Drop, Copy < {
					drop :: fn(mut self) { print("drop", self.val) }
					copy :: fn(mut self) { print("copy", self.val) }
				}
			"#},
			"a :: Box[int].{val = 1}",
			"b :: a",
			r#"print("built", b.val)"#,
		],
		["copy 1", "built 1", "drop 1", "drop 1"],
	);
}

#[test]
fn copy_without_drop_is_rejected() {
	fail(
		[
			"Plain :: struct { n: int }",
			"Plain : Copy < { copy :: fn(mut self) {} }",
		],
		"claims `Copy` without `Drop`",
	);
}

#[test]
fn moved_capture_drops_with_the_last_closure_copy() {
	check(
		[
			FILE,
			"mk :: fn(n: int) fn() int {",
			"	f :: File.{fd = n}",
			"	return fn [move f] () int { f.fd }",
			"}",
			"g :: mk(4)",
			"h :: g",
			"print(g(), h())",
		],
		["4 4", "drop 4"],
	);
}
