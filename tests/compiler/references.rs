use crate::helpers::*;

#[test]
fn aliasing_shares_identity() {
	check(
		indoc! {r#"
			Node :: struct { value: int }
			list := &Node.{ value = 5 }
			head :: list
			list.value = 9
			print("head is {head}")
		"#},
		"head is Node.{value = 9}",
	);
}

#[test]
fn bare_ref_without_value_errors() {
	fail_with(
		["Node :: struct { value: int }", "n: &Node"],
		"a reference must be initialized (`?&T` for an optional one)",
	);
}

#[test]
fn optional_ref_zero_value_assign_unwrap() {
	check(["Node :: struct { value: int }", "o: ?&Node", "o"], "none");
	check(
		indoc! {"
			Node :: struct { value: int }
			o: ?&Node
			o = ?&Node.(&Node.{ value = 7 })
			match o {
				.some(n) => n.value,
				.none => -1,
			}
		"},
		"7",
	);
}

#[test]
fn no_leaks_on_share_and_release() {
	assert_clean(indoc! {"
		Node :: struct { value: int }
		list := &Node.{ value = 5 }
		head :: list
		list.value = 9
		print(head.value)
	"});
}

#[test]
fn bare_ref_field_rejected() {
	let src = indoc! {"
		Node :: struct { value: int }
		List :: struct { head: &Node }
	"};
	fail_with(src, "must be optional (`?&T`)");
}

#[test]
fn linked_nodes() {
	check(
		indoc! {r#"
			Node :: struct { value: int, next: ?&Node }
			tail :: &Node.{ value = 2 }
			head :: &Node.{ value = 1, next = ?&Node.(tail) }
			match head.next {
				.some(n) => print("{head.value} -> {n.value}"),
				.none => print("lonely"),
			}
		"#},
		"1 -> 2",
	);
}

#[test]
fn interior_ref_frees_on_release() {
	assert_clean(indoc! {"
		Node :: struct { value: int, next: ?&Node }
		head :: &Node.{ value = 1, next = ?&Node.(&Node.{ value = 2 }) }
		print(head.value)
	"});
}

#[test]
fn shared_option_ref_stays_clean() {
	assert_clean(indoc! {"
		Node :: struct { value: int, next: ?&Node }
		t :: &Node.{ value = 2 }
		o :: ?&Node.(t)
		a :: &Node.{ value = 1, next = o }
		b :: &Node.{ value = 3, next = o }
		print(t.value)
	"});
}

#[test]
fn returned_box_keeps_zeroed_field() {
	let src = indoc! {r#"
		Node :: struct { value: int, next: ?&Node }
		make :: fn() &Node { &Node.{ value = 1 } }
		n :: make()
		match n.next {
			.some(x) => print(x.value),
			.none => print("ok"),
		}
	"#};
	check(src, "ok");
	assert_clean(src);
}

#[test]
fn rebind_releases_old_target() {
	assert_clean(indoc! {r#"
		Node :: struct { value: int }
		o: ?&Node
		o = ?&Node.(&Node.{ value = 1 })
		o = ?&Node.(&Node.{ value = 2 })
		print("done")
	"#});
}

#[test]
fn user_enum_with_ref_payload_stays_boxed() {
	check(
		indoc! {r#"
			Node :: struct { value: int }
			E :: enum { empty, full(&Node) }
			e :: E.full(&Node.{ value = 7 })
			print(e)
			match e {
				.full(n) => print(n.value),
				.empty => print("no"),
			}
		"#},
		["full(Node.{value = 7})", "7"],
	);
}

#[test]
fn value_recursion_still_errors() {
	fail_with(
		["A :: struct { b: B }", "B :: struct { a: A }"],
		"would require infinitely nested fields",
	);
}

#[test]
fn two_node_cycle_reclaimed() {
	assert_clean(indoc! {r#"
		Node :: struct { value: int, next: ?&Node }
		a := &Node.{ value = 1 }
		b :: &Node.{ value = 2, next = ?&Node.(a) }
		a.next = ?&Node.(b)
		print(a.value)
	"#});
}

#[test]
fn self_cycle_reclaimed() {
	assert_clean(indoc! {r#"
		Node :: struct { value: int, next: ?&Node }
		n := &Node.{ value = 1 }
		n.next = ?&Node.(n)
		print(n.value)
	"#});
}

#[test]
fn cycle_with_acyclic_hangoff_reclaimed() {
	assert_clean(indoc! {r#"
		Leaf :: struct { v: int }
		Node :: struct { value: int, leaf: ?&Leaf, next: ?&Node }
		a := &Node.{ value = 1, leaf = ?&Leaf.(&Leaf.{ v = 9 }) }
		b :: &Node.{ value = 2, next = ?&Node.(a) }
		a.next = ?&Node.(b)
		print(a.value)
	"#});
}
