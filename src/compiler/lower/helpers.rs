use super::*;

// Create field Binds from idents.
pub(super) fn field_binds<'a>(
	elems: impl Iterator<Item = (&'a Spanned<Expr>, &'a Typ)>,
	base: i32,
	stride: i32,
) -> Result<Vec<Bind>, Diagnostic> {
	elems
		.enumerate()
		.filter(|(_, (e, _))| !matches!(&e.0, Expr::Ident(n) if n == "_"))
		.map(|(i, (e, t))| match &e.0 {
			Expr::Ident(n) => Ok((n.clone(), t.clone(), base + i as i32 * stride)),
			_ => Err(Diagnostic::new("patterns must bind names", e.1.into_range()).with_label("not a name")),
		})
		.collect()
}

// A struct pattern's field bindings.
pub(super) fn struct_pattern(
	fdefs: &[FieldDef],
	pname: &str,
	sname: &str,
	entries: &[(Option<String>, Spanned<Expr>)],
	span: Span,
) -> Result<Vec<Bind>, Diagnostic> {
	if pname != sname {
		let msg = format!("pattern is `{pname}` but subject is `{sname}`");
		return Err(Diagnostic::new(msg, span.into_range()).with_label("type mismatch"));
	}
	entries
		.iter()
		.filter(|(_, e)| !matches!(&e.0, Expr::Ident(n) if n == "_"))
		.map(|(fname, e)| {
			let Expr::Ident(local) = &e.0 else {
				return Err(
					Diagnostic::new("struct patterns must bind names", e.1.into_range()).with_label("not a name")
				);
			};
			let field = fname.as_deref().unwrap_or(local);
			let idx = fdefs.iter().position(|f| f.name == field).ok_or_else(|| {
				Diagnostic::new(format!("struct `{sname}` has no field `{field}`"), e.1.into_range())
					.with_label("no such field")
			})?;
			Ok((local.clone(), fdefs[idx].typ.clone(), idx as i32 * 8))
		})
		.collect()
}

// Enforce a closure not outliving its captures.
pub(super) fn closure_escape(typ: &Typ, span: Range<usize>, action: &str) -> Result<(), Diagnostic> {
	if let Typ::Closure(_, _, false) = typ {
		return Err(Diagnostic::new(
			format!("this closure borrows its captures, so it can't be {action}"),
			span,
		)
		.with_label("borrows its captures")
		.with_note("use `[move ...]` in the capture list to give the closure ownership"));
	}
	Ok(())
}

// Unwrap one level of `&T` so things can see throughva ref.
pub(super) fn peel(typ: &Typ) -> &Typ {
	match typ {
		Typ::Ref(inner) => inner,
		other => other,
	}
}

// The element type of an array.
pub(super) fn array_elem(typ: &Typ) -> &Typ {
	match typ {
		Typ::Array(e) | Typ::FixedArray(e, _) => e,
		Typ::Str => &Typ::UInt(8),
		_ => unreachable!("not an array type"),
	}
}

// The runtime tag used to hash a map key.
pub(super) fn map_key_tag(typ: &Typ) -> Option<runtime::Tag> {
	match typ {
		Typ::Bool => Some(runtime::Tag::Bool),
		Typ::Int(_) | Typ::ISize => Some(runtime::Tag::Int),
		Typ::UInt(_) | Typ::USize => Some(runtime::Tag::UInt),
		Typ::Float(_) => Some(runtime::Tag::Float),
		Typ::Str => Some(runtime::Tag::Str),
		Typ::Atom => Some(runtime::Tag::Str),
		_ => None,
	}
}

pub(super) fn uint_max(width: u16) -> i64 {
	if width >= 64 {
		u64::MAX as i64
	} else {
		((1u64 << width) - 1) as i64
	}
}

pub(super) fn int_min(width: u16) -> i64 {
	if width >= 64 { i64::MIN } else { -(1i64 << (width - 1)) }
}

pub(super) fn int_max(width: u16) -> i64 {
	if width >= 64 {
		i64::MAX
	} else {
		(1i64 << (width - 1)) - 1
	}
}

pub(super) fn unsigned_cc(icc: IntCC) -> IntCC {
	match icc {
		IntCC::SignedLessThan => IntCC::UnsignedLessThan,
		IntCC::SignedLessThanOrEqual => IntCC::UnsignedLessThanOrEqual,
		IntCC::SignedGreaterThan => IntCC::UnsignedGreaterThan,
		IntCC::SignedGreaterThanOrEqual => IntCC::UnsignedGreaterThanOrEqual,
		other => other,
	}
}

// Signed comparison codes for a `BinOp` comparison variant.
pub(super) fn cmp_cc(op: BinOp) -> (IntCC, FloatCC) {
	match op {
		BinOp::Eq => (IntCC::Equal, FloatCC::Equal),
		BinOp::Ne => (IntCC::NotEqual, FloatCC::NotEqual),
		BinOp::Lt => (IntCC::SignedLessThan, FloatCC::LessThan),
		BinOp::Gt => (IntCC::SignedGreaterThan, FloatCC::GreaterThan),
		BinOp::Le => (IntCC::SignedLessThanOrEqual, FloatCC::LessThanOrEqual),
		BinOp::Ge => (IntCC::SignedGreaterThanOrEqual, FloatCC::GreaterThanOrEqual),
		_ => unreachable!("non-comparison op in cmp_cc"),
	}
}
