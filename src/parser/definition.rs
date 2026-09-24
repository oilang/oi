use super::{P, Rec, brace, bracket, ident, loose_list, paren, shadow_params, spanned, types};
use crate::ast::{Access, BinOp, Capture, Expr, MatchArm, Param, Span, Spanned, TypeExpr, record_args};
use crate::lexer::Token;

use chumsky::{
	input::ValueInput,
	pratt::{infix, left, postfix, prefix, right},
	prelude::*,
};

// field/tuple/method access
enum Dot {
	Fields(Vec<String>),
	Method(String, Vec<Spanned<TypeExpr>>, Vec<Spanned<Expr>>),
}

fn pipe_step((e, span): Spanned<Expr>) -> Spanned<Expr> {
	match e {
		Expr::Ident(name) => (
			Expr::Call {
				name,
				type_args: vec![],
				args: vec![(Expr::Dollar, span)],
			},
			span,
		),
		Expr::Propagate(inner) => (Expr::Propagate(Box::new(pipe_step(*inner))), span),
		e => (e, span),
	}
}

fn range(start: Spanned<Expr>, end: Option<Spanned<Expr>>, inclusive: bool, span: Span) -> Spanned<Expr> {
	let (start, end) = (Box::new(start), end.map(Box::new));
	(Expr::Range { start, end, inclusive }, span)
}

fn pipe(value: Spanned<Expr>, step: Spanned<Expr>, span: Span) -> Spanned<Expr> {
	(
		Expr::Pipe {
			value: Box::new(value),
			step: Box::new(pipe_step(step)),
		},
		span,
	)
}

// A juxtaposed leading literal and/or trailing fn.
type Juxt = (Option<Spanned<Expr>>, Option<Spanned<Expr>>);

// Wrap a value and `or` body into an `OrElse`.
fn or_else((value, body): (Spanned<Expr>, Option<Vec<Spanned<Expr>>>), span: Span) -> Spanned<Expr> {
	match body {
		Some(body) => (
			Expr::OrElse {
				value: Box::new(value),
				body,
			},
			span,
		),
		None => value,
	}
}

// Parsers used by the definition grammar.
pub(super) struct DefinitionParsers<'token, I>
where
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
	pub(super) access: P<'token, I, Access>,
	pub(super) dotted_name: P<'token, I, String>,
	pub(super) same_line: P<'token, I, ()>,
	pub(super) adjacent: P<'token, I, ()>,
	pub(super) unquote: P<'token, I, Spanned<Expr>>,
	pub(super) annotation: P<'token, I, Spanned<Expr>>,
	pub(super) type_expr: P<'token, I, TypeExpr>,
	pub(super) params: P<'token, I, (Vec<Param>, bool)>,
	pub(super) ret: P<'token, I, Option<Spanned<TypeExpr>>>,
	pub(super) pat: P<'token, I, Spanned<Expr>>,
	pub(super) pat_name: P<'token, I, Spanned<Expr>>,
	pub(super) lit_path: P<'token, I, String>,
	pub(super) block_ast: P<'token, I, Spanned<Expr>>,
	pub(super) place: P<'token, I, Spanned<Expr>>,
	pub(super) stmt: P<'token, I, Spanned<Expr>>,
	pub(super) item: P<'token, I, Spanned<Expr>>,
	pub(super) block: P<'token, I, Vec<Spanned<Expr>>>,
	pub(super) expr: P<'token, I, Spanned<Expr>>,
}

// The core expr/atom/pratt grammar.
pub(super) fn definition<'token, I>(
	p: DefinitionParsers<'token, I>,
	binds: impl FnOnce(
		P<'token, I, Spanned<Expr>>,
		P<'token, I, Spanned<Expr>>,
	) -> (P<'token, I, Spanned<Expr>>, P<'token, I, Spanned<Expr>>),
	mut header_expr: Rec<'token, I, Spanned<Expr>>,
	mut header_cond: Rec<'token, I, Spanned<Expr>>,
	mut juxt_expr: Rec<'token, I, Spanned<Expr>>,
) -> P<'token, I, Spanned<Expr>>
where
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
	let dot = || just(Token::Dot).or(just(Token::SpaceDot));

	let literal = select! {
		Token::Bool(b) => Expr::Bool(b),
		Token::Int(n) => Expr::Int(n),
		Token::Float(s) => Expr::Float(s.parse().unwrap()),
		Token::String(s) => Expr::String(s),
		Token::Atom(name) => Expr::Atom(name),
		Token::Dollar => Expr::Dollar,
	};

	// arg mods
	let mod_arg = p
		.access
		.clone()
		.then(p.expr.clone())
		.map_with(|(a, e), ex| (Expr::ArgMod(a, Box::new(e)), ex.span()));
	// named args collect into one trailing record arg
	let named_arg = ident()
		.map_with(|n, ex| (Expr::Ident(n), ex.span()))
		.then_ignore(just(Token::Assign))
		.then(mod_arg.clone().or(p.expr.clone()))
		.map(|(key, value)| (Some(key), value));
	// variable vs. call vs. struct literal
	let args = paren(
		named_arg
			.or(mod_arg.or(p.expr.clone()).map(|e| (None, e)))
			.separated_by(just(Token::Comma))
			.allow_trailing()
			.collect::<Vec<_>>(),
	)
	.validate(|elems, ex, emitter| {
		let mut args = Vec::new();
		let mut named = Vec::new();
		for (key, value) in elems {
			match key {
				Some(key) => named.push((key, value)),
				None if named.is_empty() => args.push(value),
				None => emitter.emit(Rich::custom(ex.span(), "positional args go before named args")),
			}
		}
		if !named.is_empty() {
			args.push((Expr::Record(named), ex.span()));
		}
		args
	})
	.boxed();

	// named or positional field entry
	let struct_field_entry = ident().then_ignore(just(Token::Assign)).or_not().then(p.expr.clone());
	let struct_body = brace(loose_list(struct_field_entry.clone()));

	// explicit generic types
	let call_type_args = bracket(
		spanned(p.type_expr.clone())
			.separated_by(just(Token::Comma))
			.at_least(1)
			.collect::<Vec<_>>(),
	);

	// struct literals
	let struct_lit = p
		.lit_path
		.clone()
		.then(call_type_args.clone().or_not())
		.or_not()
		.then_ignore(dot())
		.then(struct_body.clone())
		.map(|(head, fields)| {
			let (name, type_args) = head.unwrap_or_default();
			Expr::StructLit {
				name,
				type_args: type_args.unwrap_or_default(),
				fields,
			}
		});

	let foreign_lit = just(Token::Foreign).to(Expr::Foreign);

	let ref_lit = just(Token::Amp).ignore_then(p.expr.clone()).try_map(|e, span| match &e.0 {
		Expr::StructLit { .. } => Ok(Expr::Ref(Box::new(e))),
		_ => Err(Rich::custom(
			span,
			"only a struct literal can be boxed into a reference yet",
		)),
	});

	let call_tail = call_type_args.clone().or_not().then(args.clone());
	let var_or_call = ident().then(call_tail.or_not()).map(|(name, call)| match call {
		Some((type_args, args)) => Expr::Call {
			name,
			type_args: type_args.unwrap_or_default(),
			args,
		},
		None => Expr::Ident(name),
	});

	// leaf atoms pair themselves with their span
	let leaf = spanned(literal.or(foreign_lit).or(ref_lit).or(struct_lit).or(var_or_call)).boxed();

	// record entries
	let key = select! {
		Token::Ident(name) => Expr::Ident(name),
		Token::Int(n) => Expr::Int(n),
		Token::String(s) => Expr::String(s),
		Token::Atom(a) => Expr::Atom(a),
	};
	let keyed = spanned(key).then(just(Token::Assign).ignore_then(p.expr.clone()));
	// enum shorthand
	let brace_payload = struct_body.clone().map_with(|fs, ex| record_args(fs, ex.span()));
	let payload = dot().ignore_then(args.clone().or(brace_payload)).boxed();
	let enum_shorthand = dot()
		.ignore_then(select! { Token::Ident(v) => v })
		.then(payload.clone().or_not())
		.map_with(|(variant, args), ex| {
			let args = args.unwrap_or_default();
			(Expr::EnumShorthand { variant, args }, ex.span())
		})
		.boxed();

	// a lexer error token
	let bad = select! { Token::Error(text) => text }
		.try_map(|text, span| Err(Rich::custom(span, format!("unexpected character `{text}`"))));

	// grouping before tuple rule to avoid making 1ples
	let group = paren(p.place.clone().or(p.expr.clone()));

	// tuple literals
	let tuple = paren(loose_list(struct_field_entry)).map_with(|elems, ex| (Expr::Tuple(elems), ex.span()));

	// dot array literals
	let dot_array = p
		.type_expr
		.clone()
		.or_not()
		.then_ignore(dot())
		.then(bracket(loose_list(p.expr.clone())))
		.map_with(|(elem, elems), ex| (Expr::DotArray(elem.map(|t| (t, ex.span())), elems), ex.span()));

	// casting
	let cast = spanned(p.type_expr.clone())
		.then_ignore(just(Token::Dot))
		.then(paren(loose_list(p.expr.clone())))
		.map_with(|(target, args), ex| (Expr::Cast { target, args }, ex.span()));

	// dot tuple literals
	let dot_tuple = dot()
		.ignore_then(paren(loose_list(p.expr.clone())))
		.map_with(|elems, ex| (Expr::DotTuple(elems), ex.span()));

	// inline macro calls
	let macro_call = p
		.dotted_name
		.clone()
		.then_ignore(p.adjacent.clone())
		.then_ignore(just(Token::Not))
		.then_ignore(p.adjacent.clone())
		.then(paren(loose_list(p.expr.clone())))
		.map_with(|(name, args), ex| (Expr::MacroCall { name, args }, ex.span()));

	// map literals
	let map_entry = p.expr.clone().then_ignore(just(Token::Assign)).then(p.expr.clone());
	let map = bracket(
		map_entry
			.separated_by(just(Token::Comma).or_not())
			.allow_trailing()
			.at_least(1)
			.collect::<Vec<_>>(),
	)
	.map_with(|entries, ex| (Expr::Map(entries), ex.span()));

	let array = bracket(loose_list(p.expr.clone())).map_with(|elems, ex| (Expr::Array(elems), ex.span()));

	// match patterns
	let bind = ident().map_with(|n, ex| ((Expr::Ident(n.clone()), ex.span()), (Expr::Ident(n), ex.span())));
	let struct_pat = dot()
		.ignore_then(select! { Token::Ident(v) => v })
		.then_ignore(dot())
		.then(brace(loose_list(keyed.clone().or(bind))))
		.map_with(|(variant, es), ex| {
			let args = vec![(Expr::Record(es), ex.span())];
			(Expr::EnumShorthand { variant, args }, ex.span())
		});
	// container types
	let type_pat = p
		.type_expr
		.clone()
		.filter(|t| matches!(t, TypeExpr::Array(_) | TypeExpr::Map(..) | TypeExpr::FixedArray(..)))
		.map_with(|t, ex| (Expr::TypePat(t), ex.span()));
	let match_pat = struct_pat.or(type_pat).or(p.expr.clone()).boxed();

	let if_expr = recursive(|if_expr| {
		just(Token::If)
			.ignore_then(header_cond.clone())
			.then(p.block.clone())
			.then(
				just(Token::Else)
					.ignore_then(if_expr.map(|e| vec![e]).or(p.block.clone()))
					.or_not(),
			)
			.map_with(|((cond, then), els), ex| {
				(
					Expr::If {
						cond: Box::new(cond),
						then,
						els,
					},
					ex.span(),
				)
			})
	})
	.boxed();

	// loops
	let loop_expr = just(Token::Loop)
		.ignore_then(
			p.block
				.clone()
				.map(|body| (None, body))
				.or(header_cond.clone().map(Some).then(p.block.clone()))
				.or(header_expr.clone().map(|e| (None, vec![e]))),
		)
		.map_with(|(cond, body), ex| {
			(
				Expr::Loop {
					cond: cond.map(Box::new),
					body,
				},
				ex.span(),
			)
		})
		.boxed();

	let for_expr = just(Token::Loop)
		.ignore_then(p.pat.clone().or(p.pat_name))
		.then_ignore(just(Token::In))
		.then(header_expr.clone().map(Box::new))
		.then(p.block.clone())
		.map_with(|((pat, iter), body), ex| {
			(
				Expr::For {
					pat: Box::new(pat),
					iter,
					body,
				},
				ex.span(),
			)
		})
		.boxed();
	let break_expr = just(Token::Break)
		.ignore_then(p.expr.clone().or_not())
		.map_with(|v, ex| (Expr::Break(v.map(Box::new)), ex.span()));
	let continue_expr = just(Token::Continue).map_with(|_, ex| (Expr::Continue, ex.span()));

	// match expression
	let binding = ident().then_ignore(just(Token::At)).or_not();
	let arm_end = choice((
		just(Token::Comma).ignored(),
		just(Token::RBrace).rewind().ignored(),
		just(Token::Else).rewind().ignored(),
		just(Token::Backtick).rewind().ignored(),
	));
	let arm_body = p
		.block
		.clone()
		.then_ignore(just(Token::Comma).or_not())
		.or(juxt_expr.clone().map(|e| vec![e]).then_ignore(arm_end));
	let match_arm = binding
		.then(
			match_pat
				.clone()
				.separated_by(just(Token::Comma))
				.allow_trailing()
				.at_least(1)
				.collect::<Vec<_>>(),
		)
		.then_ignore(just(Token::FatArrow))
		.then(arm_body.clone())
		.map(|((binding, patterns), body)| MatchArm {
			binding,
			patterns,
			body,
		})
		.boxed();
	let arm_spread = p.unquote.clone().map(|e| MatchArm {
		body: vec![e],
		..Default::default()
	});
	let match_expr = just(Token::Match)
		.ignore_then(header_expr.clone())
		.then(brace(
			match_arm.clone().or(arm_spread).repeated().collect::<Vec<_>>().then(
				just(Token::Else)
					.ignore_then(just(Token::FatArrow))
					.ignore_then(arm_body)
					.or_not(),
			),
		))
		.map_with(|(subject, (arms, else_body)), ex| {
			(
				Expr::Match {
					subject: Box::new(subject),
					arms,
					else_body,
				},
				ex.span(),
			)
		})
		.boxed();

	// ast literals
	let quote = match_arm
		.map_with(|a, ex| (Expr::Arm(a), ex.span()))
		.or(p.item.clone())
		.or(p.stmt.clone())
		.repeated()
		.at_least(1)
		.collect::<Vec<_>>()
		.delimited_by(just(Token::Backtick), just(Token::Backtick))
		.map_with(|stmts, ex| (Expr::Quote(stmts), ex.span()));

	let comp_expr = just(Token::Comp)
		.ignore_then(header_expr.clone())
		.map_with(|inner, ex| (Expr::Comp(Box::new(inner)), ex.span()))
		.boxed();
	let unsafe_expr = just(Token::Unsafe)
		.ignore_then(p.expr.clone())
		.map_with(|inner, ex| (Expr::Unsafe(Box::new(inner)), ex.span()))
		.boxed();

	// anonymous functions
	let capture = ident();
	let capture = just(Token::Move)
		.ignore_then(capture)
		.map(Capture::Move)
		.or(just(Token::Mut).ignore_then(capture).map(Capture::Mut))
		.or(capture.map(Capture::ReadOnly));
	let captures = bracket(capture.separated_by(just(Token::Comma)).allow_trailing().collect::<Vec<_>>());
	let anon_fn = just(Token::Fn)
		.ignore_then(captures.or_not())
		.then(p.params.clone().or_not())
		.then(p.ret.clone())
		.then(p.block.clone())
		.map_with(|(((captures, params), ret), body), ex| {
			let (params, tuple) = params.unwrap_or((vec![], true));
			let body = shadow_params(&params, body);
			(
				Expr::AnonFn {
					captures,
					params,
					params_tuple: tuple,
					ret,
					body,
				},
				ex.span(),
			)
		})
		.boxed();

	// atoms
	let atom = choice((
		dot_array,
		cast,
		dot_tuple,
		macro_call,
		quote,
		leaf,
		p.unquote.clone(),
		enum_shorthand.clone(),
		group,
		tuple,
		map,
		array,
		p.block_ast.clone(),
		if_expr,
		match_expr,
		comp_expr,
		unsafe_expr,
		for_expr,
		loop_expr,
		break_expr,
		continue_expr,
		anon_fn.clone(),
		bad,
	))
	.boxed();

	// field/tuple/method access
	let access = choice((
		select! { Token::Int(n) => Dot::Fields(vec![n.to_string()]) },
		// NOTE: chained tuple access like `x.0.1` lexes `0.1` as a float, hence the split
		select! { Token::Float(s) => Dot::Fields(s.split('.').map(String::from).collect()) },
		// `[T]` is type args or subscript, based on whether a call follows
		ident()
			.then(call_type_args.or_not().then(args.clone()).or_not())
			.map(|(name, call)| match call {
				Some((type_args, args)) => Dot::Method(name, type_args.unwrap_or_default(), args),
				None => Dot::Fields(vec![name]),
			}),
	))
	.boxed();

	// array subscripts
	let subscript = bracket(p.expr.clone().map(Some).or(just(Token::DotDot).map(|_| None))).boxed();

	// infix operator builder
	let binop = |prec, tok: Token, op: BinOp| {
		infix(left(prec), just(tok), move |l, _, r, ex| {
			(Expr::Binary(op, Box::new(l), Box::new(r)), ex.span())
		})
	};

	let core = atom
		.pratt((
			// field/tuple/method access
			postfix(13, just(Token::Dot).ignore_then(access), |lhs, acc, ex| match acc {
				Dot::Fields(parts) => parts.into_iter().fold(lhs, |cur, field| {
					(
						Expr::Field {
							tuple: Box::new(cur),
							field,
						},
						ex.span(),
					)
				}),
				Dot::Method(method, type_args, args) => (
					Expr::MethodCall {
						recv: Box::new(lhs),
						method,
						type_args,
						args,
					},
					ex.span(),
				),
			}),
			// indexing and slicing
			postfix(13, subscript, |lhs, sub: Option<Spanned<Expr>>, ex| {
				let collection = Box::new(lhs);
				let e = match sub {
					Some(index) if index.0.bounds().is_none() => Expr::Index {
						collection,
						index: Box::new(index),
					},
					range => Expr::Slice {
						collection,
						range: range.map(Box::new),
					},
				};
				(e, ex.span())
			}),
			// applying a fn value
			postfix(13, p.adjacent.ignore_then(args.clone()), |lhs, args, ex| {
				let callee = Box::new(lhs);
				(Expr::Apply { callee, args }, ex.span())
			}),
			// propagator
			postfix(13, just(Token::Question), |lhs, _, ex| {
				(Expr::Propagate(Box::new(lhs)), ex.span())
			}),
			// expression annotations
			prefix(12, p.annotation.clone(), |a, rhs, ex| {
				(Expr::Annotated(vec![a], Box::new(rhs)), ex.span())
			}),
			// unary
			prefix(12, just(Token::Minus).or(just(Token::SpaceMinus)), |_, rhs, ex| {
				(Expr::Negative(Box::new(rhs)), ex.span())
			}),
			prefix(12, just(Token::Not), |_, rhs, ex| match rhs {
				(Expr::Cast { target: (t, ts), args }, _) => {
					let target = (types::result_of(t, None), ts);
					(Expr::Cast { target, args }, ex.span())
				}
				_ => (Expr::Not(Box::new(rhs)), ex.span()),
			}),
			// arithmetic
			infix(right(12), just(Token::StarStar), |l, _, r, ex| {
				(Expr::Binary(BinOp::Pow, Box::new(l), Box::new(r)), ex.span())
			}),
			binop(11, Token::Asterisk, BinOp::Mul),
			binop(11, Token::Slash, BinOp::Div),
			infix(
				left(11),
				p.same_line.clone().ignore_then(just(Token::Percent)),
				|l, _, r, ex| (Expr::Binary(BinOp::Mod, Box::new(l), Box::new(r)), ex.span()),
			),
			binop(10, Token::Plus, BinOp::Add),
			binop(10, Token::Minus, BinOp::Sub),
			// bitwise
			(
				binop(9, Token::LtLt, BinOp::Shl),
				binop(9, Token::GtGt, BinOp::Shr),
				binop(8, Token::Amp, BinOp::BitAnd),
				binop(7, Token::Tilde, BinOp::BitXor),
				binop(6, Token::Pipe, BinOp::BitOr),
			),
			// relational
			binop(4, Token::Lt, BinOp::Lt),
			binop(4, Token::Gt, BinOp::Gt),
			binop(4, Token::Le, BinOp::Le),
			binop(4, Token::Ge, BinOp::Ge),
			// trait check
			postfix(
				4,
				just(Token::Is)
					.ignore_then(just(Token::Ident("not".into())).or_not())
					.then(ident()),
				|lhs, (not, trait_name): (Option<Token>, String), ex| {
					(
						Expr::Is {
							subject: Box::new(lhs),
							trait_name,
							negated: not.is_some(),
						},
						ex.span(),
					)
				},
			),
			// equality | membership
			binop(3, Token::Eq, BinOp::Eq),
			binop(3, Token::Ne, BinOp::Ne),
			binop(3, Token::In, BinOp::In),
			// logical
			binop(2, Token::AndAnd, BinOp::And),
			binop(1, Token::OrOr, BinOp::Or),
			// ranges
			(
				postfix(
					5,
					just(Token::DotDot).then_ignore(just(Token::LBrace).not().then(p.expr.clone()).not()),
					|l, _, ex| range(l, None, false, ex.span()),
				),
				infix(left(5), just(Token::DotDot), |l, _, r, ex| {
					range(l, Some(r), false, ex.span())
				}),
				infix(left(5), just(Token::DotDotEq), |l, _, r, ex| {
					range(l, Some(r), true, ex.span())
				}),
				prefix(5, just(Token::DotDot), |_, r, ex| {
					(Expr::Spread(Box::new(r)), ex.span())
				}),
			),
			// pipelines
			infix(left(0), just(Token::Pipeline), |l, _, r, ex| pipe(l, r, ex.span())),
		))
		.boxed();

	// juxts
	let trailing = anon_fn.clone().or(p.block_ast.clone()).boxed();
	let trail_only = trailing.clone().map(|t| (None, Some(t))).boxed();
	let with_lead = p
		.same_line
		.ignore_then(header_expr.clone())
		.then(trailing.or_not())
		.map(|(l, t)| (Some(l), t))
		.boxed();

	// or blocks
	let or_tail = just(Token::Or).ignore_then(p.block.clone().or(core.clone().map(|e| vec![pipe_step(e)])));
	let level = |juxt: Option<P<'token, I, Juxt>>| {
		let inner = match juxt {
			None => core.clone().boxed(),
			Some(juxt) => core
				.clone()
				.then(juxt.or_not())
				.try_map(|((inner, s), jx), span| {
					let Some((lead, trail)) = jx else { return Ok((inner, s)) };
					let has_lead = lead.is_some();
					let args: Vec<_> = lead.into_iter().chain(trail).collect();
					let e = match inner {
						Expr::Ident(name) => Expr::Call {
							name,
							type_args: vec![],
							args,
						},
						Expr::Field { tuple, field } => Expr::MethodCall {
							recv: tuple,
							method: field,
							type_args: vec![],
							args,
						},
						Expr::Call {
							name,
							type_args,
							args: mut a,
						} if !has_lead => {
							a.extend(args);
							Expr::Call {
								name,
								type_args,
								args: a,
							}
						}
						Expr::MethodCall {
							recv,
							method,
							type_args,
							args: mut a,
						} if !has_lead => {
							a.extend(args);
							Expr::MethodCall {
								recv,
								method,
								type_args,
								args: a,
							}
						}
						_ => return Err(Rich::custom(span, "trailing arg needs a call or method callee")),
					};
					Ok((e, span))
				})
				.or(core.clone())
				.boxed(),
		};
		inner
			.then(or_tail.clone().or_not())
			.map_with(|pair, ex| or_else(pair, ex.span()))
			.boxed()
	};
	header_expr.define(level(None));
	let (bind, test) = binds(level(None).boxed(), match_pat.clone());
	header_cond.define(bind.or(test).or(header_expr.clone()));
	juxt_expr.define(level(Some(trail_only.clone().or(with_lead).boxed())));
	level(Some(trail_only))
}
