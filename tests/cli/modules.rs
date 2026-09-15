use crate::common::{Project, Run, err, oi, ok};

#[test]
fn imports_work() {
	let p = Project::new()
		.file("main.oi", ["module main", "use foo", "print(foo.total())"])
		.file(
			"foo/lib.oi",
			[
				"module foo",
				"use bar",
				"P :: struct { x: int, y: int }",
				"sum :: fn(p: P) int { p.x + p.y }",
				"pub total :: fn() int { bar.twice(sum(P.{x = 2, y = 5})) }",
			],
		)
		.file("bar/lib.oi", ["module bar", "pub twice :: fn(n: int) int { n * 2 }"]);
	p.check("14");
}

#[test]
fn private_fn_rejected() {
	let p = Project::new()
		.file("main.oi", ["module main", "use foo", "print(foo.secret())"])
		.file("foo/lib.oi", ["module foo", "secret :: fn() int { 1 }"]);
	p.fail_with("private to module `foo`");
}

#[test]
fn wrong_module_decl_names_the_file() {
	let p = Project::new()
		.file("main.oi", ["module main", "use foo", "print(foo.hi())"])
		.file("foo/lib.oi", ["module bar", "pub hi :: fn() int { 1 }"]);
	let out = err(p.run());
	assert!(out.contains("foo/lib.oi"), "{out}");
	assert!(out.contains("module foo"), "{out}");
}

#[test]
fn import_cycle_rejected() {
	let p = Project::new()
		.file("main.oi", ["module main", "use a", "print(a.v())"])
		.file("a/m.oi", ["module a", "use b", "pub v :: fn() int { b.v() }"])
		.file("b/m.oi", ["module b", "use a", "pub v :: fn() int { a.v() }"]);
	p.fail_with("import cycle");
}

#[test]
fn duplicate_name_across_files_rejected() {
	let p = Project::new()
		.file("main.oi", ["module main", "use foo", "print(foo.hi())"])
		.file("foo/a.oi", ["module foo", "pub hi :: fn() int { 1 }"])
		.file("foo/b.oi", ["module foo", "hi :: fn() int { 2 }"]);
	p.fail_with("defined twice");
}

#[test]
fn import_alias() {
	let p = Project::new()
		.file("main.oi", ["module main", "f :: use foo", "print(f.hi())"])
		.file("foo/lib.oi", ["module foo", "pub hi :: fn() int { 7 }"]);
	p.check("7");

	let p = Project::new()
		.file("main.oi", ["module main", "f :: use foo", "print(foo.hi())"])
		.file("foo/lib.oi", ["module foo", "pub hi :: fn() int { 7 }"]);
	p.fail_with("foo");
}

#[test]
fn selective_import() {
	for main in [
		"use foo\nuse foo.{ hi }\nprint(hi() + foo.hi())",
		"use foo.hi\nprint(hi() + hi())",
		"use foo.{ h :: hi }\nprint(h() + h())",
	] {
		let p = Project::new()
			.file("main.oi", ["module main", main])
			.file("foo/lib.oi", ["module foo", "pub hi :: fn() int { 7 }"]);
		p.check("14");
	}
}

#[test]
fn selective_import_fails() {
	let p = Project::new()
		.file("main.oi", ["module main", "use foo.{ nope }", "print(nope())"])
		.file("foo/lib.oi", ["module foo", "pub hi :: fn() int { 7 }"]);
	p.fail_with("has no `nope`");

	let p = Project::new()
		.file("main.oi", ["module main", "use foo.{ secret }", "print(secret())"])
		.file("foo/lib.oi", ["module foo", "secret :: fn() int { 1 }"]);
	p.fail_with("private to module `foo`");
}

#[test]
fn type_import() {
	for (main, lib) in [
		(
			"use foo.{ P }\np := P.{ x = 3, y = 4 }\nprint(p.x + p.y)",
			"pub P :: struct { pub x: int, pub y: int }",
		),
		(
			"Q :: use foo.P\nsum :: fn(q: Q) int { q.x + q.y }\nprint(sum(Q.{ x = 3, y = 4 }))",
			"pub P :: struct { pub x: int, pub y: int }",
		),
		(
			"use foo.{ E }\ne := E.a\nprint(match e { E.a => 7, E.b => 0 })",
			"pub E :: enum { a, b }",
		),
		("use foo.{ Id }\nn: Id = 7\nprint(n)", "pub Id :: int"),
		("use foo.{ id }\nn: id = 7\nprint(n)", "pub id :: int"),
	] {
		let p = Project::new()
			.file("main.oi", ["module main", main])
			.file("foo/lib.oi", ["module foo", lib]);
		p.check("7");
	}
}

#[test]
fn type_import_fails() {
	for lib in ["P :: struct { x: int }", "P :: trait {}"] {
		let p = Project::new()
			.file("main.oi", ["module main", "use foo.{ P }"])
			.file("foo/lib.oi", ["module foo", lib]);
		p.fail_with("private to module `foo`");
	}
}

#[test]
fn member_visibility() {
	for (main, lib) in [
		// a private field read
		("use foo.{ make }\nprint(make().x)", "pub P :: struct { x: int }"),
		// a private field in a literal
		(
			"use foo.{ P }\nprint(P.{ x = 1 }.y)",
			"pub P :: struct { x: int, pub y: int }",
		),
		// a private field in a positional literal
		(
			"use foo.{ P }\nprint(P.{ 1, 2 }.y)",
			"pub P :: struct { x: int, pub y: int }",
		),
		// a private field in a match pattern
		(
			"use foo.{ P, make }\nmatch make() { P.{ x } => print(x), }",
			"pub P :: struct { x: int }",
		),
		// a private `:<` method call
		(
			"use foo.{ make }\nprint(make().hidden())",
			"pub P :: struct { x: int }\nP :< { hidden :: fn(self) int { 1 } }",
		),
	] {
		let p = Project::new()
			.file("main.oi", ["module main", main])
			.file("foo/lib.oi", ["module foo", lib, "pub make :: fn() P { P.{ x = 1 } }"]);
		p.fail_with("private to module `foo`");
	}

	let p = Project::new()
		.file(
			"main.oi",
			["module main", "use foo.{ P }", "print(P.{ x = 3 }.shown())"],
		)
		.file(
			"foo/lib.oi",
			[
				"module foo",
				"pub P :: struct { pub x: int }",
				"P :< { pub shown :: fn(self) int { self.x * 2 } }",
			],
		);
	p.check("6");

	let p = Project::new()
		.file(
			"main.oi",
			[
				"module main",
				"use foo.{ P, make }",
				"match make() { P.{ x } => print(x), }",
			],
		)
		.file(
			"foo/lib.oi",
			[
				"module foo",
				"pub P :: struct { pub x: int }",
				"pub make :: fn() P { P.{ x = 9 } }",
			],
		);
	p.check("9");
}

#[test]
fn traits_are_module_scoped() {
	let p = Project::new()
		.file(
			"main.oi",
			[
				"module main",
				"use foo.{ FooP :: P, FooT :: T }",
				"use bar.{ BarP :: P, BarT :: T }",
				"print(FooP is FooT, BarP is BarT)",
			],
		)
		.file(
			"foo/lib.oi",
			[
				"module foo",
				"pub T :: trait {}",
				"pub P :: struct { x: int }",
				"P : T < {}",
			],
		)
		.file(
			"bar/lib.oi",
			[
				"module bar",
				"pub T :: trait {}",
				"pub P :: struct { y: int }",
				"P : T < {}",
			],
		);
	p.check("true true");
}

#[test]
fn std_trait_claimed_without_import_in_module() {
	let p = Project::new()
		.file(
			"main.oi",
			["module main", "use foo.{ P }", "print((P.{ x = 2 } + P.{ x = 3 }).x)"],
		)
		.file(
			"foo/lib.oi",
			[
				"module foo",
				"pub P :: struct { pub x: int }",
				"P : Add < { add :: fn(self, other: P) P { P.{ x = self.x + other.x } } }",
			],
		);
	p.check("5");
}

#[test]
fn generic_fn_uses_local_type() {
	let p = Project::new()
		.file("main.oi", ["module main", "use foo", "print(foo.pack(7))"])
		.file(
			"foo/lib.oi",
			[
				"module foo",
				"P :: struct { x: int }",
				"pub pack[T] :: fn(v: T) int { P.{ x = 3 }.x + v }",
			],
		);
	p.check("10");
}

#[test]
fn generic_struct_in_module() {
	let p = Project::new()
		.file("main.oi", ["module main", "use foo", "print(foo.mk().v)"])
		.file(
			"foo/lib.oi",
			[
				"module foo",
				"pub Box[T] :: struct { pub v: T }",
				"pub mk :: fn() Box[int] { Box.{ v = 7 } }",
			],
		);
	p.check("7");
}

#[test]
fn type_reexport() {
	let p = Project::new()
		.file("main.oi", ["module main", "use mid.{ P }", "print(P.{ x = 7 }.x)"])
		.file("mid/lib.oi", ["module mid", "pub use base.P"])
		.file("base/lib.oi", ["module base", "pub P :: struct { pub x: int }"]);
	p.check("7");
}

#[test]
fn narrowed_import() {
	for main in [
		"io :: use foo.{ hi }\nprint(io.hi())",
		"io :: use foo.{ h :: hi }\nprint(io.h())",
	] {
		let p = Project::new()
			.file("main.oi", ["module main", main])
			.file("foo/lib.oi", ["module foo", "pub hi :: fn() int { 7 }"]);
		p.check("7");
	}
}

#[test]
fn narrowed_import_fails() {
	for (main, expected) in [
		("io :: use foo.{ hi }\nprint(io.yo())", "not part of"),
		("io :: use foo.{ nope }\nprint(io.nope())", "has no `nope`"),
	] {
		let p = Project::new().file("main.oi", ["module main", main]).file(
			"foo/lib.oi",
			["module foo", "pub hi :: fn() int { 7 }", "pub yo :: fn() int { 8 }"],
		);
		p.fail_with(expected);
	}
}

#[test]
fn reexport() {
	for (mid, call) in [
		("pub use base.hi", "mid.hi()"),
		("pub use base.{ hi }", "mid.hi()"),
		("pub use base.{ h :: hi }", "mid.h()"),
	] {
		let p = Project::new()
			.file("main.oi", ["module main", "use mid", &format!("print({call})")])
			.file("mid/lib.oi", ["module mid", mid])
			.file("base/lib.oi", ["module base", "pub hi :: fn() int { 7 }"]);
		p.check("7");
	}
}

#[test]
fn reexport_selected() {
	let p = Project::new()
		.file("main.oi", ["module main", "use mid.{ hi }", "print(hi())"])
		.file("mid/lib.oi", ["module mid", "pub use base.hi"])
		.file("base/lib.oi", ["module base", "pub hi :: fn() int { 7 }"]);
	p.check("7");
}

#[test]
fn reexport_chain() {
	let p = Project::new()
		.file("main.oi", ["module main", "use top", "print(top.hi())"])
		.file("top/lib.oi", ["module top", "pub use mid.hi"])
		.file("mid/lib.oi", ["module mid", "pub use base.hi"])
		.file("base/lib.oi", ["module base", "pub hi :: fn() int { 7 }"]);
	p.check("7");
}

#[test]
fn reexport_fails() {
	for (mid, expected) in [
		("use base.hi", "has no function `hi`"),
		("pub use base", "only item imports can be re-exported"),
		("pub io :: use base.{ hi }", "only item imports can be re-exported"),
	] {
		let p = Project::new()
			.file("main.oi", ["module main", "use mid", "print(mid.hi())"])
			.file("mid/lib.oi", ["module mid", mid])
			.file("base/lib.oi", ["module base", "pub hi :: fn() int { 7 }"]);
		p.fail_with(expected);
	}
}

#[test]
fn const_import() {
	for main in [
		"use foo\nprint(foo.name)",
		"use foo.{ name }\nprint(name)",
		"use foo.name\nprint(name)",
	] {
		let p = Project::new()
			.file("main.oi", ["module main", main])
			.file("foo/lib.oi", ["module foo", "pub name :: 7"]);
		p.check("7");
	}
}

#[test]
fn const_import_fails() {
	let p = Project::new()
		.file("main.oi", ["module main", "use foo.{ name }", "print(name)"])
		.file("foo/lib.oi", ["module foo", "name :: 7"]);
	p.fail_with("private to module `foo`");
}

#[test]
fn const_reexport() {
	let p = Project::new()
		.file("main.oi", ["module main", "use mid", "print(mid.name)"])
		.file("mid/lib.oi", ["module mid", "pub use base.name"])
		.file("base/lib.oi", ["module base", "pub name :: 7"]);
	p.check("7");
}

#[test]
fn module_cannot_call_main_private_fn() {
	let p = Project::new()
		.file(
			"main.oi",
			["module main", "secret :: fn() int { 1 }", "use foo", "print(foo.hi())"],
		)
		.file("foo/lib.oi", ["module foo", "pub hi :: fn() int { secret() }"]);
	p.fail_with("undefined function");
}

#[test]
fn module_fn_uses_builtins_and_prints() {
	let p = Project::new().file("main.oi", ["module main", "use foo", "foo.go()"]).file(
		"foo/lib.oi",
		["module foo", "pub go :: fn() { n: int = 3\nprint(n + 1) }"],
	);
	p.check("4");
}

#[test]
fn single_file_module() {
	let p = Project::new()
		.file("main.oi", ["module main", "use foo", "print(foo.hi())"])
		.file("foo.oi", ["module foo", "pub hi :: fn() int { 7 }"]);
	p.check("7");
}

#[test]
fn dir_wins_over_single_file_module() {
	let p = Project::new()
		.file("main.oi", ["module main", "use foo", "print(foo.hi())"])
		.file("foo.oi", ["module foo", "pub hi :: fn() int { 1 }"])
		.file("foo/lib.oi", ["module foo", "pub hi :: fn() int { 2 }"]);
	p.check("2");
}

#[test]
fn subdir_folds_into_module() {
	let p = Project::new()
		.file("main.oi", ["module main", "use foo", "print(foo.hi())"])
		.file("foo/lib.oi", ["module foo", "pub hi :: fn() int { gen() }"])
		.file("foo/api/gen.oi", ["module foo", "pub gen :: fn() int { 7 }"]);
	p.check("7");
}

#[test]
fn single_file_module_chains_import() {
	let p = Project::new()
		.file("main.oi", ["module main", "use foo", "print(foo.hi())"])
		.file("foo.oi", ["module foo", "use bar", "pub hi :: fn() int { bar.hi() }"])
		.file("bar.oi", ["module bar", "pub hi :: fn() int { 7 }"]);
	p.check("7");
}

#[test]
fn exec_resolves_imports_against_cwd() {
	let p = Project::new().file("foo/lib.oi", ["module foo", "pub hi :: fn() int { 99 }"]);
	let out = ok(oi(&["exec", "use foo\nprint(foo.hi())"]).current_dir(&p).run(None));
	assert_eq!(out, "99");
}

#[test]
fn oi_path_searches_extra_dirs() {
	let deps = Project::new().file("greet/lib.oi", ["module greet", "pub hi :: fn() int { 42 }"]);
	let entry = Project::new().file("main.oi", ["module main", "use greet", "print(greet.hi())"]);
	let out = ok(oi(&["run", "main.oi"])
		.current_dir(&entry)
		.env("OI_PATH", deps.as_ref())
		.run(None));
	assert_eq!(out, "42");
}
