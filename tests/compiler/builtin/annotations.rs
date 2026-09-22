use crate::helpers::*;

#[test]
fn nozero_structs() {
	check(
		indoc! {r#"
			@nozero
			Handle :: struct { fd: int }
			h := Handle.{ fd = 3 }
			print(h.fd)
		"#},
		"3",
	);
	check(
		indoc! {r#"
			@nozero
			Handle :: struct { fd: int }
			Conn :: struct { handle: Handle = Handle.{ fd = 7 } }
			c := Conn.{}
			print(c.handle.fd)
		"#},
		"7",
	);
}

#[test]
fn nozero_failures() {
	fail_with(
		indoc! {r#"
			@nozero
			Handle :: struct { fd: int }
			h: Handle
		"#},
		"`Handle` has no zero value",
	);
	fail_with(
		indoc! {r#"
			@nozero
			Handle :: struct { fd: int }
			Conn :: struct { handle: Handle }
			c: Conn
		"#},
		"`Handle` has no zero value",
	);
}
