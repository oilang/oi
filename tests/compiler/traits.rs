use crate::helpers::*;

const ANIMAL_DOG: &str = indoc! {r#"
	Animal :: trait { speak : fn(self) string }
	Dog :: struct {}
	Dog : Animal < { speak :: fn(self) string { "woof" } }
"#};

const ANIMAL_DOG_KIND: &str = indoc! {r#"
	Animal :: trait { speak : fn(self) string }
	Dog :: struct { kind: string }
	Dog : Animal < { speak :: fn(self) string { "woof" } }
"#};

const ANIMAL_KIND: &str = indoc! {r#"
	Animal :: trait {
		kind: string
		speak : fn(self) string
	}
	Dog :: struct { kind: string }
	Dog : Animal < { speak :: fn(self) string { "woof" } }
"#};

#[test]
fn trait_def_and_impl() {
	let src = indoc! {r#"
		Animal :: trait {
			kind: string
			speak : fn(self) string
			shout :: fn(self) string { self.speak() + "!" }
		}
		Dog :: struct { kind: string }
		Dog : Animal < { speak :: fn(self) string { "woof" } }
		Dog.{ "Collie" }.speak()
	"#};
	check(src, "woof");
}

#[test]
fn marker_impl() {
	let src = indoc! {r#"
		Marker :: trait {}
		Dog :: struct { kind: string }
		Dog :< Marker
		Dog.{ "Collie" }.kind
	"#};
	check(src, "Collie");
}

#[test]
fn supertraits() {
	let src = indoc! {"
		Eq :: trait {}
		Ord : Eq : trait { cmp : fn(self, other: Self) int }
	"};
	check(src, "");

	let src = indoc! {"
		Foo :: trait {}
		Baz : Foo, Bar : trait {}
	"};
	check(src, "");

	let src = indoc! {"
		Eq :: trait {}
		Ord : Eq : trait {}
		X :: struct {}
		X :< Eq
		X :< Ord
		print(X is Ord)
	"};
	check(src, "true");

	fail(
		indoc! {"
			Eq :: trait {}
			Ord :: trait is Eq {}
			X :: struct {}
			X :< Ord
		"},
		"",
	);
}

#[test]
fn builtin_claims_std_ord() {
	check("print(int is Ord)", "true");
}

#[test]
fn default_methods() {
	let src = indoc! {r#"
		Animal :: trait {
			speak : fn(self) string
			shout :: fn(self) string { self.speak() + "!" }
		}
		Dog :: struct {}
		Dog : Animal < { speak :: fn(self) string { "woof" } }
		Dog.{}.shout()
	"#};
	check(src, "woof!");
	let src = indoc! {r#"
		Animal :: trait {
			speak : fn(self) string
			shout :: fn(self) string { self.speak() + "!" }
		}
		Dog :: struct {}
		Dog : Animal < {
			speak :: fn(self) string { "woof" }
			shout :: fn(self) string { "WOOF" }
		}
		Dog.{}.shout()
	"#};
	check(src, "WOOF");
}

#[test]
fn field_requirement_satisfied() {
	let src = indoc! {r#"
		Animal :: trait { kind: string }
		Dog :: struct { kind: string }
		Dog :< Animal
		Dog.{ "Collie" }.kind
	"#};
	check(src, "Collie");
}

#[test]
fn is_expression() {
	let src = indoc! {"
		Animal :: trait {}
		Dog :: struct {}
		Dog :< Animal
		D :: Dog
		print(Dog is Animal)
		print(Dog is not Animal)
		print(D is Animal)
	"};
	check(src, ["true", "false", "true"]);

	let src = indoc! {"
		Animal :: trait {}
		Cat :: struct {}
		print(Cat is Animal)
		print(Cat is not Animal)
	"};
	check(src, ["false", "true"]);
}

#[test]
fn implicit_trait() {
	let src = indoc! {"
		@implicit
		Fruit :: trait { seeds: bool }
		Kiwi :: struct { seeds := true }
		Bike :: struct { wheels := 2 }
		print(Kiwi is Fruit)
		print(Bike is Fruit)
	"};
	check(src, ["true", "false"]);
}

#[test]
fn is_on_a_trait_object() {
	let src = indoc! {r#"
		Animal :: trait {}
		Dog :: struct {}
		Dog :< Animal
		Cat :: struct {}
		Cat :< Animal
		a := Animal.(Dog.{})
		print(a is Dog, a is Cat, a is not Dog)
	"#};
	check(src, "true false false");
}

#[test]
fn is_on_an_error() {
	let src = indoc! {r#"
		Bad :: struct {}
		Bad : Error < { message :: fn(self) string { "bad" } }
		e := Error.(Bad.{})
		print(e is Bad)
	"#};
	check(src, "true");
}

#[test]
fn is_expression_unknown_type() {
	fail(
		indoc! {"
			Animal :: trait {}
			print(Ghost is Animal)
		"},
		"",
	);
}

#[test]
fn rejects_bad_impls() {
	fail(
		indoc! {r#"
			Dog :: struct {}
			Dog : Animal < { speak :: fn(self) string { "woof" } }
		"#},
		"",
	);
	fail(
		indoc! {r#"
			Animal :: trait { speak: fn(self) string }
			Dog :: struct {}
			Dog : Animal < {}
		"#},
		"",
	);
	fail(
		indoc! {r#"
			Animal :: trait { kind: string }
			Dog :: struct {}
			Dog :< Animal
		"#},
		"",
	);
}

#[test]
fn rejects_duplicate_trait() {
	fail(
		indoc! {"
			Animal :: trait { speak :: fn(self) string }
			Animal :: trait { bark :: fn(self) string }
		"},
		"",
	);
}

#[test]
fn allows_extra_helper_fills() {
	let src = indoc! {r#"
		Animal :: trait { speak: fn(self) string }
		Dog :: struct {}
		Dog : Animal < {
			speak :: fn(self) string { self.noise() }
			noise :: fn(self) string { "woof" }
		}
		Dog.{}.speak()
	"#};
	check(src, "woof");
}

#[test]
fn rejects_wrong_arity_impl() {
	fail(
		indoc! {r#"
			Animal :: trait { speak: fn(self) string }
			Dog :: struct {}
			Dog : Animal < { speak :: fn(self, loud: bool) string { "woof" } }
		"#},
		"wrong signature",
	);
}

#[test]
fn one_fill_satisfies_two_traits() {
	let src = indoc! {"
		A :: trait { f: fn(self) int }
		B :: trait { f: fn(self) int }
		S :: struct {}
		S : A, B < { f :: fn(self) int { 9 } }
		S.{}.f()
	"};
	check(src, "9");
}

#[test]
fn rejects_shared_fill_sig_mismatch() {
	fail(
		indoc! {"
			A :: trait { f: fn(self) int }
			B :: trait { f: fn(self) string }
			S :: struct {}
			S : A, B < { f :: fn(self) int { 9 } }
		"},
		"declares",
	);
}

#[test]
fn rejects_conflicting_defaults() {
	fail(
		indoc! {"
			A :: trait { f :: fn(self) int { 1 } }
			B :: trait { f :: fn(self) int { 2 } }
			S :: struct {}
			S :< A, B
		"},
		"takes default `f` from both",
	);
}

#[test]
fn own_fill_settles_default_conflict() {
	let src = indoc! {"
		A :: trait { f :: fn(self) int { 1 } }
		B :: trait { f :: fn(self) int { 2 } }
		S :: struct { f :: fn(self) int { 3 } }
		S :< A, B
		S.{}.f()
	"};
	check(src, "3");
}

#[test]
fn body_fill_discharges_bare_claim() {
	let src = indoc! {"
		A :: trait { f: fn(self) int }
		S :: struct { f :: fn(self) int { 7 } }
		S :< A
		S.{}.f()
	"};
	check(src, "7");
}

#[test]
fn rejects_duplicate_fill() {
	fail(
		indoc! {"
			S :: struct { f :: fn(self) int { 1 } }
			S :< { f :: fn(self) int { 2 } }
		"},
		"duplicate fill",
	);
}

#[test]
fn dyn_dispatch_zoo() {
	let src = indoc! {r#"
		Cat :: struct { legs: int, kind: string }
		Cat : Animal < { speak :: fn(self) string { "meow" } }
		zoo : []Animal : [ Dog.{ "collie" }, Cat.{ 4, "mau" } ]
		loop a in zoo { print("a " + a.kind + " says " + a.speak()) }
	"#};
	check([ANIMAL_KIND, src], ["a collie says woof", "a mau says meow"]);
}

const CAR_HORN: &str = indoc! {r#"
	Horn :: struct { kind: string }
	Horn : Animal < { speak :: fn(self) string { "honk" } }
	Car :: struct { Horn }
"#};

#[test]
fn via_delegation() {
	// via through embedded struct
	let src = indoc! {r#"
		Car :< Animal via Horn
		zoo : []Animal : [ Car.{ Horn = Horn.{ kind = "vroom" } } ]
		loop a in zoo { print("a {a.kind} says {a.speak()}") }
	"#};
	check([ANIMAL_KIND, CAR_HORN, src], "a vroom says honk");

	let over = r#"Car : Animal via Horn < { speak :: fn(self) string { "HONK HONK" } }
	Car.{ kind = "vroom" }.speak()"#;
	check([ANIMAL_KIND, CAR_HORN, over], "HONK HONK");

	// via through named field
	let named = indoc! {r#"
		Wagon :: struct { horn: Horn, kind: string }
		Wagon :< Animal via horn
		print(Wagon.{ Horn.{ "vroom" }, "wagon" }.speak())
	"#};
	check([ANIMAL_KIND, CAR_HORN, named], "honk");
}

#[test]
fn embedded_field_satisfies_requirement() {
	let src = indoc! {r#"
		Meta :: struct { kind: string, id: int }
		Enemy :: struct { Meta, hp: int }
		Enemy : Animal < { speak :: fn(self) string { "rawr" } }
		zoo : []Animal : [ Enemy.{ Meta = Meta.{ kind = "goblin" } } ]
		loop a in zoo { print("a {a.kind} says {a.speak()}") }
	"#};
	check([ANIMAL_KIND, src], "a goblin says rawr");
}

#[test]
fn trait_object_array_literal() {
	let src = indoc! {r#"
		Cat :: struct { kind: string }
		Cat : Animal < { speak :: fn(self) string { "meow" } }
		animals :: Animal.[ Dog.{ "collie" }, Cat.{ "mau" } ]
		loop a in animals { print("{a.kind}: {a.speak()}") }
	"#};
	check([ANIMAL_KIND, src], ["collie: woof", "mau: meow"]);
}

#[test]
fn dyn_trait_param_and_default() {
	let src = indoc! {r#"
		Animal :: trait {
			speak : fn(self) string
			shout :: fn(self) string { self.speak() + "!" }
		}
		Dog :: struct {}
		Cat :: struct {}
		Dog : Animal < { speak :: fn(self) string { "woof" } }
		Cat : Animal < { speak :: fn(self) string { "meow" } }
		greet :: fn(a: Animal) string { a.shout() }
		relay :: fn(a: Animal) string { greet(a) }
		print(greet(Dog.{}))
		print(relay(Cat.{}))
	"#};
	check(src, ["woof!", "meow!"]);
}

#[test]
fn trait_typed_struct_field() {
	let src = indoc! {r#"
		Pen :: struct { pet: Animal }
		p :: Pen.{ pet = Dog.{} }
		print(p.pet.speak())
	"#};
	check([ANIMAL_DOG, src], "woof");
}

#[test]
fn self_sig_static_dispatch_ok() {
	let src = indoc! {r#"
		Cloner :: trait { dup : fn(self) Self }
		Dog :: struct {}
		Dog : Cloner < { dup :: fn(self) Self { Dog.{} } }
		d :: Dog.{}.dup()
		print("cloned")
	"#};
	check(src, "cloned");
}

#[test]
fn rejects_non_object_safe_trait() {
	fail(
		indoc! {r#"
			Cloner :: trait { dup : fn(self) Self }
			Dog :: struct {}
			Dog : Cloner < { dup :: fn(self) Self { Dog.{} } }
			f :: fn(c: Cloner) string { "no" }
		"#},
		"",
	);
	fail(
		indoc! {r#"
			Eater :: trait { eat : fn(self, other: Self) string }
			Dog :: struct {}
			Dog : Eater < { eat :: fn(self, other: Self) string { "ate" } }
			pack :: Eater.[ Dog.{} ]
		"#},
		"",
	);
}

#[test]
fn trait_object_renders_concrete_struct() {
	let src = indoc! {r#"
		a : Animal : Dog.{ "collie" }
		print(a)
		print("says {a}")
		print(a.str())
	"#};
	check(
		[ANIMAL_DOG_KIND, src],
		[
			r#"Dog.{kind = "collie"}"#,
			r#"says Dog.{kind = "collie"}"#,
			r#"Dog.{kind = "collie"}"#,
		],
	);
}

#[test]
fn trait_object_uses_str_override() {
	let src = indoc! {r#"
		Dog :< { str :: fn(self) string { "a " + self.kind + " dog" } }
		a : Animal : Dog.{ "collie" }
		print(a)
	"#};
	check([ANIMAL_DOG_KIND, src], "a collie dog");
}

#[test]
fn array_of_trait_objects_renders() {
	let src = indoc! {r#"
		Cat :: struct { kind: string }
		Cat : Animal < { speak :: fn(self) string { "meow" } }
		print(Animal.[ Dog.{ "collie" }, Cat.{ "mau" } ])
	"#};
	check(
		[ANIMAL_DOG_KIND, src],
		r#"[Dog.{ kind = "collie" }, Cat.{ kind = "mau" }]"#,
	);
}

#[test]
fn trait_declared_str_dyn_dispatches() {
	let src = indoc! {r#"
		Animal :: trait { str : fn(self) string }
		Dog :: struct { kind: string }
		Dog : Animal < { str :: fn(self) string { "custom-" + self.kind } }
		a : Animal : Dog.{ "collie" }
		print(a)
		print(a.str())
	"#};
	check(src, ["custom-collie", "custom-collie"]);
}

#[test]
fn append_boxes_struct_literal_into_trait_array() {
	let src = indoc! {r#"
		Cat :: struct {}
		Cat : Animal < { speak :: fn(self) string { "meow" } }
		zoo: []Animal = [ Dog.{} ]
		zoo << Cat.{}
		loop a in zoo { print(a.speak()) }
	"#};
	check([ANIMAL_DOG, src], ["woof", "meow"]);
}

#[test]
fn rejects_non_implementing_trait_object() {
	let src = indoc! {"
		Rock :: struct {}
		a : Animal : Rock.{}
	"};
	fail([ANIMAL_DOG, src], "");
}

#[test]
fn fill_headers_come_from_the_trait() {
	let src = indoc! {"
		Dbl :: trait { dbl: fn(n: int) int }
		Foo :: struct {}
		Bar :: struct {}
		Foo : Dbl < { dbl :: fn(i) int { i * 2 } }
		Bar : Dbl < { dbl :: { $ * 2 } }
		print(Foo.dbl(3))
		print(Bar.dbl(4))
	"};
	check(src, ["6", "8"]);
}

#[test]
fn branches_box_into_the_expected_trait_object() {
	let src = indoc! {"
		Shape :: trait { area: fn(self) int }
		Sq :: struct { s: int }
		Sq : Shape < { area :: { self.s * self.s } }
		Ci :: struct { r: int }
		Ci : Shape < { area :: { 3 * self.r * self.r } }
		pick :: fn(big: bool) Shape { if big { Ci.{2} } else { Sq.{2} } }
		print(pick(true).area(), pick(false).area())
	"};
	check(src, "12 4");
}

#[test]
fn headerless_fill_binds_self() {
	let src = indoc! {"
		Shape :: trait { area: fn(self) int }
		Sq :: struct { s: int }
		Sq : Shape < { area :: { self.s * self.s } }
		print(Sq.{ 3 }.area())
	"};
	check(src, "9");
}

#[test]
fn rejects_headerless_fill_the_trait_does_not_declare() {
	let src = indoc! {"
		Dbl :: trait { dbl: fn(n: int) int }
		Foo :: struct {}
		Foo : Dbl < { nope :: { 1 } }
	"};
	fail(src, "no trait method `nope` supplies a signature");
}

#[test]
fn fill_and_default_satisfy_trait_fields() {
	let src = indoc! {r#"
		Animal :: trait {
			name: string
			greeting: string = "hi"
			greet :: fn(self) string { "{self.name} says {self.greeting}" }
		}
		Rock :: struct {}
		Rock : Animal < { name :: "Bob" }
		Rock.{}.greet()
	"#};
	check(src, "Bob says hi");
}

#[test]
fn dyn_field_from_const() {
	let src = indoc! {r#"
		Animal :: trait {
			name: string
			legs: int = 4
			depth: int = -7
			speak : fn(self) string
		}
		Cat :: struct { name: string }
		Cat : Animal < { speak :: fn(self) string { "meow" } }
		Rock :: struct {}
		Rock : Animal < {
			name :: "Bob"
			speak :: fn(self) string { "..." }
		}
		zoo : []Animal : [ Cat.{ "Tom" }, Rock.{} ]
		loop a in zoo { print("{a.name} {a.legs} {a.depth} {a.speak()}") }
	"#};
	check(src, ["Tom 4 -7 meow", "Bob 4 -7 ..."]);
}

#[test]
fn rejects_wrong_type_fill() {
	fail(
		indoc! {"
			Animal :: trait { name: string }
			Rock :: struct {}
			Rock : Animal < { name :: 5 }
		"},
		"must be a `string` literal to satisfy trait",
	);
	fail(
		indoc! {r#"
			Animal :: trait { greeting: string = "hi" }
			Rock :: struct { greeting: int }
			Rock :< Animal
		"#},
		"missing field",
	);
}

#[test]
fn a_trait_takes_type_arguments() {
	let src = indoc! {r#"
		Money :: struct { cents: int }
		Money : From[int] < { from :: fn(n: int) Self { Money.{ n * 100 } } }
		Money : From[string] < { from :: fn(s: string) Self { Money.{ int.try_from(s) or 0 } } }
		print(Money.from(5).cents)
		print(Money.from("7").cents)
	"#};
	check(src, ["500", "7"]);
}

#[test]
fn a_type_argument_fills_a_field_slot() {
	let src = indoc! {"
		Wrap[T] :: trait {
			value: T
			get : fn(self) T
		}
		Box :: struct { value: int }
		Box : Wrap[int] < { get :: fn(self) int { self.value } }
		print(Box.{ 7 }.get())
	"};
	check(src, "7");
	fail(
		indoc! {r#"
			Wrap[T] :: trait { value: T }
			Box :: struct { value: string }
			Box : Wrap[int] < {}
			print(Box.{ "x" }.value)
		"#},
		"missing field `value int`",
	);
}

#[test]
fn boxed_error_outlives_its_scope() {
	let src = indoc! {r#"
		Bad :: struct { line: int }
		Bad : Error < { message :: fn(self) string { "at " + self.line.str() } }
		parse :: fn() !int { return Bad.{ line = 4 } }
		print(parse() or { print($.message()); 0 })
	"#};
	check(src, ["at 4", "0"]);
}

#[test]
fn embedded_err_promotes_error_claim() {
	let src = indoc! {r#"
		Parse :: struct { Err, line: int }
		Parse :< { message :: fn(self) string { "parse error at line {self.line}" } }
		Io :: struct { Err }
		boom :: fn(n: int) !int {
			if n == 0 { return Parse.{ line = 4 } }
			return Io.{ Err = Err.{ msg = "disk on fire" } }
		}
		loop n in 0..2 { boom(n) or { print($.message()) 0 } }
	"#};
	check(src, ["parse error at line 4", "disk on fire"]);
}

#[test]
fn embedded_trait_object_promotes_its_claim() {
	let src = indoc! {r#"
		Io :: struct { Error }
		Ctx :: struct { Error, where: string }
		Ctx : Error < { message :: fn(self) string { "{self.where}: {self.Error.message()}" } }
		boom :: fn(n: int) !int {
			if n == 0 { return Io.{ Error = error("disk on fire") } }
			return Ctx.{ Error = error("disk on fire"), where = "save" }
		}
		loop n in 0..2 { boom(n) or { print($.message()) 0 } }
	"#};
	check(src, ["disk on fire", "save: disk on fire"]);
}
