use crate::common::Project;
use crate::helpers::*;

#[test]
fn comp_folds() {
	let src = indoc! {r#"
		use math
		f :: fn() int { 40 + 2 }
		PI :: comp 22.0 / 7.0
		V :: comp f()
		X :: comp {
			a := 10
			a * 2
		}
		S :: comp "hi" + " there"
		A :: comp math.abs(0 - 5)
		print(PI, V, X, S, A)
	"#};
	check(src, "3.142857142857143 42 20 hi there 5");
}

#[test]
fn comp_folds_aggregates() {
	let src = indoc! {r#"
		Point :: struct { x: int, y: int }
		Config :: struct { name: string, origin: Point }
		Method :: struct { name: string, argc: int }
		mk :: fn() Config { Config.{ "grid", Point.{ 3, 4 } } }
		methods :: fn() []Method { [Method.{ "add", 2 }, Method.{ "neg", 1 }] }
		C :: comp mk()
		MS :: comp methods()
		print("{C.name} {C.origin.x} {C.origin.y}")
		print("{MS.len} {MS[1].name} {MS[1].argc}")
	"#};
	check(src, ["grid 3 4", "2 neg 1"]);
}

#[test]
fn comp_is_actually_comptime() {
	let src = indoc! {r#"
		print("run")
		V :: comp { print("fold") 7 }
		print(V)
	"#};
	check(src, ["fold", "run", "7"]);
}

#[test]
fn comp_if_is_conditional_compilation() {
	let src = indoc! {r#"
		log :: fn(msg: string) {
			comp if 1 == 1 { print(msg) } else { missing() }
		}
		comp if 2 < 1 { missing() }
		log("hi")
		print(comp if 1 == 2 { missing() } else if 2 == 2 { 42 })
	"#};
	check(src, ["hi", "42"]);
}

#[test]
fn comp_site_in_a_fn_the_program_calls() {
	let src = indoc! {"
		f :: fn(x: int) {
			n :: comp { 1 + 1 }
			print(n + x)
		}
		main :: fn() { f(3) }
	"};
	check(src, "5");
}

#[test]
fn comp_yields_an_ast() {
	let src = indoc! {r#"
		comp { `print("spliced")` }
		comp {
			out: []Ast = []
			loop i in 1..4 { out << `print(%i)` }
			out
		}
	"#};
	check(src, ["spliced", "1", "2", "3"]);
}

#[test]
fn comp_rejects_unreifiable_type() {
	fail(r#"A :: comp ["a" = 1]"#, "can't use this type in `comp` yet");
}

#[test]
fn comp_consts_in_a_module() {
	Project::new()
		.file("main.oi", ["module main", "use util", "print(util.BEST)"])
		.file(
			"util/lib.oi",
			[
				"module util",
				"pick :: fn() int { 40 + 2 }",
				"pub BEST :: comp SEED * 21",
				"pub SEED :: comp pick() / 21",
			],
		)
		.check("42");
}

#[test]
fn comp_folds_in_an_imported_module() {
	Project::new()
		.file("main.oi", ["module main", "use util", "print(util.f())"])
		.file(
			"util/lib.oi",
			["module util", r#"pub f :: fn() int { comp { print("fold") 40 + 2 } }"#],
		)
		.check(["fold", "42"]);
}

#[test]
fn comp_seeds_module_statics_before_use() {
	Project::new()
		.file(
			"main.oi",
			[
				"use seenmod",
				r#"V :: comp { seenmod.bump("k") seenmod.seen }"#,
				"print(V)",
			],
		)
		.file(
			"seenmod.oi",
			[
				"module seenmod",
				r#"pub seen := """#,
				"pub bump :: fn(k: string) { seen = seen + k }",
			],
		)
		.check("k");
}

#[test]
fn struct_fields_are_typed_asts() {
	check(
		indoc! {r#"
			mirror! :: fn(s: Ast) Ast {
				f := s.items[0]
				line := "{f.name.str()} {f.typ.str()} {f.notes.len}"
				`
					print(%line)
					show :: fn(%{..s.items}) { print(speed) }
					show(3.5)
				`
			}
			mirror!(`Ship :: struct { speed: float @required }`)
		"#},
		["speed float 1", "3.5"],
	);
}

#[test]
fn fns_claims_and_notes_are_typed_asts() {
	check(
		indoc! {r#"
			export :: struct { hint: string = "" }
			note! :: fn(s: Ast) Ast {
				n := s.items[0].notes[0]
				line := "{n.name.str()} {n.items[0].str()}"
				`
					print(%line)
					%s
				`
			}
			api! :: fn(c: Ast) Ast {
				m := c.items[0]
				line := "{c.name.str()} {m.name.str()} {m.items.len} {m.typ.str()}"
				`
					print(%line)
					%c
				`
			}
			@note!
			Foo :: struct { speed: float @export.{"spd"} }
			@api!
			Foo :< { ready :: fn(self) int { 1 } }
			print(Foo.{speed = 3.5}.ready())
		"#},
		["export spd", "Foo ready 1 int", "1"],
	);
}

#[test]
fn type_info_reflects_a_definition() {
	check(
		indoc! {r#"
			Point :: struct { x: int, y: []float @required }
			Color :: enum { red, green, blue }
			comp {
				t := type_info(Point)
				print(t.name.str(), t.items[0].name.str(), t.items[0].typ.str(), t.items[1].notes.len)
				print(type_info(Color).items[2].str())
			}
		"#},
		["Point x int 1", "blue"],
	);
}

#[test]
fn macros_and_comp_share_one_stage0() {
	let src = indoc! {"
		twice! :: fn(e: Ast) Ast { `%e + %e` }
		SEED :: 21
		print(twice!(SEED))
		print(comp SEED * 2)
		print(comp SEED - 1)
	"};
	check(src, ["42", "42", "20"]);
}
