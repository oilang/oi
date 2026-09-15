use crate::helpers::*;

const FILE: &str = indoc! {r#"
	File :: struct { fd: int }
	File : Drop < { drop :: fn(mut self) { print("drop", self.fd) } }
"#};

#[test]
fn reverse_drop_order() {
	check([FILE, "a :: File.{fd = 1}", "b :: File.{fd = 2}"], ["drop 2", "drop 1"]);
}

#[test]
fn bind_move_kills_source() {
	fail_with([FILE, "f :: File.{fd = 1}", "g :: f", "print(f)"], "undefined variable");
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
	fail_with(
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
fn returned_resource_drops_once() {
	check(
		[
			FILE,
			"open :: fn(n: int) File { File.{fd = n} }",
			"f :: open(3)",
			r#"print("before")"#,
		],
		["before", "drop 3"],
	);
}

#[test]
fn resource_field_makes_its_owner_one() {
	let owner = "Handle :: struct { file: File }";
	check(
		[FILE, owner, "h :: Handle.{file = File.{fd = 1}}", r#"print("built")"#],
		["built", "drop 1"],
	);
	fail_with(
		[FILE, owner, "h :: Handle.{file = File.{fd = 1}}", "g :: h", "print(h)"],
		"undefined variable",
	);
	fail_with(
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
	fail_with(
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
	fail_with([FILE, a, "g :: a[0]"], "cannot move File out of its container");
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
fn an_unbound_resource_drops_at_scope_exit() {
	check([FILE, "File.{fd = 1}", r#"print("end")"#], ["end", "drop 1"]);
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
	fail_with(
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
	fail_with([FILE, eat, f, "eat(move f)", "print(f.fd)"], "undefined variable");
	fail_with([FILE, eat, f, "eat(f)"], "missing `move` at the callsite");
	fail_with(["look :: fn(n: int) {}", "look(move 1)"], "not `move`");
}

#[test]
fn move_self_consumes_the_receiver() {
	let close = r#"File :< { close :: fn(move self) { print("closing", self.fd) } }"#;
	check(
		[FILE, close, "f :: File.{fd = 1}", "f.close()", r#"print("after")"#],
		["closing 1", "drop 1", "after"],
	);
	fail_with(
		[FILE, close, "f :: File.{fd = 1}", "f.close()", "print(f)"],
		"undefined variable",
	);
}

#[test]
fn an_object_cannot_hand_over_what_it_borrows() {
	let sink = "Sink :: trait { swallow : fn(move self) }";
	let claim = "File : Sink < { swallow :: fn(move self) {} }";
	let d = "d : Sink : File.{fd = 1}";
	fail_with([FILE, sink, claim, d, "d.swallow()"], "only borrows its data");
	fail_with([FILE, "Sink :: trait { swallow : fn(self) }", claim], "wrong signature");
}

#[test]
fn fixed_array_elements_drop_with_their_last_owner() {
	let a = "a : [2]File : .[File.{fd = 1}, File.{fd = 2}]";
	check([FILE, a, "b :: a", r#"print("built")"#], ["built", "drop 1", "drop 2"]);
	fail_with([FILE, a, "b :: a", "print(a[0].fd)"], "undefined variable");
}

#[test]
fn boxed_payloads_drop_with_their_box() {
	let v = "V :: enum { Empty, Held(File) }";
	check(
		[
			FILE,
			v,
			"o: ?File = File.{fd = 1}",
			"h :: V.Held(File.{fd = 2})",
			r#"print("built")"#,
		],
		["built", "drop 2", "drop 1"],
	);
	fail_with(
		[FILE, v, "f :: File.{fd = 1}", "h :: V.Held(f)", "print(f)"],
		"undefined variable",
	);
}

#[test]
fn tuple_payloads_drop_with_their_tuple() {
	check(
		[FILE, "t :: (File.{fd = 1}, 2)", r#"print("built")"#],
		["built", "drop 1"],
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
	fail_with(
		[
			"Show :: trait { show : fn(self) }",
			"Box[T] :: struct { val: T }",
			"Box[T] : Show < { show :: fn(self) {} }",
		],
		"generic trait claims aren't supported yet",
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
		[REF, "eat :: fn(move r: Ref) {}", "r :: Ref.{id = 1}", "eat(move r)", r#"print("after")"#],
		["drop 1", "after"],
	);
}

#[test]
fn a_container_copies_what_it_owns() {
	let owner = ["Box :: struct { r: Ref }", "b :: Box.{r = Ref.{id = 1}}", "c :: b"].join("\n");
	check([REF, &owner, r#"print("held", c.r.id)"#], ["copy 1", "held 1", "drop 1", "drop 1"]);
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
	fail_with(
		["Plain :: struct { n: int }", "Plain : Copy < { copy :: fn(mut self) {} }"],
		"claims `Copy` without `Drop`",
	);
}
