use crate::helpers::*;

#[test]
fn aliasing_shares_identity() {
	let src = indoc! {r#"
		Node :: struct { value: int }
		list := &Node.{ value = 5 }
		head :: list
		list.value = 9
		print("head is {head}")
	"#};
	check(src, "head is Node.{value = 9}");
	assert_clean(src);
}

#[test]
fn ref_of_existing_aliases() {
	check(
		indoc! {"
			Node :: struct { value: int, tags: []int }
			n := Node.{ value = 5, tags = [1] }
			r :: &n
			s := r
			s.value = 9
			s.tags = [2]
			n.value = 7
			print(n, r)
		"},
		"Node.{value = 7, tags = [2]} Node.{value = 7, tags = [2]}",
	);
}

const ESCAPE: &str = indoc! {r#"
	name :: fn() ^string {
		s := "hi"
		p := &s
		s = "bye"
		return p
	}
	n := name()
	print(n)
"#};

#[test]
fn ref_of_local_escapes() {
	check(ESCAPE, "bye");
	assert_clean(ESCAPE);
}

#[test]
fn ref_of_immutable_errors() {
	fail(["x :: 5", "p := &x"], "cannot take the address of `x`");
}

#[test]
fn bare_ref_without_value_errors() {
	fail(
		["Node :: struct { value: int }", "n: ^Node"],
		"a reference must be initialized (`?^T` for an optional one)",
	);
}

#[test]
fn optional_ref_zero_value_assign_unwrap() {
	check(
		indoc! {"
			Node :: struct { value: int }
			o: ?^Node
			print(o)
			o = ?^Node.(&Node.{ value = 7 })
			match o {
				.some.(n) => n.value,
				.none => -1,
			}
		"},
		["none", "7"],
	);
}

#[test]
fn bare_ref_field_rejected() {
	let src = indoc! {"
		Node :: struct { value: int }
		List :: struct { head: ^Node }
	"};
	fail(src, "must be optional (`?^T`)");
}

#[test]
fn linked_nodes() {
	check(
		indoc! {r#"
			Node :: struct { value: int, next: ?^Node }
			tail :: &Node.{ value = 2 }
			head :: &Node.{ value = 1, next = ?^Node.(tail) }
			match head.next {
				.some.(n) => print("{head.value} -> {n.value}"),
				.none => print("lonely"),
			}
		"#},
		"1 -> 2",
	);
}

#[test]
fn interior_and_shared_refs_free_on_release() {
	assert_clean(indoc! {"
		Node :: struct { value: int, next: ?^Node }
		head :: &Node.{ value = 1, next = ?^Node.(&Node.{ value = 2 }) }
		t :: &Node.{ value = 2 }
		o :: ?^Node.(t)
		a :: &Node.{ value = 1, next = o }
		b :: &Node.{ value = 3, next = o }
		r: ?^Node
		r = ?^Node.(&Node.{ value = 1 })
		r = ?^Node.(&Node.{ value = 2 })
		print(head.value, t.value)
	"});
}

#[test]
fn returned_box_keeps_zeroed_field() {
	let src = indoc! {r#"
		Node :: struct { value: int, next: ?^Node }
		make :: fn() ^Node { &Node.{ value = 1 } }
		n :: make()
		match n.next {
			.some.(x) => print(x.value),
			.none => print("ok"),
		}
	"#};
	check(src, "ok");
	assert_clean(src);
}

#[test]
fn user_enum_with_ref_payload_stays_boxed() {
	check(
		indoc! {r#"
			Node :: struct { value: int }
			E :: enum { empty, full(^Node) }
			e :: E.full.(&Node.{ value = 7 })
			print(e)
			match e {
				.full.(n) => print(n.value),
				.empty => print("no"),
			}
		"#},
		["full.(Node.{value = 7})", "7"],
	);
}

#[test]
fn self_ref_array_field_prints() {
	check(
		indoc! {"
			Node :: struct { value: int, kids: []^Node }
			a := &Node.{ value = 1 }
			a.kids << &Node.{ value = 2 }
			print(a)
		"},
		"Node.{value = 1, kids = [Node.{value = 2, kids = []}]}",
	);
}

#[test]
fn value_recursion_still_errors() {
	fail(
		["A :: struct { b: B }", "B :: struct { a: A }"],
		"would require infinitely nested fields",
	);
}

#[test]
fn cycles_reclaimed() {
	assert_clean(indoc! {r#"
		Node :: struct { value: int, next: ?^Node }
		a := &Node.{ value = 1 }
		b :: &Node.{ value = 2, next = ?^Node.(a) }
		a.next = ?^Node.(b)
		n := &Node.{ value = 1 }
		n.next = ?^Node.(n)
		print(a.value, n.value)
	"#});
}

#[test]
fn cycle_with_acyclic_hangoff_reclaimed() {
	assert_clean(indoc! {r#"
		Leaf :: struct { v: int }
		Node :: struct { value: int, leaf: ?^Leaf, next: ?^Node }
		a := &Node.{ value = 1, leaf = ?^Leaf.(&Leaf.{ v = 9 }) }
		b :: &Node.{ value = 2, next = ?^Node.(a) }
		a.next = ?^Node.(b)
		print(a.value)
	"#});
}

#[test]
fn ref_boxes_any_type() {
	check(["p: ^int = &5", r#"s := &"hi""#, r#"print("{p}{s}", s.len)"#], "5hi 2");
	check(["xs := &[1, 2, 3]", "print(xs[0], xs[1..])"], "1 [2, 3]");
	assert_clean(["xs := &[1, 2]", "print(xs)"]);
}

#[test]
fn eq_is_identity() {
	let src = indoc! {"
		Node :: struct { value: int }
		a := &Node.{ value = 1 }
		b := &Node.{ value = 1 }
		c := a
		print(a == b, a == c, a != b, a^ == b^, ?^Node.(a) == ?^Node.(c))
	"};
	check(src, "false true true true true");
	assert_clean(src);
}

#[test]
fn match_sees_through_ref() {
	check(
		indoc! {r#"
			P :: struct { x: int, y: int }
			p := &P.{ x = 1, y = 2 }
			print(match p { P.{ x, y } => x + y, })
			n := 5
			q := &n
			match q { 5 => print("five"), _ => print("other") }
		"#},
		["3", "five"],
	);
}

#[test]
fn deref_reads_and_writes() {
	check(["p := &5", "p^ = 6", "p^ += 1", "print(p^)"], "7");
	check(["x := 5", "p := &x", "p^ = 6", "print(x)"], "6");
	check(
		indoc! {"
			Node :: struct { value: int }
			p := &Node.{ value = 5 }
			s := p^
			p.value = 9
			print(s.value, p.value)
		"},
		"5 9",
	);
	fail(["p :: &5", "p^ = 6"], "cannot assign through immutable `p`");
	fail(["n := 5", "n^"], "cannot deref int, it is not a pointer");
	assert_clean([r#"s := &"a""#, r#"s^ = "b" + "c""#, "print(s^)"]);
	check(
		indoc! {"
			P :: struct { a: ^int = &0 }
			n := 1
			s := P.{ a = &n }
			s.a^ = 5
			ps := [&n]
			ps[0]^ += 1
			print(n)
		"},
		"6",
	);
}
