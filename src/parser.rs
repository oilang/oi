use crate::ast::{
	Access, BinOp, Capture, Child, EnumVariant, Expr, MatchArm, Param, Span, Spanned, TypeExpr, TypeParam, UseItem,
};
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
	Cast(Spanned<Expr>),
}

// One entry of a struct/enum/trait body.
enum Member {
	Field(Param),
	Fn(Spanned<Expr>),
	Variant(EnumVariant),
}

// The type a binding default's literal names.
fn literal_typ(e: &Expr) -> Option<&'static str> {
	match e {
		Expr::Bool(_) => Some("bool"),
		Expr::Int(_) => Some("int"),
		Expr::Float(_) => Some("float"),
		Expr::String(_) => Some("string"),
		_ => None,
	}
}

fn split_members(members: Vec<Member>) -> (Vec<Param>, Vec<Spanned<Expr>>, Vec<EnumVariant>) {
	let (mut fields, mut fns, mut variants) = (vec![], vec![], vec![]);
	for m in members {
		match m {
			Member::Field(f) => fields.push(f),
			Member::Fn(f) => fns.push(f),
			Member::Variant(v) => variants.push(v),
		}
	}
	(fields, fns, variants)
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

// `value |> step`
fn pipe(value: Spanned<Expr>, step: Spanned<Expr>, span: Span) -> Spanned<Expr> {
	(
		Expr::Pipe {
			value: Box::new(value),
			step: Box::new(pipe_step(step)),
		},
		span,
	)
}

// Handle named results.
type Bound = (String, (Spanned<TypeExpr>, Option<Spanned<Expr>>));
fn named_ret(bound: Option<Bound>, mut body: Vec<Spanned<Expr>>, span: Span) -> Vec<Spanned<Expr>> {
	let Some((name, (typ, default))) = bound else {
		return body;
	};
	body.iter_mut().for_each(|e| bind_return(e, &name));
	let decl = Expr::Bind {
		mutable: true,
		name,
		typ: Some(typ),
		value: default.map(Box::new),
	};
	body.insert(0, (decl, span));
	body
}

// Bind return variables for named results.
fn bind_return((e, span): &mut Spanned<Expr>, name: &str) {
	match e {
		Expr::Fn { .. } | Expr::AnonFn { .. } | Expr::MacroDef { .. } | Expr::Quote(_) => return,
		Expr::Return(v @ None) => *v = Some(Box::new((Expr::Ident(name.into()), *span))),
		_ => {}
	}
	e.for_children(|c| match c {
		Child::List(list) => list.iter_mut().for_each(|x| bind_return(x, name)),
		Child::One(x) => bind_return(x, name),
	});
}

// Assemble a fn item.
fn fn_def(
	(name, mut type_params): (String, Vec<TypeParam>),
	params: Option<(Vec<Param>, bool)>,
	ret: Option<Spanned<TypeExpr>>,
	body: Vec<Spanned<Expr>>,
	span: Span,
) -> Spanned<Expr> {
	let (params, params_tuple) = params.unwrap_or_else(|| {
		type_params.push(TypeParam {
			name: "$I".into(),
			bound: None,
		});
		let param = Param {
			name: "$".into(),
			typ: TypeExpr::Name("$I".into()),
			span,
			default: None,
			access: Access::Read,
			public: false,
			annotations: vec![],
		};
		(vec![param], false)
	});
	(
		Expr::Fn {
			name,
			type_params,
			params,
			params_tuple,
			ret,
			body,
		},
		span,
	)
}

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

pub fn parser<'src, 'token, I>(
	src: &'src str,
	origin: usize,
) -> impl Parser<'token, I, Vec<Spanned<Expr>>, extra::Err<Rich<'token, Token>>>
where
	'src: 'token,
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
	// `expr` and the statement/block parsers are mutually recursive
	// `if` is an expression, but its branches are statement blocks.
	// Declare `expr` up front so the statement parsers can reference it before it is defined.
	let mut expr = Recursive::declare();
	let mut header_expr = Recursive::declare();
	let mut block = Recursive::declare();
	let mut anon_fields = Recursive::declare();
	let mut item = Recursive::declare();

	let ident = || select! { Token::Ident(name) => name };

	// param access modifiers
	let access = choice((just(Token::Mut).to(Access::Mut), just(Token::Move).to(Access::Move))).boxed();

	let dotted_name = ident()
		.then(just(Token::Dot).ignore_then(ident()).or_not())
		.map(|(first, rest)| match rest {
			Some(second) => format!("{first}.{second}"),
			None => first,
		});

	let gap = |sp: Span| Span::from(sp.end..sp.start);
	// a guard that the next token has no token gap following it
	let adjacent = empty().map_with(move |_, ex| gap(ex.span())).try_map(move |sp: Span, _| {
		match src.get(sp.start - origin..sp.end - origin) {
			Some("") => Ok(()),
			_ => Err(Rich::custom(sp, "must immediately follow, with no space")),
		}
	});
	// a guard that the next token opens on the same line
	let same_line = empty().map_with(move |_, ex| gap(ex.span())).try_map(move |sp: Span, _| {
		match src.get(sp.start - origin..sp.end - origin) {
			Some(gap) if gap.contains('\n') => Err(Rich::custom(sp, "must continue on the same line")),
			_ => Ok(()),
		}
	});

	fn paren<'token, I, O, P>(p: P) -> impl Parser<'token, I, O, extra::Err<Rich<'token, Token>>> + Clone
	where
		I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
		P: Parser<'token, I, O, extra::Err<Rich<'token, Token>>> + Clone,
	{
		p.delimited_by(just(Token::LParen), just(Token::RParen))
	}
	fn brace<'token, I, O, P>(p: P) -> impl Parser<'token, I, O, extra::Err<Rich<'token, Token>>> + Clone
	where
		I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
		P: Parser<'token, I, O, extra::Err<Rich<'token, Token>>> + Clone,
	{
		p.delimited_by(just(Token::LBrace), just(Token::RBrace))
	}
	fn bracket<'token, I, O, P>(p: P) -> impl Parser<'token, I, O, extra::Err<Rich<'token, Token>>> + Clone
	where
		I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
		P: Parser<'token, I, O, extra::Err<Rich<'token, Token>>> + Clone,
	{
		p.delimited_by(just(Token::LBracket), just(Token::RBracket))
	}
	fn list<'token, I, O, P>(p: P) -> impl Parser<'token, I, Vec<O>, extra::Err<Rich<'token, Token>>> + Clone
	where
		I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
		P: Parser<'token, I, O, extra::Err<Rich<'token, Token>>> + Clone,
	{
		p.separated_by(just(Token::Comma)).collect::<Vec<_>>()
	}
	fn loose_list<'token, I, O, P>(p: P) -> impl Parser<'token, I, Vec<O>, extra::Err<Rich<'token, Token>>> + Clone
	where
		I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
		P: Parser<'token, I, O, extra::Err<Rich<'token, Token>>> + Clone,
	{
		p.separated_by(just(Token::Comma).or_not()).allow_trailing().collect::<Vec<_>>()
	}
	fn spanned<'token, I, O, P>(p: P) -> impl Parser<'token, I, Spanned<O>, extra::Err<Rich<'token, Token>>> + Clone
	where
		I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
		P: Parser<'token, I, O, extra::Err<Rich<'token, Token>>> + Clone,
	{
		p.map_with(|o, ex| (o, ex.span()))
	}

	let unquote = just(Token::Percent)
		.then_ignore(adjacent)
		.ignore_then(ident().map(Expr::Unquote).or(brace(expr.clone()).map(|e| match e {
			(Expr::Spread(inner), _) => Expr::UnquoteSplat(inner),
			e => Expr::UnquoteExpr(Box::new(e)),
		})))
		.map_with(|e, ex| (e, ex.span()))
		.boxed();

	// annotations
	let ann_entry = ident().then_ignore(just(Token::Assign)).or_not().then(expr.clone());
	let ann_tag = spanned(select! { Token::Atom(name) => Expr::Atom(name) });
	enum AnnTail {
		Fields(Vec<(Option<String>, Spanned<Expr>)>),
		Args(Vec<Spanned<Expr>>),
	}
	let ann_tail = just(Token::Dot)
		.ignore_then(brace(loose_list(ann_entry)))
		.map(AnnTail::Fields)
		.or(paren(loose_list(expr.clone())).map(AnnTail::Args));
	let ann_value = spanned(
		ident()
			.then_ignore(adjacent.then(just(Token::Not)).not())
			.then(adjacent.ignore_then(ann_tail).or_not())
			.map(|(name, tail)| match tail {
				Some(AnnTail::Fields(fields)) => Expr::StructLit {
					name,
					type_args: vec![],
					fields,
				},
				Some(AnnTail::Args(args)) => Expr::Call {
					name,
					type_args: vec![],
					args,
				},
				None => Expr::Ident(name),
			}),
	);
	// `@unsafe`
	let ann_unsafe = spanned(just(Token::Unsafe).to(Expr::Ident("unsafe".into())));
	let annotation = just(Token::At)
		.then_ignore(adjacent)
		.ignore_then(ann_tag.or(ann_unsafe).or(ann_value))
		.boxed();
	let annotations = annotation.clone().repeated().at_least(1).collect::<Vec<_>>().boxed();

	// type annotations
	let type_expr = recursive(|te| {
		let base = recursive(|base| {
			let name = dotted_name.clone().map(TypeExpr::Name);
			let unit = just(Token::LParen).then(just(Token::RParen)).to(TypeExpr::Tuple(vec![]));
			let tuple_field = ident().then_ignore(just(Token::Colon)).or_not().then(te.clone());
			let tuple = paren(
				tuple_field
					.separated_by(just(Token::Comma).or_not())
					.allow_trailing()
					.at_least(1)
					.collect::<Vec<_>>(),
			)
			.map(TypeExpr::Tuple);
			// arrays
			let len = spanned(select! { Token::Int(n) => Expr::Int(n) }.or(dotted_name.clone().map(Expr::Ident)));
			let array = just(Token::LBracket)
				.ignore_then(len.or_not())
				.then_ignore(just(Token::RBracket))
				.then(base.clone())
				.map(|(n, elem)| match n {
					Some(n) => TypeExpr::FixedArray(Box::new(elem), Box::new(n)),
					None => TypeExpr::Array(Box::new(elem)),
				});
			let fn_param = access
				.clone()
				.or_not()
				.then(ident().then_ignore(just(Token::Colon)).or_not())
				.then(te.clone())
				.map(|((a, n), t)| (n, a.unwrap_or_default(), t));
			let fn_ret = same_line.ignore_then(base.clone()).or_not();
			let fn_type = just(Token::Fn)
				.ignore_then(paren(loose_list(fn_param)))
				.then(fn_ret)
				.map(|(params, ret)| TypeExpr::Fn(params, Box::new(ret.unwrap_or(TypeExpr::Tuple(vec![])))));
			// annotations
			let annotated = annotation
				.clone()
				.then(base.clone())
				.map(|(a, t)| TypeExpr::Annotated(vec![a], Box::new(t)));
			// options
			let option = just(Token::Question)
				.ignore_then(base.clone())
				.map(|t| TypeExpr::Option(Box::new(t)));
			// varargs
			let variadic = just(Token::DotDot)
				.ignore_then(base.clone())
				.map(|t| TypeExpr::Variadic(Box::new(t)));
			// results
			let result = just(Token::Not)
				.ignore_then(base.clone())
				.map(|t| TypeExpr::Result(Box::new(t), None));
			// shared refs
			let ref_type = just(Token::Amp).ignore_then(base.clone()).map(|t| TypeExpr::Ref(Box::new(t)));
			// atom(s)
			let atom = select! { Token::Atom(a) => TypeExpr::AtomSum(vec![a]) };

			// built-in generic types
			let result_long = just(Token::Ident("Result".to_string()))
				.ignore_then(bracket(te.clone().then_ignore(just(Token::Comma)).then(te.clone())))
				.map(|(t, e)| TypeExpr::Result(Box::new(t), Some(Box::new(e))));
			let option_long = just(Token::Ident("Option".to_string()))
				.ignore_then(bracket(te.clone()))
				.map(|t| TypeExpr::Option(Box::new(t)));
			// anonymous structs
			let anon_struct = just(Token::Struct)
				.ignore_then(brace(anon_fields.clone()))
				.map(TypeExpr::AnonStruct);

			// generic struct instantiation
			let generic_instance = ident()
				.then(bracket(
					te.clone().separated_by(just(Token::Comma)).at_least(1).collect::<Vec<_>>(),
				))
				.map(|(name, args)| TypeExpr::Generic(name, args));

			let hole = unquote.clone().map(|u| TypeExpr::Unquote(Box::new(u)));

			choice((
				hole,
				unit,
				annotated,
				fn_type,
				option,
				variadic,
				result,
				atom,
				anon_struct,
				result_long,
				option_long,
				generic_instance,
				name,
				tuple,
				array,
			))
			.or(ref_type)
		});

		base.clone()
			.then(same_line.ignore_then(just(Token::Not)).ignore_then(base).or_not())
			.map(|(e, ok)| match ok {
				Some(ok) => TypeExpr::Result(Box::new(ok), Some(Box::new(e))),
				None => e,
			})
			.separated_by(just(Token::Pipe))
			.at_least(1)
			.collect::<Vec<_>>()
			.map(|mut ms| {
				if ms.len() == 1 {
					return ms.pop().unwrap();
				}
				let atom = |m: &TypeExpr| match m {
					TypeExpr::AtomSum(a) if a.len() == 1 => Some(a[0].clone()),
					_ => None,
				};
				match ms.iter().map(atom).collect::<Option<Vec<_>>>() {
					Some(names) => TypeExpr::AtomSum(names),
					None => TypeExpr::Sum(ms),
				}
			})
	})
	.boxed();

	// defaults
	let bind_default = just(Token::Bind)
		.ignore_then(expr.clone())
		.try_map(|(value, span), _| {
			let typ = literal_typ(&value)
				.ok_or_else(|| Rich::custom(span, "a `:=` default must be a literal, or name the type"))?;
			Ok((TypeExpr::Name(typ.into()), Some((value, span))))
		})
		.boxed();

	// param type is kept for the compiler to resolve
	// NOTE: a bare `self` receiver gets the type `Self`
	let param = access
		.clone()
		.or_not()
		.then(ident())
		.then(
			just(Token::Colon)
				.ignore_then(type_expr.clone())
				.then(just(Token::Assign).ignore_then(expr.clone()).or_not())
				.or(bind_default.clone())
				.or_not(),
		)
		.map_with(|((access, name), typed), ex| {
			let (typ, default) = typed.unzip();
			Param {
				typ: typ.unwrap_or_else(|| TypeExpr::Name(if name == "self" { "Self" } else { "$?" }.into())),
				name,
				span: ex.span(),
				default: default.flatten(),
				access: access.unwrap_or_default(),
				public: false,
				annotations: vec![],
			}
		});
	let param_hole = unquote.clone().map_with(|u, ex| Param {
		name: String::new(),
		typ: TypeExpr::Unquote(Box::new(u)),
		span: ex.span(),
		default: None,
		access: Access::Read,
		public: false,
		annotations: vec![],
	});
	let param = param_hole.clone().or(param).boxed();
	// NOTE: a trailing comma forces a tuple even for one param
	let params = paren(
		param
			.separated_by(just(Token::Comma))
			.collect::<Vec<_>>()
			.then(just(Token::Comma).or_not()),
	)
	.map(|(params, trailing)| {
		let tuple = params.len() != 1 || trailing.is_some();
		(params, tuple)
	})
	.boxed();

	// optional return type annotation, optionally bound to a name
	let ret = spanned(type_expr.clone()).or_not();
	let typed = just(Token::Colon)
		.ignore_then(spanned(type_expr.clone()))
		.then(just(Token::Assign).ignore_then(expr.clone()).or_not());
	let bound_ret = bind_default.clone().map_with(|(typ, default), ex| ((typ, ex.span()), default));
	let fn_ret = ident().then(typed.or(bound_ret)).or_not().then(ret.clone()).boxed();

	// generics
	let type_param = ident()
		.then(just(Token::Colon).ignore_then(ident()).or_not())
		.map(|(name, bound)| TypeParam { name, bound });
	let type_params = bracket(list(type_param)).or_not().map(Option::unwrap_or_default).boxed();

	// a bare block is a fn literal wherever a fn type is expected
	let block_lit = block
		.clone()
		.map_with(|body, ex| {
			(
				Expr::AnonFn {
					captures: None,
					params: vec![],
					params_tuple: true,
					ret: None,
					body,
				},
				ex.span(),
			)
		})
		.boxed();

	// bindings
	let annot = spanned(type_expr.clone());
	let value_tail = just(Token::Bind)
		.to(true)
		.or(just(Token::DoubleColon).to(false))
		.then(expr.clone())
		.map(|(mutable, value)| (mutable, None, Some(value)));
	let sandwich_tail = just(Token::Colon)
		.ignore_then(annot.clone().validate(|t, _, emitter| {
			if let (TypeExpr::AnonStruct(_), s) = &t {
				emitter.emit(Rich::custom(*s, "an anonymous struct type can't be a binding's middle"));
			}
			t
		}))
		.then(
			just(Token::Assign)
				.to(true)
				.or(just(Token::Colon).to(false))
				.then(expr.clone().or(block_lit.clone()))
				.or_not(),
		)
		.map(|(typ, tail)| match tail {
			Some((mutable, value)) => (mutable, Some(typ), Some(value)),
			None => (true, Some(typ), None),
		});
	// macro bindings
	let bind_name = just(Token::Percent)
		.then_ignore(adjacent)
		.ignore_then(
			ident()
				.map(|n| (format!("%{n}"), None))
				.or(brace(expr.clone()).map(|e| ("%".to_string(), Some(e)))),
		)
		.or(ident().map(|n| (n, None)));
	let bind = bind_name
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
		});

	// compound assignment
	let assign_op = choice((
		just(Token::PlusEq).to(Some(BinOp::Add)),
		just(Token::MinusEq).to(Some(BinOp::Sub)),
		just(Token::StarStarEq).to(Some(BinOp::Pow)),
		just(Token::StarEq).to(Some(BinOp::Mul)),
		just(Token::SlashEq).to(Some(BinOp::Div)),
		just(Token::PercentEq).to(Some(BinOp::Mod)),
		just(Token::Assign).to(None),
	));
	let fold = |op, lhs, value: Spanned<Expr>, span| match op {
		None => value,
		Some(op) => (Expr::Binary(op, Box::new((lhs, span)), Box::new(value)), span),
	};

	// assignment
	let assign = ident()
		.then(assign_op.clone())
		.then(expr.clone().or(block_lit.clone()))
		.map_with(move |((name, op), value), ex| {
			let value = fold(op, Expr::Ident(name.clone()), value, ex.span());
			(
				Expr::Assign {
					name,
					value: Box::new(value),
				},
				ex.span(),
			)
		});

	// return statements
	let ret_stmt = just(Token::Return)
		.ignore_then(expr.clone().or_not())
		.map_with(|value, ex| (Expr::Return(value.map(Box::new)), ex.span()));

	// index assignment
	let index_assign = ident()
		.then(bracket(expr.clone()))
		.then(assign_op.clone())
		.then(expr.clone())
		.map_with(move |(((name, index), op), value), ex| {
			let collection = Box::new((Expr::Ident(name.clone()), ex.span()));
			let lhs = Expr::Index {
				collection,
				index: Box::new(index.clone()),
			};
			let value = fold(op, lhs, value, ex.span());
			(
				Expr::IndexAssign {
					name,
					index: Box::new(index),
					value: Box::new(value),
				},
				ex.span(),
			)
		});

	// array appending
	let append = ident()
		.then_ignore(just(Token::LtLt))
		.then(expr.clone().or(block_lit.clone()))
		.map_with(|(name, value), ex| {
			(
				Expr::Append {
					name,
					value: Box::new(value),
				},
				ex.span(),
			)
		});

	// map deletion
	let map_delete = ident()
		.then_ignore(just(Token::Dot))
		.then_ignore(just(Token::Ident("delete".to_string())))
		.then(bracket(expr.clone()))
		.map_with(|(name, key), ex| {
			(
				Expr::MapDelete {
					name,
					key: Box::new(key),
				},
				ex.span(),
			)
		});

	// field assignment
	let field_assign = ident()
		.then_ignore(just(Token::Dot))
		.then(ident())
		.then(assign_op)
		.then(expr.clone())
		.map_with(move |(((name, field), op), value), ex| {
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

	// pattern binds
	let destructure = pat
		.clone()
		.then(
			just(Token::Bind)
				.to(Some(true))
				.or(just(Token::DoubleColon).to(Some(false)))
				.or(just(Token::Assign).to(None)),
		)
		.then(expr.clone())
		.map_with(|((pat, mutable), value), ex| {
			(
				Expr::PatBind {
					pat: Box::new(pat),
					mutable,
					value: Box::new(value),
				},
				ex.span(),
			)
		});

	let doc = select! { Token::Doc(text) => text }
		.repeated()
		.at_least(1)
		.collect::<Vec<_>>()
		.map_with(|lines, ex| (Expr::Doc(lines), ex.span()))
		.then_ignore(just(Token::DocBreak).or_not());

	let macro_def = ident()
		.then_ignore(adjacent)
		.then_ignore(just(Token::Not))
		.then_ignore(just(Token::DoubleColon))
		.then_ignore(just(Token::Fn))
		.then(params.clone())
		.then(ret.clone())
		.then(block.clone())
		.map_with(|(((name, (params, _)), ret), body), ex| {
			(
				Expr::MacroDef {
					name,
					params,
					ret,
					body,
				},
				ex.span(),
			)
		});

	let macro_stmt = dotted_name
		.clone()
		.then_ignore(adjacent)
		.then_ignore(just(Token::Not))
		.then_ignore(adjacent.then(just(Token::LParen)).not())
		.then(expr.clone())
		.map_with(|(name, arg), ex| (Expr::MacroCall { name, args: vec![arg] }, ex.span()));

	// statements
	let stmt = doc
		.or(ret_stmt)
		.or(destructure)
		.or(bind.clone())
		.or(field_assign)
		.or(assign)
		.or(index_assign)
		.or(map_delete)
		.or(append)
		.or(macro_def.clone())
		.or(macro_stmt)
		.or(expr.clone())
		.boxed();

	// blocks
	block.define(brace(stmt.clone().repeated().collect::<Vec<_>>()));

	let definition = {
		let literal = select! {
			Token::Bool(b) => Expr::Bool(b),
			Token::Int(n) => Expr::Int(n),
			Token::Float(s) => Expr::Float(s.parse().unwrap()),
			Token::String(s) => Expr::String(s),
			Token::Atom(name) => Expr::Atom(name),
			Token::Dollar => Expr::Dollar,
			Token::None => Expr::None,
		};

		// arg mods
		let mod_arg = access
			.clone()
			.then(expr.clone())
			.map_with(|(a, e), ex| (Expr::ArgMod(a, Box::new(e)), ex.span()));
		// named args collect into one trailing record arg
		let named_arg = ident()
			.map_with(|n, ex| (Expr::Ident(n), ex.span()))
			.then_ignore(just(Token::Assign))
			.then(mod_arg.clone().or(expr.clone()))
			.map(|(key, value)| (Some(key), value));
		// variable vs. call vs. struct literal
		let args = paren(
			named_arg
				.or((mod_arg.or(expr.clone()).or(block_lit.clone())).map(|e| (None, e)))
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
		let struct_field_entry = ident()
			.then_ignore(just(Token::Assign))
			.or_not()
			.then(expr.clone().or(block_lit.clone()));
		let struct_body = brace(loose_list(struct_field_entry.clone()));

		// explicit generic types
		let call_type_args = bracket(
			spanned(type_expr.clone())
				.separated_by(just(Token::Comma))
				.at_least(1)
				.collect::<Vec<_>>(),
		);

		// struct literals
		let struct_lit = ident()
			.then(call_type_args.clone().or_not())
			.or_not()
			.then_ignore(just(Token::Dot))
			.then(struct_body)
			.map(|(head, fields)| {
				let (name, type_args) = head.unwrap_or_default();
				Expr::StructLit {
					name,
					type_args: type_args.unwrap_or_default(),
					fields,
				}
			});

		let foreign_lit = just(Token::Foreign).to(Expr::Foreign);

		let ref_lit = just(Token::Amp).ignore_then(expr.clone()).try_map(|e, span| match &e.0 {
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
		let keyed = spanned(key).then(just(Token::Assign).ignore_then(expr.clone().or(block_lit.clone())));
		let record_entries = keyed
			.clone()
			.then_ignore(just(Token::Comma).or_not())
			.then(loose_list(keyed.clone()))
			.map(|(first, mut rest)| {
				rest.insert(0, first);
				rest
			})
			.boxed();
		let record_arg = brace(record_entries.clone()).map_with(|es, ex| vec![(Expr::Record(es), ex.span())]);

		// enum shorthand
		let enum_shorthand = just(Token::Dot)
			.ignore_then(select! { Token::Ident(v) => v, Token::None => "none".to_string() })
			.then(args.clone().or(record_arg.clone()).or_not())
			.map_with(|(variant, args), ex| {
				let args = args.unwrap_or_default();
				(Expr::EnumShorthand { variant, args }, ex.span())
			})
			.boxed();

		// a lexer error token
		let bad = select! { Token::Error(text) => text }
			.try_map(|text, span| Err(Rich::custom(span, format!("unexpected character `{text}`"))));

		// grouping before tuple rule to avoid making 1ples, which are instead made with `(expr,)`
		let group = paren(expr.clone());

		// tuple literals
		let tuple = paren(loose_list(struct_field_entry)).map_with(|elems, ex| (Expr::Tuple(elems), ex.span()));

		// dot array literals
		let dot_array = type_expr
			.clone()
			.or_not()
			.then_ignore(just(Token::Dot))
			.then(bracket(loose_list(expr.clone())))
			.map_with(|(elem, elems), ex| (Expr::DotArray(elem.map(|t| (t, ex.span())), elems), ex.span()));

		// dot tuple literals
		let dot_tuple = just(Token::Dot)
			.ignore_then(paren(loose_list(expr.clone())))
			.map_with(|elems, ex| (Expr::DotTuple(elems), ex.span()));

		let option_init = just(Token::Question)
			.ignore_then(type_expr.clone())
			.then(paren(expr.clone()))
			.map_with(|(elem, arg), ex| {
				(
					Expr::OptionInit {
						inner: (elem, ex.span()),
						arg: Box::new(arg),
					},
					ex.span(),
				)
			});

		// result literals
		let result_shape = type_expr
			.clone()
			.filter(|t| !matches!(t, TypeExpr::Name(_)))
			.then(paren(expr.clone()));
		let result_init = just(Token::Not).ignore_then(result_shape.clone()).map_with(|(elem, arg), ex| {
			(
				Expr::ResultInit {
					inner: (elem, ex.span()),
					arg: Box::new(arg),
				},
				ex.span(),
			)
		});

		// inline macro calls
		let macro_call = dotted_name
			.clone()
			.then_ignore(adjacent)
			.then_ignore(just(Token::Not))
			.then_ignore(adjacent)
			.then(paren(loose_list(expr.clone())))
			.map_with(|(name, args), ex| (Expr::MacroCall { name, args }, ex.span()));

		// map literals
		let map_entry = expr
			.clone()
			.then_ignore(just(Token::Assign))
			.then(expr.clone().or(block_lit.clone()));
		let map = bracket(
			map_entry
				.separated_by(just(Token::Comma).or_not())
				.allow_trailing()
				.at_least(1)
				.collect::<Vec<_>>(),
		)
		.map_with(|entries, ex| (Expr::Map(entries), ex.span()));

		let array = bracket(loose_list(expr.clone())).map_with(|elems, ex| (Expr::Array(elems), ex.span()));

		let if_expr = recursive(|if_expr| {
			just(Token::If)
				.ignore_then(header_expr.clone())
				.then(block.clone())
				.then(
					just(Token::Else)
						.ignore_then(if_expr.map(|e| vec![e]).or(block.clone()))
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
				block
					.clone()
					.map(|body| (None, body))
					.or(header_expr.clone().map(Some).then(block.clone())),
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
			.ignore_then(pat.clone().or(pat_name))
			.then_ignore(just(Token::In))
			.then(header_expr.clone().map(Box::new))
			.then(block.clone())
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
		let break_expr = just(Token::Break).map_with(|_, ex| (Expr::Break, ex.span()));
		let continue_expr = just(Token::Continue).map_with(|_, ex| (Expr::Continue, ex.span()));

		// match expression
		let binding = ident().then_ignore(just(Token::At)).or_not();
		let arm_end = choice((
			just(Token::Comma).ignored(),
			just(Token::RBrace).rewind().ignored(),
			just(Token::Else).rewind().ignored(),
			just(Token::Backtick).rewind().ignored(),
		));
		let arm_body = block
			.clone()
			.then_ignore(just(Token::Comma).or_not())
			.or(expr.clone().map(|e| vec![e]).then_ignore(arm_end));
		let bind = ident().map_with(|n, ex| ((Expr::Ident(n.clone()), ex.span()), (Expr::Ident(n), ex.span())));
		let struct_pat = just(Token::Dot)
			.ignore_then(select! { Token::Ident(v) => v })
			.then(brace(loose_list(keyed.clone().or(bind))))
			.map_with(|(variant, es), ex| {
				let args = vec![(Expr::Record(es), ex.span())];
				(Expr::EnumShorthand { variant, args }, ex.span())
			});
		// container types
		let type_pat = type_expr
			.clone()
			.filter(|t| matches!(t, TypeExpr::Array(_) | TypeExpr::Map(..) | TypeExpr::FixedArray(..)))
			.map_with(|t, ex| (Expr::TypePat(t), ex.span()));
		let match_arm = binding
			.then(
				struct_pat
					.or(type_pat)
					.or(expr.clone())
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
		let arm_spread = unquote.clone().map(|e| MatchArm {
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
			.or(item.clone())
			.or(stmt.clone())
			.repeated()
			.at_least(1)
			.collect::<Vec<_>>()
			.delimited_by(just(Token::Backtick), just(Token::Backtick))
			.map_with(|stmts, ex| (Expr::Quote(stmts), ex.span()));

		// `comp`/`unsafe` take a braced block or a bare expression
		let kw_block = block.clone().map_with(|stmts, ex| (Expr::Block(stmts), ex.span()));
		let comp_expr = just(Token::Comp)
			.ignore_then(kw_block.clone().or(expr.clone()))
			.map_with(|inner, ex| (Expr::Comp(Box::new(inner)), ex.span()))
			.boxed();
		let unsafe_expr = just(Token::Unsafe)
			.ignore_then(kw_block.or(expr.clone()))
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
			.then(params.clone().or_not())
			.then(ret.clone())
			.then(block.clone())
			.map_with(|(((captures, params), ret), body), ex| {
				let (params, tuple) = params.unwrap_or((vec![], true));
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
			dot_tuple,
			macro_call,
			quote,
			unquote.clone(),
			leaf,
			enum_shorthand,
			group,
			tuple,
			option_init,
			result_init,
			map,
			array,
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
				.then(call_type_args.or_not().then(args.clone().or(record_arg.clone())).or_not())
				.map(|(name, call)| match call {
					Some((type_args, args)) => Dot::Method(name, type_args.unwrap_or_default(), args),
					None => Dot::Fields(vec![name]),
				}),
			paren(expr.clone()).map(Dot::Cast),
		))
		.boxed();

		// array subscripts
		let subscript = bracket(expr.clone().map(Some).or(just(Token::DotDot).map(|_| None))).boxed();

		// infix operator builder
		let binop = |prec, tok: Token, op: BinOp| {
			infix(left(prec), just(tok), move |l, _, r, ex| {
				(Expr::Binary(op, Box::new(l), Box::new(r)), ex.span())
			})
		};

		let core = atom
			.pratt((
				// field/tuple/method access
				postfix(9, just(Token::Dot).ignore_then(access), |lhs, acc, ex| match acc {
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
					Dot::Cast(value) => (
						Expr::Cast {
							target: Box::new(lhs),
							value: Box::new(value),
						},
						ex.span(),
					),
				}),
				// indexing and slicing
				postfix(9, subscript, |lhs, sub: Option<Spanned<Expr>>, ex| {
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
				postfix(9, adjacent.ignore_then(args.clone()), |lhs, args, ex| {
					let callee = Box::new(lhs);
					(Expr::Apply { callee, args }, ex.span())
				}),
				// propagator
				postfix(9, just(Token::Question), |lhs, _, ex| {
					(Expr::Propagate(Box::new(lhs)), ex.span())
				}),
				// expression annotations
				prefix(8, annotation.clone(), |a, rhs, ex| {
					(Expr::Annotated(vec![a], Box::new(rhs)), ex.span())
				}),
				// unary
				prefix(8, just(Token::Minus), |_, rhs, ex| {
					(Expr::Negative(Box::new(rhs)), ex.span())
				}),
				prefix(
					8,
					just(Token::Not).then_ignore(result_shape.clone().not()),
					|_, rhs, ex| (Expr::Not(Box::new(rhs)), ex.span()),
				),
				// arithmetic
				infix(right(8), just(Token::StarStar), |l, _, r, ex| {
					(Expr::Binary(BinOp::Pow, Box::new(l), Box::new(r)), ex.span())
				}),
				binop(7, Token::Asterisk, BinOp::Mul),
				binop(7, Token::Slash, BinOp::Div),
				infix(left(7), same_line.ignore_then(just(Token::Percent)), |l, _, r, ex| {
					(Expr::Binary(BinOp::Mod, Box::new(l), Box::new(r)), ex.span())
				}),
				binop(6, Token::Plus, BinOp::Add),
				binop(6, Token::Minus, BinOp::Sub),
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
					infix(left(5), just(Token::DotDot), |l, _, r, ex| {
						range(l, Some(r), false, ex.span())
					}),
					infix(left(5), just(Token::DotDotEq), |l, _, r, ex| {
						range(l, Some(r), true, ex.span())
					}),
					postfix(5, just(Token::DotDot).then_ignore(expr.clone().not()), |l, _, ex| {
						range(l, None, false, ex.span())
					}),
					prefix(5, just(Token::DotDot), |_, r, ex| {
						(Expr::Spread(Box::new(r)), ex.span())
					}),
				),
				// pipelines
				infix(left(0), just(Token::Pipeline), |l, _, r, ex| pipe(l, r, ex.span())),
			))
			.boxed();

		// juxts (leading literals and trailing functions)
		let trailing = anon_fn.clone().or(block_lit.clone());
		let lit_arg = spanned(literal);
		let juxt = choice((
			lit_arg.then(trailing.clone().or_not()).map(|(l, t)| (Some(l), t)),
			trailing.map(|t| (None, Some(t))),
		));
		let juxted = core
			.clone()
			.then(juxt.or_not())
			.try_map(|((inner, s), jx), span| {
				let Some((lit, trail)) = jx else { return Ok((inner, s)) };
				let has_lit = lit.is_some();
				let args: Vec<_> = lit.into_iter().chain(trail).collect();
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
					} if !has_lit => {
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
					} if !has_lit => {
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
			.boxed();

		// or blocks
		let or_tail = just(Token::Or).ignore_then(block.clone().or(core.clone().map(|e| vec![pipe_step(e)])));
		header_expr.define(
			core.then(or_tail.clone().or_not())
				.map_with(|pair, ex| or_else(pair, ex.span())),
		);
		juxted
			.then(or_tail.or_not())
			.map_with(|pair, ex| or_else(pair, ex.span()))
			.boxed()
	};
	expr.define(definition);

	// item bindings
	let item_head = ident().then(type_params.clone()).then_ignore(just(Token::DoubleColon));

	// fn defs
	let func = item_head
		.clone()
		.then_ignore(just(Token::Fn))
		.then(params.clone())
		.then(fn_ret.clone())
		.then(block.clone())
		.then_ignore(just(Token::Pipeline).not())
		.map_with(|(((head, params), (bound, ret)), body), ex| {
			let ret = bound.as_ref().map(|(_, (typ, _))| typ.clone()).or(ret);
			fn_def(head, Some(params), ret, named_ret(bound, body, ex.span()), ex.span())
		})
		.boxed();

	// struct defs
	let field_typed = just(Token::Colon)
		.ignore_then(type_expr.clone())
		.then(just(Token::Assign).ignore_then(expr.clone().or(block_lit.clone())).or_not());
	let struct_field = just(Token::Pub)
		.or_not()
		.then(ident())
		.then(field_typed.or(bind_default))
		.then(same_line.ignore_then(annotation.clone()).repeated().collect::<Vec<_>>())
		.map_with(|(((public, name), (typ, default)), annotations), ex| Param {
			name,
			typ,
			span: ex.span(),
			default,
			access: Access::Read,
			public: public.is_some(),
			annotations,
		})
		.boxed();
	let struct_field = param_hole.or(struct_field).boxed();
	anon_fields.define(loose_list(struct_field.clone()));
	// embedded structs
	let embedded = just(Token::Pub).or_not().then(ident()).map_with(|(public, name), ex| Param {
		typ: TypeExpr::Name(name.clone()),
		name,
		span: ex.span(),
		default: None,
		access: Access::Read,
		public: public.is_some(),
		annotations: vec![],
	});
	let struct_def = item_head
		.clone()
		.then_ignore(just(Token::Struct))
		.then(brace(loose_list(
			struct_field
				.clone()
				.map(Member::Field)
				.or(func.clone().map(Member::Fn))
				.or(embedded.map(Member::Field)),
		)))
		.map_with(|((name, type_params), members), ex| {
			let (fields, fills, _) = split_members(members);
			(
				Expr::StructDef {
					name,
					type_params,
					fields,
					fills,
				},
				ex.span(),
			)
		})
		.boxed();

	// tuple struct defs
	let ts_field = ident()
		.then_ignore(just(Token::Colon))
		.then(type_expr.clone())
		.map(|(n, t)| (Some(n), t))
		.or(type_expr.clone().map(|t| (None, t)));
	let tuple_struct_def = ident()
		.then_ignore(just(Token::DoubleColon))
		.then_ignore(just(Token::Struct))
		.then(paren(
			ts_field
				.separated_by(just(Token::Comma))
				.allow_trailing()
				.at_least(1)
				.collect::<Vec<_>>(),
		))
		.map_with(|(name, fields), ex| {
			let typ = TypeExpr::TupleStruct(name.clone(), fields);
			(Expr::TypeAlias { name, typ }, ex.span())
		})
		.boxed();

	// enum defs
	let disc_int = just(Token::Minus)
		.or_not()
		.then(select! { Token::Int(n) => n })
		.map(|(neg, n)| (Some(if neg.is_some() { -n } else { n }), None));
	let disc = just(Token::Assign).ignore_then(disc_int.or(select! { Token::String(s) => (None, Some(s)) }));
	let fields = brace(loose_list(ident().then_ignore(just(Token::Colon)).then(annot.clone())));
	let backing = just(Token::DoubleColon).to(None).or(just(Token::Colon)
		.ignore_then(annot.clone())
		.then_ignore(just(Token::Colon))
		.map(Some));
	let payload = paren(annot.separated_by(just(Token::Comma)).allow_trailing().collect::<Vec<_>>());
	let variant = ident()
		.then(
			payload
				.map(|p| (vec![], p))
				.or(fields.map(|fs| fs.into_iter().unzip()))
				.or_not(),
		)
		.then(disc.or_not())
		.map(|((name, body), assign)| {
			let (names, payload) = body.unwrap_or_default();
			let (disc, raw) = assign.unwrap_or((None, None));
			EnumVariant {
				name,
				disc,
				raw,
				payload,
				names,
			}
		});
	let enum_def = ident()
		.then(type_params.clone())
		.then(backing)
		.then_ignore(just(Token::Enum))
		.then(brace(loose_list(
			func.clone()
				.or(unquote.clone())
				.map(Member::Fn)
				.or(variant.map(Member::Variant)),
		)))
		.validate(|(((name, type_params), backing), members), ex, emitter| {
			let (_, fills, variants) = split_members(members);
			let mut next = 0;
			let mut seen = Vec::new();
			for v in &variants {
				let d = v.disc.unwrap_or(next);
				if seen.contains(&d) {
					let msg = format!("discriminant value `{d}` assigned more than once");
					emitter.emit(Rich::custom(ex.span(), msg));
				}
				seen.push(d);
				next = d + 1;
			}
			if backing.is_none() && variants.iter().any(|v| v.raw.is_some()) {
				emitter.emit(Rich::custom(ex.span(), "a raw value needs a string backing"));
			}
			(
				Expr::EnumDef {
					name,
					backing,
					type_params,
					variants,
					fills,
				},
				ex.span(),
			)
		})
		.boxed();

	// type aliases
	fn expr_shaped(t: &TypeExpr) -> bool {
		match t {
			TypeExpr::Name(_) => true,
			TypeExpr::AtomSum(atoms) => atoms.len() == 1,
			TypeExpr::Tuple(ts) => ts.iter().all(|(n, t)| n.is_none() && expr_shaped(t)),
			TypeExpr::Generic(_, args) => matches!(args.as_slice(), [a] if expr_shaped(a)),
			_ => false,
		}
	}
	let type_alias = ident()
		.then_ignore(just(Token::DoubleColon))
		.then(type_expr.clone())
		.filter(|(_, typ)| !expr_shaped(typ))
		.then_ignore(
			one_of([
				Token::LBrace,
				Token::LParen,
				Token::LBracket,
				Token::Dot,
				Token::DotDot,
				Token::Question,
				Token::Pipeline,
				Token::DoubleColon,
				Token::Bind,
				Token::Assign,
			])
			.not(),
		)
		.map_with(|(name, typ), ex| (Expr::TypeAlias { name, typ }, ex.span()));

	// trait definitions
	let slot_fn = ident()
		.then_ignore(just(Token::Colon))
		.then_ignore(just(Token::Fn))
		.then(params.clone())
		.then(ret.clone())
		.map_with(|((name, params), ret), ex| fn_def((name, vec![]), Some(params), ret, vec![], ex.span()));
	let supers = just(Token::DoubleColon)
		.to(vec![])
		.or(just(Token::Colon).ignore_then(list(ident())).then_ignore(just(Token::Colon)));
	let trait_def = ident()
		.then(supers)
		.then_ignore(just(Token::Trait))
		.then(brace(loose_list(choice((
			slot_fn.map(Member::Fn),
			struct_field.clone().map(Member::Field),
			func.clone().map(Member::Fn),
		)))))
		.map_with(|((name, supers), members), ex| {
			let (fields, methods, _) = split_members(members);
			(
				Expr::TraitDef {
					name,
					supers,
					fields,
					methods,
				},
				ex.span(),
			)
		})
		.boxed();

	// impl blocks
	let bare_fill = ident()
		.then_ignore(just(Token::DoubleColon))
		.then(block.clone())
		.map_with(|(name, body), ex| fn_def((name, vec![]), Some((vec![], false)), None, body, ex.span()));
	// associated consts
	let const_fill = ident()
		.then_ignore(just(Token::DoubleColon))
		.then(expr.clone())
		.map_with(|(name, v), ex| {
			(
				Expr::Bind {
					mutable: false,
					name,
					typ: None,
					value: Some(Box::new(v)),
				},
				ex.span(),
			)
		});
	let fill_docs = select! { Token::Doc(_) => () }.or(just(Token::DocBreak).ignored()).repeated();
	let fill = just(Token::Pub)
		.or_not()
		.then(func.clone().or(bare_fill).or(const_fill))
		.map_with(|(p, f), ex| match p {
			Some(_) => (Expr::Pub(Box::new(f)), ex.span()),
			None => f,
		});
	let fill = annotations.clone().or_not().then(fill).map_with(|(anns, f), ex| match anns {
		Some(anns) => (Expr::Annotated(anns, Box::new(f)), ex.span()),
		None => f,
	});
	let fill_block = brace(
		fill_docs
			.clone()
			.ignore_then(fill)
			.repeated()
			.collect::<Vec<_>>()
			.then_ignore(fill_docs),
	);
	let via = just(Token::Via).ignore_then(ident()).or_not();
	// arrays and maps
	let bracket_head = bracket(ident().or_not()).then(ident()).map(|(k, v)| {
		let names: Vec<_> = k.into_iter().chain([v]).map(|name| TypeParam { name, bound: None }).collect();
		(if names.len() == 1 { "array" } else { "map" }.into(), names)
	});
	let head = ident().then(type_params.clone()).or(bracket_head).boxed();
	let claim = head
		.clone()
		.then_ignore(just(Token::Colon))
		.then(list(ident()))
		.then(via.clone())
		.then_ignore(just(Token::Lt))
		.then(fill_block)
		.or(head
			.then_ignore(just(Token::Colon))
			.then_ignore(just(Token::Lt))
			.then(ident().separated_by(just(Token::Comma)).at_least(1).collect::<Vec<_>>())
			.then(via)
			.map(|(head, via)| ((head, via), vec![])))
		.map_with(|((((typ, type_params), traits), via), fills), ex| {
			(
				Expr::Claim {
					typ,
					type_params,
					traits,
					via,
					fills,
				},
				ex.span(),
			)
		})
		.boxed();

	let def = func
		.clone()
		.or(tuple_struct_def)
		.or(struct_def)
		.or(enum_def)
		.or(trait_def)
		.or(claim)
		.or(type_alias)
		.boxed();
	item.define(def.clone());
	let module_decl = just(Token::Module)
		.ignore_then(ident())
		.map_with(|name, ex| (Expr::Module(name), ex.span()));
	// imports
	let use_item = spanned(ident())
		.then(just(Token::DoubleColon).ignore_then(spanned(ident())).or_not())
		.map(|(local, rename_of)| UseItem { local, rename_of });
	let use_decl = spanned(ident())
		.then_ignore(just(Token::DoubleColon))
		.or_not()
		.then_ignore(just(Token::Use))
		.then(spanned(ident()).separated_by(just(Token::Dot)).at_least(1).collect())
		.then(just(Token::Dot).ignore_then(brace(loose_list(use_item))).or_not())
		.map_with(|((name, path), group), ex| (Expr::Use { name, path, group }, ex.span()))
		.boxed();
	let public = just(Token::Pub)
		.ignore_then(def.clone().or(use_decl.clone()).or(bind.clone()).or(macro_def))
		.map_with(|d, ex| (Expr::Pub(Box::new(d)), ex.span()));

	// annotations
	let annotated = annotations.then(just(Token::Pub).or_not()).then(def.clone().or(bind)).map_with(
		|((anns, public), item), ex| {
			let item = match public {
				Some(_) => (Expr::Pub(Box::new(item)), ex.span()),
				None => item,
			};
			(Expr::Annotated(anns, Box::new(item)), ex.span())
		},
	);

	// annotation macros
	let attr_macro = just(Token::At)
		.then_ignore(adjacent)
		.ignore_then(dotted_name)
		.then_ignore(adjacent)
		.then_ignore(just(Token::Not))
		.then(spanned(adjacent.ignore_then(paren(loose_list(expr.clone())))).or_not())
		.then(annotated.clone().or(def.clone()))
		.map_with(|((name, args), item), ex| {
			let mut args_v = vec![item];
			if let Some((elems, span)) = args {
				args_v.push((Expr::Array(elems), span));
			}
			(Expr::MacroCall { name, args: args_v }, ex.span())
		});

	attr_macro
		.or(annotated)
		.or(def)
		.or(public)
		.or(module_decl)
		.or(use_decl)
		.or(stmt)
		.repeated()
		.collect()
		.then_ignore(end())
}
