use crate::helpers::*;

#[test]
fn user_enum_with_common_variants() {
	check(
		indoc! {r#"
			MyOption :: enum { none some }
			MyOption.none
		"#},
		"none",
	);
	check(
		indoc! {r#"
			MyResult :: enum { ok err }
			MyResult.ok
		"#},
		"ok",
	);
}

#[test]
fn builtins_still_work() {
	check("?int.(none)", "none");
	check("!int.(42)", "ok.(42)");
}

#[test]
fn qualified_access() {
	check(["Color :: enum { red green blue }", "Color.red"], "red");
	check(["Color :: enum { red green blue }", "Color.blue"], "blue");
}

#[test]
fn bind() {
	check(["Color :: enum { red green blue }", "c :: Color.green", "c"], "green");
}

#[test]
fn reassign() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			c := Color.red
			c = Color.blue
			c
		"},
		"blue",
	);
}

#[test]
fn first_variant_is_default() {
	check(["Color :: enum { red green blue }", "c: Color", "c"], "red");
}

#[test]
fn empty_literal_is_default() {
	check(["Color :: enum { red green blue }", "Color.{}"], "red");
}

#[test]
fn empty_literal_rejects_fields() {
	fail(["Color :: enum { red green blue }", "Color.{ red }"], "only supports");
}

#[test]
fn eq_same() {
	check(["Color :: enum { red green blue }", "Color.red == Color.red"], "true");
}

#[test]
fn eq_different() {
	check(["Color :: enum { red green blue }", "Color.red == Color.blue"], "false");
}

#[test]
fn ne() {
	check(["Color :: enum { red green blue }", "Color.red != Color.blue"], "true");
}

#[test]
fn returned_from_fn() {
	check(
		[
			"Color :: enum { red green blue }",
			"fav :: fn() Color { Color.blue }",
			"fav()",
		],
		"blue",
	);
}

#[test]
fn struct_field() {
	check(
		"Stat :: enum { health mana stamina }
		User :: struct { s: Stat }
		u :: User.{ s = Stat.mana }
		u.s",
		"mana",
	);
}

#[test]
fn in_match() {
	check(
		indoc! {r#"
			Color :: enum { red green blue }
			c :: Color.green
			match c {
				Color.red => "r",
				Color.green => "g",
				else => "?",
			}
		"#},
		"g",
	);
}

#[test]
fn unknown_variant() {
	fail(
		["Color :: enum { red green blue }", "Color.purple"],
		"no variant `purple`",
	);
}

#[test]
fn shorthand_in_assignment() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			c := Color.green
			c = .red
			c
		"},
		"red",
	);
}

#[test]
fn shorthand_in_annotated_binding() {
	check(["Color :: enum { red green blue }", "c : Color : .blue", "c"], "blue");
}

#[test]
fn shorthand_in_comparison() {
	check(
		["Color :: enum { red green blue }", "c :: Color.red", "c == .red"],
		"true",
	);
	check(
		["Color :: enum { red green blue }", "c :: Color.red", "c != .blue"],
		"true",
	);
}

#[test]
fn shorthand_in_match() {
	check(
		indoc! {r#"
			Color :: enum { red green blue }
			c :: Color.green
			match c {
				.red => "r",
				.green => "g",
				else => "?",
			}
		"#},
		"g",
	);
}

#[test]
fn shorthand_in_struct_field() {
	check(
		"Stat :: enum { health mana stamina }
		User :: struct { s: Stat }
		u :: User.{ s = .mana }
		u.s",
		"mana",
	);
	check(
		"Stat :: enum { health mana stamina }
		User :: struct { s: Stat }
		u :: User.{ .stamina }
		u.s",
		"stamina",
	);
}

#[test]
fn shorthand_in_field_and_element_assignment() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			Pen :: struct { ink: Color }
			p := Pen.{}
			p.ink = .blue
			p.ink
		"},
		"blue",
	);
	check(
		indoc! {"
			Color :: enum { red green blue }
			cs := [Color.red]
			cs[0] = .green
			cs[0]
		"},
		"green",
	);
}

#[test]
fn shorthand_unknown_variant() {
	fail(
		["Color :: enum { red green blue }", "c :: Color.red", "c == .purple"],
		"no variant `purple`",
	);
}

#[test]
fn shorthand_without_context_errors() {
	fail(
		["Color :: enum { red green blue }", ".red"],
		"cannot infer the enum type",
	);
}

#[test]
fn duplicate_disc_rejected() {
	fail("E :: enum { a = 2, b, c = 2 }", "discriminant value `2`");
}

#[test]
fn auto_increment_from_explicit() {
	fail("E :: enum { a = 5, b, c = 6 }", "discriminant value `6`");
}

#[test]
fn const_expr_disc() {
	check(
		[
			"SHIFT :: 2",
			"Perm :: enum { ayy = 0 << SHIFT, bee = 1 << SHIFT, cee = (1 << SHIFT) | 1 }",
			r#""{ord(Perm.bee)} {ord(Perm.cee)}""#,
		],
		"4 5",
	);
}

#[test]
fn payload_construct() {
	check(
		[
			"Shape :: enum { point triangle(f64, f64, f64) }",
			"Shape.triangle.(3.0, 4.0, 5.0)",
		],
		"triangle.(3.0, 4.0, 5.0)",
	);
}

#[test]
fn payloadless_variant_of_boxed_enum() {
	check(["Opt :: enum { nope some(int) }", "Opt.nope"], "nope");
	check(["Opt :: enum { nope some(int) }", "o : Opt : .nope", "o"], "nope");
}

#[test]
fn payload_enum_default_is_first() {
	check(["Opt :: enum { nope some(int) }", "o: Opt", "o"], "nope");
}

#[test]
fn payload_empty_literal_is_default() {
	check(["Opt :: enum { nope some(int) }", "Opt.{}"], "nope");
}

#[test]
fn payload_int_cast_errors() {
	fail(
		["Opt :: enum { nope some(int) }", "int.(Opt.some.(1))"],
		"no backing value",
	);
}

#[test]
fn payload_field_type_mismatch() {
	fail(
		["Opt :: enum { nope some(int) }", "Opt.some.(3.0)"],
		"expected int, got float",
	);
}

#[test]
fn payload_wrong_arity() {
	fail(
		["Opt :: enum { nope some(int) }", "Opt.some.()"],
		"takes 1 field(s), got 0",
	);
}

#[test]
fn payload_match_binds_fields() {
	check(
		indoc! {r#"
			Opt :: enum { nope some(int) }
			o :: Opt.some.(7)
			match o {
				.some.(n) => n,
				.nope => -1,
			}
		"#},
		"7",
	);
}

#[test]
fn payload_match_fieldless_arm() {
	check(
		indoc! {r#"
			Opt :: enum { nope some(int) }
			o : Opt : .nope
			match o {
				.some.(n) => n,
				.nope => -1,
			}
		"#},
		"-1",
	);
}

#[test]
fn payload_match_multiple_fields() {
	check(
		indoc! {r#"
			Shape :: enum { rect(int, int) tri(int, int, int) }
			s :: Shape.rect.(3, 4)
			match s {
				.rect.(w, h) => w * h,
				.tri.(a, b, c) => a + b + c,
			}
		"#},
		"12",
	);
}

#[test]
fn shorthand_payload_construct() {
	check(
		[
			"Opt :: enum { nope some(int) }",
			"o : Opt : .some.(5)",
			"match o { .some.(n) => n, .nope => 0 }",
		],
		"5",
	);
}

#[test]
fn payload_eq() {
	check(
		["Opt :: enum { nope some(int) }", "Opt.some.(1) == Opt.some.(1)"],
		"true",
	);
	check(
		["Opt :: enum { nope some(int) }", "Opt.some.(1) == Opt.some.(2)"],
		"false",
	);
	check(["Opt :: enum { nope some(int) }", "Opt.nope == Opt.some.(1)"], "false");
	check(["Opt :: enum { nope some(int) }", "Opt.nope != Opt.some.(1)"], "true");
}

#[test]
fn payload_eq_string_field() {
	check(
		indoc! {r#"
			Msg :: enum { quit say(string) }
			Msg.say.("hi") == Msg.say.("hi")
		"#},
		"true",
	);
	check(
		indoc! {r#"
			Msg :: enum { quit say(string) }
			Msg.say.("hi") == Msg.say.("bye")
		"#},
		"false",
	);
}

#[test]
fn payload_ordering_rejected() {
	fail(
		["Opt :: enum { nope some(int) }", "Opt.some.(1) < Opt.some.(2)"],
		"claim `Ord` for `Opt` to define ordering",
	);
}

#[test]
fn struct_payload() {
	check(
		indoc! {r#"
			Point :: struct { x: int, y: int }
			Shape :: enum { dot rect(Point) }
			s :: Shape.rect.(Point.{ x = 3, y = 4 })
			match s {
				.rect.(p) => print(p),
				.dot => {}
			}
		"#},
		"Point.{x = 3, y = 4}",
	);
}

#[test]
fn enum_payload() {
	check(
		indoc! {r#"
			A :: enum { one two }
			B :: enum { wrap(A) empty }
			b :: B.wrap.(A.two)
			match b {
				.wrap.(a) => match a {
					.one => "one",
					.two => "two",
				},
				.empty => "none",
			}
		"#},
		"two",
	);
}

#[test]
fn struct_form_construct_and_match() {
	check(
		indoc! {r#"
			Shape :: enum {
				circle { radius: f64 }
				rectangle { width: f64, height: f64 }
				triangle(f64, f64, f64)
				point
			}
			s :: Shape.circle.{ radius = 5.0 }
			match s {
				.circle.{ radius } => radius * 2.0,
				.rectangle.{ width, height } => width * height,
				.triangle.(a, b, c) => a + b + c,
				.point => 0.0,
			}
		"#},
		"10.0",
	);
}

#[test]
fn struct_form_shorthand_and_rename() {
	check(
		indoc! {r#"
			Shape :: enum { circle { radius: f64 } rectangle { width: f64, height: f64 } }
			mk :: fn() Shape { .rectangle.{ width = 3.0, height = 4.0 } }
			match mk() {
				.rectangle.{ width = w, height } => w * height,
				else => 0.0,
			}
		"#},
		"12.0",
	);
}

#[test]
fn struct_form_zero_is_first_variant() {
	check(
		indoc! {r#"
			Shape :: enum { circle { radius: f64 } rectangle { width: f64, height: f64 } }
			match Shape.{} { .circle.{ radius } => radius, else => -1.0 }
		"#},
		"0.0",
	);
}

#[test]
fn struct_form_unknown_field() {
	fail(
		["S :: enum { circle { radius: f64 } }", "S.circle.{ r = 1.0 }"],
		"no field `r`",
	);
}

#[test]
fn struct_form_omitted_field_zeroes() {
	check(
		indoc! {r#"
			S :: enum { rect { w: f64, h: f64 } }
			s :: S.rect.{ h = 2.0 }
			match s { .rect.{ w, h } => w + h }
		"#},
		"2.0",
	);
}

#[test]
fn struct_form_takes_positional() {
	check(
		["S :: enum { circle { radius: f64 } }", "print(S.circle.{ 1.0 })"],
		"circle.{radius = 1.0}",
	);
}

#[test]
fn tuple_form_record_rejected() {
	fail(
		["S :: enum { tri(f64, f64) }", "S.tri.{ a = 1.0 }"],
		"takes positional fields",
	);
}

#[test]
fn alias_payload() {
	check(
		indoc! {"
			Meters :: f64
			Dist :: enum { unknown known(Meters) }
			d :: Dist.known.(5.0)
			match d {
				.known.(m) => m,
				.unknown => 0.0,
			}
		"},
		"5.0",
	);
}

#[test]
fn payload_unknown_type_rejected() {
	fail("A :: enum { wrap(NoSuchType) }", "unknown type");
}

#[test]
fn explicit_disc_default_is_first() {
	check(["E :: enum { a = 5, b c }", "x: E", "x"], "a");
}

#[test]
fn atom_coerces_in_annotated_binding() {
	check(["Color :: enum { red green blue }", "c : Color : :blue", "c"], "blue");
}

#[test]
fn atom_coerces_in_assignment() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			c := Color.green
			c = :red
			c
		"},
		"red",
	);
}

#[test]
fn atom_coerces_in_comparison() {
	check(
		["Color :: enum { red green blue }", "c :: Color.red", "c == :red"],
		"true",
	);
	check(["Color :: enum { red green blue }", "Color.blue == :blue"], "true");
}

#[test]
fn atom_coerces_in_struct_field() {
	check(
		indoc! {"
			Stat :: enum { health mana stamina }
			User :: struct { s: Stat }
			u :: User.{ s = :mana }
			u.s
		"},
		"mana",
	);
}

#[test]
fn atom_unknown_variant() {
	fail(
		["Color :: enum { red green blue }", "c : Color : :purple"],
		"no variant `purple`",
	);
}

#[test]
fn cast_to_int() {
	check(["Color :: enum { red green blue }", "int.(Color.blue)"], "2");
}

#[test]
fn cast_to_int_explicit_disc() {
	check(["Status :: enum { ok = 200, err = 500 }", "int.(Status.err)"], "500");
}

#[test]
fn backed_cast_to_backing() {
	check(["Status : u8 : enum { ok = 200, err = 250 }", "u8.(Status.ok)"], "200");
}

#[test]
fn backing_non_integer_errors() {
	fail("E : bool : enum { a }", "not an enum-able type");
}

#[test]
fn backing_out_of_range_errors() {
	fail("E : u8 : enum { a = 300 }", "out of range for its backing type");
}

#[test]
fn backing_with_payload_errors() {
	fail("E : u8 : enum { a some(int) }", "cannot have payload");
}

#[test]
fn ord_gives_discriminant() {
	check(["Color :: enum { red green blue }", "ord(Color.blue)"], "2");
}

#[test]
fn ord_on_payload_variant() {
	check(["Opt :: enum { nope some(int) }", "ord(Opt.some.(1))"], "1");
}

#[test]
fn str_method() {
	check(["Color :: enum { red green blue }", "Color.blue.str()"], "blue");
}

#[test]
fn no_such_method() {
	fail(
		["Color :: enum { red green blue }", "Color.red.hex()"],
		"has no method `hex`",
	);
}

#[test]
fn from_int_match() {
	check(
		["Color :: enum { red green blue }", "Color.from(1) or { Color.red }"],
		"green",
	);
}

#[test]
fn from_int_no_match() {
	check(
		["Color :: enum { red green blue }", "Color.from(9) or { Color.red }"],
		"red",
	);
}

#[test]
fn from_int_or_shorthand() {
	check(["Color :: enum { red green blue }", "Color.from(9) or .red"], "red");
}

#[test]
fn from_int_no_match_carries_error() {
	check(
		[
			"Color :: enum { red green blue }",
			"Color.from(9) or { print($)",
			"Color.red }",
		],
		["no matching variant", "red"],
	);
}

#[test]
fn from_str_match() {
	check(
		[
			"Color :: enum { red green blue }",
			r#"Color.from("blue") or { Color.red }"#,
		],
		"blue",
	);
}

#[test]
fn from_str_no_match() {
	check(
		[
			"Color :: enum { red green blue }",
			r#"Color.from("purple") or { print($)"#,
			"Color.red }",
		],
		["no matching variant", "red"],
	);
}

#[test]
fn from_atom_match() {
	check(
		["Color :: enum { red green blue }", "Color.from(:blue) or { Color.red }"],
		"blue",
	);
}

#[test]
fn from_atom_no_match() {
	check(
		[
			"Color :: enum { red green blue }",
			"Color.from(:purple) or { print($)",
			"Color.red }",
		],
		["no matching variant", "red"],
	);
}

#[test]
fn from_payload_zero_fills() {
	check(
		[
			"Shape :: enum { point triangle(f64, f64, f64) }",
			"Shape.from(1) or { Shape.point }",
		],
		"triangle.(0.0, 0.0, 0.0)",
	);
}

#[test]
fn from_wrong_type() {
	fail(
		["Color :: enum { red green blue }", "Color.from(true)"],
		"needs an int, string, or atom",
	);
}

#[test]
fn shorthand_coerces_in_fn_arg() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			name :: fn(c: Color) string { c.str() }
			name(.blue)
		"},
		"blue",
	);
}

#[test]
fn atom_coerces_in_fn_arg() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			name :: fn(c: Color) string { c.str() }
			name(:blue)
		"},
		"blue",
	);
}

#[test]
fn shorthand_coerces_in_if_tail_return() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			fav :: fn(pick: bool) Color {
				if pick { .blue } else { .red }
			}
			fav(true)
		"},
		"blue",
	);
}

#[test]
fn shorthand_coerces_in_match_tail_return() {
	check(
		indoc! {r#"
			Color :: enum { red green blue }
			fav :: fn(n: int) Color {
				match n {
					1 => .red,
					else => .blue,
				}
			}
			fav(9)
		"#},
		"blue",
	);
}

#[test]
fn shorthand_coerces_in_if_expr() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			c : Color : if false { .red } else { .blue }
			c
		"},
		"blue",
	);
}

#[test]
fn shorthand_coerces_in_match_expr() {
	check(
		indoc! {r#"
			Color :: enum { red green blue }
			n :: 9
			c : Color : match n {
				1 => .red,
				else => .blue,
			}
			c
		"#},
		"blue",
	);
}

#[test]
fn print_payloads() {
	check(
		indoc! {"
			Shape :: enum {
				point
				circle { radius: f64 }
				triangle(f64, f64, f64)
			}
			print(Shape.point)
			print(Shape.circle.{ radius = 5.0 })
			print(Shape.triangle.(3.0, 4.0, 5.0))
		"},
		["point", "circle.{radius = 5.0}", "triangle.(3.0, 4.0, 5.0)"],
	);
}

#[test]
fn backed_arrays_pack() {
	check(
		indoc! {"
			Status : u8 : enum { ok = 200, err = 250 }
			a := [Status.ok, Status.err]
			a[0] = Status.err
			a << Status.ok
			print(a)
			loop s in a { print(s) }
			print(Status.err in a)
			print(match a { [x, y, z] => z, else => Status.err })
			f: [3]Status
			f[1] = Status.err
			print(f[1])
			f[0] == Status.ok
		"},
		["[err, err, ok]", "err", "err", "ok", "true", "ok", "err", "true"],
	);
}

#[test]
fn string_backed_raws() {
	check(
		indoc! {r#"
			Suit : string : enum { hearts = "♥" spades = "♠" }
			print(string.(Suit.spades))
			print(string.(Suit.hearts))
			print(Suit.spades.str())
			print(ord(Suit.spades))
			print(Suit.hearts == Suit.hearts)
			a :: [Suit.spades, Suit.hearts]
			print(string.(a[1]))
			match Suit.spades { .spades => "s", else => "?" }
		"#},
		["♠", "♥", "spades", "1", "true", "♥", "s"],
	);
	check(["S : string : enum { a b }", "string.(S.b)"], "b");
}

#[test]
fn string_backed_errors() {
	fail(["S : string : enum { a b }", "int.(S.a)"], "cannot cast string");
	fail(r#"S :: enum { a = "x" }"#, "needs a string backing");
	fail(r#"S : string : enum { a = 2 }"#, "uses raw values");
	fail(r#"S : string : enum { a = "x" b = "x" }"#, "assigned more than once");
	fail(r#"S : string : enum { a b = "a" }"#, "assigned more than once");
}

#[test]
fn backed_array_signed_sextends() {
	check(
		indoc! {r#"
			Delta : i8 : enum { down = -3, up = 4 }
			a :: [Delta.up, Delta.down]
			d :: a[1]
			print(int.(d))
			print(match d { Delta.down => "yes", else => "no" })
			d == Delta.down
		"#},
		["-3", "yes", "true"],
	);
}

#[test]
fn method_on_receiver() {
	check(
		indoc! {r##"
			Color :: enum { red green blue }
			Color :< {
				hex :: fn(self) string {
					match self {
						.red => "#f00",
						.green => "#0f0",
						.blue => "#00f",
					}
				}
			}
			c :: Color.green
			c.hex()
		"##},
		"#0f0",
	);
}

#[test]
fn method_compares_self() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			Color :< { is_warm :: fn(self) bool { self == .red } }
			print(Color.red.is_warm())
			print(Color.blue.is_warm())
		"},
		["true", "false"],
	);
}

#[test]
fn method_on_payload_enum() {
	check(
		indoc! {r#"
			Shape :: enum { point triangle(f64, f64, f64) }
			Shape :< {
				perimeter :: fn(self) f64 {
					match self {
						.triangle.(a, b, c) => a + b + c,
						.point => 0.0,
					}
				}
			}
			Shape.triangle.(2.0, 3.0, 4.0).perimeter()
		"#},
		"9.0",
	);
}

#[test]
fn static_fill_beside_variants() {
	check(
		indoc! {"
			Color :: enum { red green blue }
			Color :< { primary :: fn() Color { .red } }
			print(Color.primary())
			print(Color.green)
		"},
		["red", "green"],
	);
}

#[test]
fn str_fill_overrides_derived() {
	check(
		indoc! {r#"
			E :: enum { a b }
			E :< { str :: fn(self) string { "custom" } }
			E.a.str()
		"#},
		"custom",
	);
}

#[test]
fn variant_holes_from_a_macro() {
	check(
		indoc! {r"
			def! :: fn() Ast {
				vs := [`A`, `B(int)`]
				`E :: enum { %{..vs} }`
			}
			def!()
			print(E.A)
			print(match E.B.(7) { .A => 0, .B.(n) => n })
		"},
		["A", "7"],
	);
}
