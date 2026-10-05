use super::{P, Parsers, Rec, brace, bracket, ident, loose_list, paren, spanned};
use crate::ast::{self, BinOp, Expr, Spanned, TypeExpr};
use crate::lexer::Token;

use chumsky::{input::ValueInput, prelude::*};

// The destructuring patterns, as `(pat_name, pat)`.
pub(super) fn patterns<'token, I>() -> (P<'token, I, Spanned<Expr>>, P<'token, I, Spanned<Expr>>)
where
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
	// pattern bindings
	let pat_name = spanned(ident()).map(|(n, s)| (Expr::Ident(n), s));

	// tuple patterns
	let tuple_pat =
		spanned(paren(loose_list(pat_name.clone().map(|e| (None::<String>, e))))).try_map(|(elems, espan), span| {
			if elems.len() < 2 {
				return Err(Rich::custom(span, "tuple destructuring needs at least 2 names"));
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
		.map(|((name, nspan), fields)| {
			(
				Expr::StructLit {
					name,
					type_args: vec![],
					fields,
				},
				nspan,
			)
		});
	// array patterns
	let array_pat = spanned(bracket(loose_list(pat_name.clone()))).try_map(|(elems, espan), span| {
		if elems.is_empty() {
			return Err(Rich::custom(span, "array destructuring needs at least 1 name"));
		}
		Ok((Expr::Array(elems), espan))
	});
	let pat = tuple_pat.or(struct_pat).or(array_pat).boxed();
	(pat_name.boxed(), pat)
}

// The binding grammar.
pub(super) fn binds<'token, I>(
	p: &Parsers<'token, I>,
	value: P<'token, I, Spanned<Expr>>,
	pat: P<'token, I, Spanned<Expr>>,
) -> (P<'token, I, Spanned<Expr>>, P<'token, I, Spanned<Expr>>)
where
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
	let bind_name = just(Token::Percent)
		.then_ignore(p.adjacent.clone())
		.ignore_then(
			ident()
				.map(|n| (format!("%{n}"), None))
				.or(brace(p.expr.clone()).map(|e| ("%".to_string(), Some(e)))),
		)
		.or(ident().map(|n| (n, None)))
		.boxed();

	let value_tail = just(Token::Bind)
		.to(true)
		.or(just(Token::DoubleColon).to(false))
		.then(value.clone())
		.map(|(mutable, value)| (mutable, None, Some(value)));
	let sandwich_tail = just(Token::Colon)
		.ignore_then(p.annot.clone().validate(|t, _, emitter| {
			if let (TypeExpr::AnonStruct(_), s) = &t {
				emitter.emit(Rich::custom(*s, "an anonymous struct type can't be a binding's middle"));
			}
			t
		}))
		.then(
			just(Token::Assign)
				.to(true)
				.or(just(Token::Colon).to(false))
				.then(value.clone())
				.or_not(),
		)
		.map(|(typ, tail)| match tail {
			Some((mutable, value)) => (mutable, Some(typ), Some(value)),
			None => (true, Some(typ), None),
		});
	let bind = bind_name
		.clone()
		.then(value_tail.or(sandwich_tail))
		.map_with(|((name, binder), (mutable, typ, value)), ex| {
			let bind = (
				Expr::Bind {
					mutable,
					name,
					typ,
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
			just(Token::Bind)
				.to(Some(true))
				.or(just(Token::DoubleColon).to(Some(false)))
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
pub(super) fn stmt<'token, I>(
	p: &Parsers<'token, I>,
	mut stmt: Rec<'token, I, Spanned<Expr>>,
	mut place: Rec<'token, I, Spanned<Expr>>,
	mut bind: Rec<'token, I, Spanned<Expr>>,
) where
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
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
	let rhs = assign_op.clone().then(p.juxt_expr.clone());
	let fold = |op, lhs, value: Spanned<Expr>, span| match op {
		None => value,
		Some(op) => (Expr::Binary(op, Box::new((lhs, span)), Box::new(value)), span),
	};

	// assignment
	let mut assign = Recursive::declare();
	assign.define(
		ident()
			.then(assign_op.clone())
			.then(assign.clone().or(p.juxt_expr.clone()))
			.map_with(move |((name, op), value), ex| {
				let value = fold(op, Expr::Ident(name.clone()), value, ex.span());
				(
					Expr::Assign {
						name,
						value: Box::new(value),
					},
					ex.span(),
				)
			}),
	);

	// return statements
	let ret_stmt = choice((
		just(Token::Return).ignore_then(p.juxt_expr.clone().or_not()),
		just(Token::BareReturn).to(None),
	))
	.map_with(|value, ex| (Expr::Return(value.map(Box::new)), ex.span()));

	// index assignment
	let index_assign = ident()
		.then(just(Token::Dot).ignore_then(ident()).or_not())
		.then(bracket(p.expr.clone()))
		.then(rhs.clone())
		.map_with(move |(((name, field), index), (op, value)), ex| {
			let lhs = Expr::Index {
				collection: Box::new(ast::place(&name, field.as_ref(), ex.span())),
				index: Box::new(index.clone()),
			};
			let value = fold(op, lhs, value, ex.span());
			(
				Expr::IndexAssign {
					name,
					field,
					index: Box::new(index),
					value: Box::new(value),
				},
				ex.span(),
			)
		});

	// map deletion
	let map_delete = ident()
		.then_ignore(just(Token::Dot))
		.then_ignore(just(Token::Ident("delete".to_string())))
		.then(bracket(p.expr.clone()))
		.map_with(|(name, key), ex| {
			(
				Expr::MapDelete {
					name,
					key: Box::new(key),
				},
				ex.span(),
			)
		});

	// deref assignment
	let deref_assign =
		ident()
			.then_ignore(just(Token::Caret))
			.then(rhs.clone())
			.map_with(move |(name, (op, value)), ex| {
				let lhs = Expr::Deref(Box::new((Expr::Ident(name.clone()), ex.span())));
				let value = Box::new(fold(op, lhs, value, ex.span()));
				(Expr::DerefAssign { name, value }, ex.span())
			});

	// field assignment
	let field_assign = ident()
		.then_ignore(just(Token::Dot))
		.then(ident().or(select! { Token::Int(n) => n.to_string() }))
		.then(rhs)
		.map_with(move |((name, field), (op, value)), ex| {
			let tuple = Box::new((Expr::Ident(name.clone()), ex.span()));
			let lhs = Expr::Field {
				tuple,
				field: field.clone(),
			};
			let value = fold(op, lhs, value, ex.span());
			(
				Expr::FieldAssign {
					name,
					field,
					value: Box::new(value),
				},
				ex.span(),
			)
		});

	let (b, destructure) = binds(p, p.juxt_expr.clone(), p.pat.clone());
	bind.define(b);

	let doc = select! { Token::Doc(text) => text }
		.repeated()
		.at_least(1)
		.collect::<Vec<_>>()
		.map_with(|lines, ex| (Expr::Doc(lines), ex.span()))
		.then_ignore(just(Token::DocBreak).or_not());

	let macro_stmt = p
		.dotted_name
		.clone()
		.then_ignore(p.adjacent.clone())
		.then_ignore(just(Token::Not))
		.then_ignore(p.adjacent.clone().not())
		.then(
			p.expr
				.clone()
				.then(p.same_line.clone().ignore_then(p.block_ast.clone()).or_not())
				.map(|(arg, body)| std::iter::once(arg).chain(body).collect::<Vec<_>>())
				.or(p.block_ast.clone().map(|body| vec![body])),
		)
		.map_with(|(name, args), ex| (Expr::MacroCall { name, args }, ex.span()));

	// statements that leave a place behind
	place.define(
		destructure
			.or(bind.clone())
			.or(field_assign)
			.or(deref_assign)
			.or(assign.clone())
			.or(index_assign)
			.or(map_delete),
	);

	let defer_stmt = just(Token::Defer)
		.ignore_then(just(Token::Or).or_not())
		.then(place.clone().or(p.juxt_expr.clone()))
		.map_with(|(or, body), ex| {
			(
				Expr::Defer {
					body: Box::new(body),
					on_err: or.is_some(),
				},
				ex.span(),
			)
		});

	// statements
	stmt.define(
		doc.or(ret_stmt)
			.or(defer_stmt)
			.or(place.clone())
			.or(p.macro_def.clone())
			.or(macro_stmt)
			.or(p.juxt_expr.clone()),
	);
}
