use crate::helpers::*;

#[test]
fn basic_dispatch() {
	let src = indoc! {"
		Box[T] :: struct { v: T }
		Box[T] :< { get :: fn(self) T { self.v } }
		Box.{ v = 7 }.get()
	"};
	check(src, "7");
}

#[test]
fn two_instances_coexist() {
	let src = indoc! {r#"
		Box[T] :: struct { v: T }
		Box[T] :< { get :: fn(self) T { self.v } }
		print(Box.{ v = 1 }.get())
		print(Box.{ v = "hi" }.get())
	"#};
	check(src, ["1", "hi"]);
}

#[test]
fn method_own_type_param() {
	let src = indoc! {r#"
		Box[T] :: struct { v: T }
		Box[T] :< { swap[U] :: fn(self, u: U) U { u } }
		Box.{ v = 1 }.swap("hi")
	"#};
	check(src, "hi");
}

#[test]
fn self_return() {
	let src = indoc! {"
		Box[T] :: struct { v: T }
		Box[T] :< { same :: fn(self) Self { self } }
		Box.{ v = 3 }.same().v
	"};
	check(src, "3");
}

#[test]
fn concrete_impl_own_type_param() {
	let src = indoc! {"
		Point :: struct { x: int, y: int }
		Point :< { id[U] :: fn(self, u: U) U { u } }
		Point.{1, 2}.id(5)
	"};
	check(src, "5");
}

#[test]
fn unknown_method_error() {
	fail(
		indoc! {"
			Box[T] :: struct { v: T }
			Box[T] :< { get :: fn(self) T { self.v } }
			Box.{ v = 1 }.nope()
		"},
		"no such method",
	);
}

#[test]
fn field_through_self() {
	let src = indoc! {"
		Box[T] :: struct { v: T }
		Box[T] :< { double :: fn(self) T { self.v + self.v } }
		Box.{ v = 7 }.double()
	"};
	check(src, "14");
}

#[test]
fn static_fill_infers_from_args() {
	let src = indoc! {r#"
		Box[T] :: struct { v: T }
		Box[T] :< { new :: fn(v: T) Self { Box.{ v = v } } }
		print(Box.new(5).v)
		print(Box.new("hi").v)
	"#};
	check(src, ["5", "hi"]);
}

#[test]
fn explicit_type_arg() {
	let src = indoc! {"
		use math
		Cell :: struct { n: int }
		Cell :< { blank[T] :: fn(self) T { z: T; z } }
		print(Cell.{ 1 }.blank[int]())
		print(math.min[int](3, 7))
	"};
	check(src, ["0", "3"]);
}

#[test]
fn amend_imported_generic_enum() {
	let src = indoc! {r#"
		Option[T] :< {
			or_zero :: fn(self) T { self or { z: T; z } }
		}
		n: ?int = 4
		s: ?string = none
		print(n.or_zero())
		print(s.or_zero())
	"#};
	check(src, ["4", ""]);
}
