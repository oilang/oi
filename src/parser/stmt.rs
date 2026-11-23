use super::{P, Parsers, Rec, Tokens, binder, binding, brace, bracket, ident, list1, loose_list, paren, spanned};
use crate::ast::{self, BinOp, Expr, Spanned, TypeExpr};
use crate::lexer::Token;

use chumsky::prelude::*;

// The destructuring patterns.
pub(super) fn patterns<'token, I: Tokens<'token>>() -> P<'token, I, Spanned<Expr>> {
	// pattern bindings
	let pat_name = spanned(ident()).map(|(n, s)| (Expr::Ident(n), s));

	// tuple patterns
	let tuple_pat =
		spanned(paren(loose_list(pat_name.clone().map(|e| (None::<String>, e))))).try_map(|(elems, espan), span| {
			if elems.len() < 2 {
				return Err(I::fault(span, "tuple destructuring needs at least 2 names"));
			}
			Ok((Expr::Tuple(elems), espan))
		});

	// struct patterns
	let struct_pat_field =
		ident()
			.then(just(Token::Assign).ignore_then(ident()).or_not())
			.map_with(|(field, local), ex| {
				let local = local.unwrap_or_else(|| field.clone());
				(Some(field), (Expr::Ident(local), ex.span()))
			});
	let struct_pat = spanned(ident())
		.then_ignore(just(Token::Dot))
		.then(brace(loose_list(struct_pat_field)))
		.map(|((name, nspan), fields)| (Expr::StructLit { name, type_args: vec![], fields }, nspan));
	// array patterns
	let array_pat = spanned(bracket(loose_list(pat_name.clone()))).try_map(|(elems, espan), span| {
		if elems.is_empty() {
			return Err(I::fault(span, "array destructuring needs at least 1 name"));
		}
		Ok((Expr::Array(elems), espan))
	});
	tuple_pat.or(struct_pat).or(array_pat).boxed()
}

// The binding grammar.
pub(super) fn binds<'token, I: Tokens<'token>>(
	p: &Parsers<'token, I>,
	value: P<'token, I, Spanned<Expr>>,
	pat: P<'token, I, Spanned<Expr>>,
) -> (P<'token, I, Spanned<Expr>>, P<'token, I, Spanned<Expr>>) {
	let bind_name = just(Token::Percent)
		.then_ignore(p.adjacent.clone())
		.ignore_then(
			ident()
				.map(|n| (format!("%{n}"), None))
				.or(brace(p.expr.clone()).map(|e| ("%".to_string(), Some(e)))),
		)
		.or(ident().map(|n| (n, None)))
		.boxed();

	let annot = p.annot.clone().validate(|t, _, emitter| {
		if let (TypeExpr::AnonStruct(_), s) = &t {
			emitter.emit(I::fault(*s, "an anonymous struct type can't be a binding's middle"));
		}
		t
	});

	let bind = bind_name
		.then(binding(annot, value.clone()))
		.map_with(|((name, binder), (mutable, typ, value)), ex| {
			let bind = (
				Expr::Bind {
					mutable,
					name,
					typ: typ.map(Box::new),
					value: value.map(Box::new),
				},
				ex.span(),
			);
			match binder {
				Some(b) => (Expr::UnquoteBind(Box::new(b), Box::new(bind)), ex.span()),
				None => bind,
			}
		})
		.boxed();

	let destructure = pat
		.then(
			binder([Token::Bind, Token::DoubleColon])
				.map(Some)
				.or(just(Token::Assign).to(None)),
		)
		.then(value)
		.map_with(|((pat, mutable), value), ex| {
			(
				Expr::PatBind {
					pat: Box::new(pat),
					mutable,
					value: Box::new(value),
				},
				ex.span(),
			)
		})
		.boxed();

	(bind, destructure)
}

// The statement grammar.
pub(super) fn stmt<'token, I: Tokens<'token>>(
	p: &Parsers<'token, I>,
	mut stmt: Rec<'token, I, Spanned<Expr>>,
	mut do_stmt: Rec<'token, I, Spanned<Expr>>,
	mut place: Rec<'token, I, Spanned<Expr>>,
	mut bind: Rec<'token, I, Spanned<Expr>>,
) {
	// tuples
	let values = list1(p.juxt_expr.clone())
		.map_with(|mut values, ex| match values.len() {
			1 => values.remove(0),
			_ => (Expr::Tuple(values.into_iter().map(|v| (None, v)).collect()), ex.span()),
		})
		.boxed();

	// compound assignment
	let assign_op = choice((
		just(Token::PlusEq).to(Some(BinOp::Add)),
		just(Token::MinusEq).to(Some(BinOp::Sub)),
		just(Token::StarStarEq).to(Some(BinOp::Pow)),
		just(Token::StarEq).to(Some(BinOp::Mul)),
		just(Token::SlashEq).to(Some(BinOp::Div)),
		just(Token::PercentEq).to(Some(BinOp::Mod)),
		just(Token::AmpEq).to(Some(BinOp::BitAnd)),
		just(Token::PipeEq).to(Some(BinOp::BitOr)),
		just(Token::TildeEq).to(Some(BinOp::BitXor)),
		just(Token::LtLtEq).to(Some(BinOp::Shl)),
		just(Token::GtGtEq).to(Some(BinOp::Shr)),
		just(Token::Assign).to(None),
	));
	let rhs = assign_op.then(values.clone());
	let fold = |op, lhs, value: Spanned<Expr>, span| match op {
		None => value,
		Some(op) => (Expr::Binary(op, Box::new((lhs, span)), Box::new(value)), span),
	};

	// unquote holes
	let hole = p.unquote.clone().try_map(|(e, span), _| match e {
		Expr::UnquoteExpr(inner) => Ok(("%".into(), Some(inner))),
		_ => Err(I::fault(span, "expected a name")),
	});

	// assignment
	let name = p.def_name.clone().map_with(|n, ex| (Expr::Ident(n), ex.span()));
	let mut assign = Recursive::declare();
	assign.define(
		(p.unquote.clone().or(name))
			.then(assign_op)
			.then(assign.clone().or(values.clone()))
			.map_with(move |((key, op), value), ex| {
				let value = Box::new(fold(op, key.0.clone(), value, ex.span()));
				let key = Box::new(key);
				(Expr::Assign { key, value }, ex.span())
			}),
	);

	// map deletion
	let map_delete = ident()
		.then_ignore(just(Token::Dot))
		.then_ignore(just(Token::Ident("delete".into())))
		.then(bracket(p.expr.clone()))
		.map_with(|(name, key), ex| (Expr::MapDelete { name, key: Box::new(key) }, ex.span()));

	// assignment through a field/index/deref chain
	let seg = just(Token::Dot)
		.ignore_then(
			p.def_name
				.clone()
				.or(select! { Token::Int(n) => n.to_string() })
				.map(|f| (f, None))
				.or(hole),
		)
		.map(|f| Some(Ok(f)))
		.or(bracket(p.expr.clone()).map(|i| Some(Err(i))))
		.or(just(Token::Caret).to(None));
	let view = spanned(p.type_expr.clone()).map(Box::new).then_ignore(just(Token::Dot));
	let view = (view.then(paren(loose_list(p.expr.clone()))))
		.map_with(|(target, args), ex| (Expr::Cast { target, args }, ex.span()));
	let root = spanned(ident().map(Expr::Ident).or(just(Token::Dollar).to(Expr::Dollar)));
	let target = (view.or(p.unquote.clone()).or(root))
		.foldl_with(seg.repeated().at_least(1), |lhs, seg, ex| {
			let e = match seg {
				None => Expr::Deref(Box::new(lhs)),
				Some(Ok((field, hole))) => {
					let tuple = Box::new(lhs);
					match hole {
						Some(binder) => Expr::UnquoteBind(binder, Box::new((Expr::Field { tuple, field }, ex.span()))),
						None => Expr::Field { tuple, field },
					}
				}
				Some(Err(index)) => Expr::Index {
					collection: Box::new(lhs),
					index: Box::new(index),
				},
			};
			(e, ex.span())
		})
		.boxed();
	let place_assign = target
		.clone()
		.then(rhs)
		.map_with(|(place, (op, value)), ex| ast::assign(place, op, value, ex.span()));

	// multiple assignment
	let multi_assign = target
		.or(spanned(ident().map(Expr::Ident)))
		.separated_by(just(Token::Comma))
		.at_least(2)
		.collect::<Vec<_>>()
		.then_ignore(just(Token::Assign))
		.then(values.clone())
		.map_with(|(places, value), ex| {
			let span = ex.span();
			let temp = |i| (Expr::Ident(format!("$m{i}")), span);
			let pat = (Expr::Tuple((0..places.len()).map(|i| (None, temp(i))).collect()), span);
			let (pat, value, mutable) = (Box::new(pat), Box::new(value), Some(false));
			let sets = places
				.into_iter()
				.enumerate()
				.map(|(i, place)| ast::assign(place, None, temp(i), span));
			let unpack = (Expr::PatBind { pat, mutable, value }, span);
			(Expr::Block(std::iter::once(unpack).chain(sets).collect()), span)
		});

	bind.define(binds(p, p.bind.clone().or(p.juxt_expr.clone()).boxed(), p.pat.clone()).0);

	// destructuring
	let names = spanned(ident().map(Expr::Ident))
		.or(p.pat.clone())
		.separated_by(just(Token::Comma))
		.at_least(2)
		.collect::<Vec<_>>()
		.map_with(|names, ex| (Expr::Tuple(names.into_iter().map(|n| (None, n)).collect()), ex.span()));
	let pat = names.clone().or(p.pat.clone()).boxed();
	let (list_bind, destructure) = binds(p, p.bind.clone().or(values.clone()).boxed(), pat);

	// declaration destructuring
	let decls = names
		.clone()
		.then(binding(p.annot.clone(), p.bind.clone().or(values.clone())))
		.map_with(|(pat, (mutable, typ, value)), ex| {
			let Expr::Tuple(names) = &pat.0 else { unreachable!() };
			let typ = typ.map(|(t, s)| (TypeExpr::Tuple(vec![(None, t); names.len()]), s));
			let (name, value) = (format!("${}", ex.span().start), value.map(Box::new));
			let value = Box::new((
				Expr::Bind {
					mutable,
					name,
					typ: typ.map(Box::new),
					value,
				},
				ex.span(),
			));
			let (pat, mutable) = (Box::new(pat), Some(mutable));
			(Expr::PatBind { pat, value, mutable }, ex.span())
		})
		.boxed();

	let return_list = just(Token::Return)
		.ignore_then(values.clone())
		.map_with(|value, ex| (Expr::Return(Some(Box::new(value))), ex.span()));

	let doc = select! { Token::Doc(text) => text.to_string() }
		.repeated()
		.at_least(1)
		.collect::<Vec<_>>()
		.map_with(|lines, ex| (Expr::Doc(lines), ex.span()))
		.then_ignore(just(Token::DocBreak).or_not());

	// unsafe assignment
	let unsafe_assign = just(Token::Unsafe)
		.ignore_then(place_assign.clone().or(assign.clone()))
		.map_with(|inner, ex| (Expr::Unsafe(Box::new(inner)), ex.span()));

	// statements
	place.define(
		destructure
			.or(decls)
			.or(list_bind)
			.or(unsafe_assign)
			.or(place_assign)
			.or(multi_assign)
			.or(assign.clone())
			.or(map_delete),
	);
	stmt.define(doc.or(place.clone()).or(p.macro_def.clone()).or(return_list.clone()).or(values));
	do_stmt.define(place.clone().or(return_list).or(p.juxt_expr.clone()));
}
