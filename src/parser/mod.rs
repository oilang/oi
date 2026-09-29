use crate::ast::{Access, BinOp, Child, EnumVariant, Expr, Param, Span, Spanned, TypeExpr, TypeParam};
use crate::lexer::Token;

use chumsky::{Boxed, input::ValueInput, prelude::*, recursive::Indirect};

mod definition;
mod item;
mod types;

// Every parser passed across a fn boundary in here wears this shape.
pub(super) type P<'t, I, O> = Boxed<'t, 't, I, O, extra::Err<Rich<'t, Token>>>;
// A Recursive handle passed across a fn boundary so it can be `.define()`'d there.
pub(super) type Rec<'t, I, O> = Recursive<Indirect<'t, 't, I, O, extra::Err<Rich<'t, Token>>>>;

fn ident<'token, I>() -> impl Parser<'token, I, String, extra::Err<Rich<'token, Token>>> + Copy
where
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
	select! { Token::Ident(name) => name }
}
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

// The parsers the grammars borrow from each other.
pub(super) struct Parsers<'token, I>
where
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
	pub(super) access: P<'token, I, Access>,
	pub(super) def_name: P<'token, I, String>,
	pub(super) dotted_name: P<'token, I, String>,
	pub(super) lit_path: P<'token, I, String>,
	pub(super) same_line: P<'token, I, ()>,
	pub(super) adjacent: P<'token, I, ()>,
	pub(super) type_expr: P<'token, I, TypeExpr>,
	pub(super) annot: P<'token, I, Spanned<TypeExpr>>,
	pub(super) type_params: P<'token, I, Vec<TypeParam>>,
	pub(super) params: P<'token, I, (Vec<Param>, bool)>,
	pub(super) param_hole: P<'token, I, Param>,
	pub(super) ret: P<'token, I, Option<Spanned<TypeExpr>>>,
	pub(super) fn_ret: P<'token, I, (Option<Bound>, Option<Spanned<TypeExpr>>)>,
	pub(super) bind_default: P<'token, I, (TypeExpr, Option<Spanned<Expr>>)>,
	pub(super) annotation: P<'token, I, Spanned<Expr>>,
	pub(super) annotations: P<'token, I, Vec<Spanned<Expr>>>,
	pub(super) unquote: P<'token, I, Spanned<Expr>>,
	pub(super) pat: P<'token, I, Spanned<Expr>>,
	pub(super) pat_name: P<'token, I, Spanned<Expr>>,
	pub(super) bind: P<'token, I, Spanned<Expr>>,
	pub(super) place: P<'token, I, Spanned<Expr>>,
	pub(super) macro_def: P<'token, I, Spanned<Expr>>,
	pub(super) block_ast: P<'token, I, Spanned<Expr>>,
	pub(super) block: P<'token, I, Vec<Spanned<Expr>>>,
	pub(super) stmt: P<'token, I, Spanned<Expr>>,
	pub(super) item: P<'token, I, Spanned<Expr>>,
	pub(super) expr: P<'token, I, Spanned<Expr>>,
}

// One entry of a struct/enum/trait body.
enum Member {
	Field(Param),
	Fn(Spanned<Expr>),
	Variant(EnumVariant),
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

// Params using `:=`/`=` are mutable copies, treated as `x := x` shadows.
fn shadow_params(params: &[Param], mut body: Vec<Spanned<Expr>>) -> Vec<Spanned<Expr>> {
	let copies = params.iter().filter(|p| p.mutable).map(|p| {
		let decl = Expr::Bind {
			mutable: true,
			name: p.name.clone(),
			typ: None,
			value: Some(Box::new((Expr::Ident(p.name.clone()), p.span))),
		};
		(decl, p.span)
	});
	body.splice(0..0, copies);
	body
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
			default: None,
		});
		let param = Param {
			name: "$".into(),
			typ: TypeExpr::Name("$I".into()),
			span,
			default: None,
			access: Access::Read,
			mutable: false,
			public: false,
			annotations: vec![],
		};
		(vec![param], false)
	});
	let body = shadow_params(&params, body);
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

pub fn parser<'src, 'token, I>(
	src: &'src str,
	origin: usize,
) -> impl Parser<'token, I, Vec<Spanned<Expr>>, extra::Err<Rich<'token, Token>>>
where
	'src: 'token,
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
	let mut expr = Recursive::declare();
	let juxt_expr = Recursive::declare();
	let header_expr = Recursive::declare();
	let header_cond = Recursive::declare();
	let mut block = Recursive::declare();
	let anon_fields = Recursive::declare();
	let item = Recursive::declare();
	let attr_macro = Recursive::declare();

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
		dotted_name
			.clone()
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
	let type_expr = types::type_expr(
		dotted_name.clone().boxed(),
		access.clone(),
		same_line.boxed(),
		annotation.clone(),
		unquote.clone(),
		anon_fields.clone().boxed(),
	);

	// defaults
	let default_value = expr.clone().map(|v| (TypeExpr::Infer(Box::new(v.clone())), Some(v))).boxed();
	let bind_default = just(Token::Bind).ignore_then(default_value.clone()).boxed();

	let param_type = just(Token::Colon)
		.ignore_then(type_expr.clone())
		.then(one_of([Token::Assign, Token::Colon]).then(expr.clone()).or_not())
		.map(|(typ, def)| match def {
			Some((tok, e)) => (typ, Some(e), tok == Token::Assign),
			None => (typ, None, false),
		})
		.or(one_of([Token::Bind, Token::DoubleColon])
			.then(default_value)
			.map(|(tok, (typ, default))| (typ, default, tok == Token::Bind)))
		.boxed();
	let param = access
		.clone()
		.or_not()
		.then(ident())
		.then(param_type.clone().or_not())
		.map_with(|((access, name), typed), ex| {
			let (typ, default, mutable) = match typed {
				Some((t, d, m)) => (Some(t), d, m),
				None => (None, None, false),
			};
			Param {
				typ: typ.unwrap_or_else(|| TypeExpr::Name(if name == "self" { "Self" } else { "$?" }.into())),
				name,
				span: ex.span(),
				default,
				access: access.unwrap_or_default(),
				mutable,
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
		mutable: false,
		public: false,
		annotations: vec![],
	});
	let name_hole = just(Token::Percent)
		.then_ignore(adjacent)
		.ignore_then(brace(ident()))
		.then(param_type)
		.map_with(|(name, (typ, default, mutable)), ex| Param {
			name: format!("%{name}"),
			typ,
			span: ex.span(),
			default,
			access: Access::Read,
			mutable,
			public: false,
			annotations: vec![],
		});
	let param = name_hole.or(param_hole.clone()).or(param).boxed();
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
		.then(just(Token::Assign).ignore_then(spanned(type_expr.clone())).or_not())
		.map(|((name, bound), default)| TypeParam { name, bound, default });
	let type_params = bracket(list(type_param)).or_not().map(Option::unwrap_or_default).boxed();

	let block_ast = block.clone().map_with(|body, ex| (Expr::Block(body), ex.span()));

	// bindings
	let annot = spanned(type_expr.clone());
	// macro bindings
	let hole_ident = just(Token::Percent)
		.then_ignore(adjacent)
		.ignore_then(ident())
		.map(|n| format!("%{n}"));
	let def_name = ident().or(hole_ident.clone()).boxed();
	let path = ident().separated_by(just(Token::Dot)).at_least(1).collect::<Vec<_>>();
	let lit_path = path.map(|p| p.join(".")).or(hole_ident.clone()).boxed();
	let bind_name = just(Token::Percent)
		.then_ignore(adjacent)
		.ignore_then(
			ident()
				.map(|n| (format!("%{n}"), None))
				.or(brace(expr.clone()).map(|e| ("%".to_string(), Some(e)))),
		)
		.or(ident().map(|n| (n, None)))
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
	let fold = |op, lhs, value: Spanned<Expr>, span| match op {
		None => value,
		Some(op) => (Expr::Binary(op, Box::new((lhs, span)), Box::new(value)), span),
	};

	// assignment
	let mut assign = Recursive::declare();
	assign.define(
		ident()
			.then(assign_op.clone())
			.then(assign.clone().or(juxt_expr.clone()))
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
		just(Token::Return).ignore_then(juxt_expr.clone().or_not()),
		just(Token::BareReturn).to(None),
	))
	.map_with(|value, ex| (Expr::Return(value.map(Box::new)), ex.span()));

	// index assignment
	let index_assign = ident()
		.then(bracket(expr.clone()))
		.then(assign_op.clone())
		.then(juxt_expr.clone())
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
		.then(ident().or(select! { Token::Int(n) => n.to_string() }))
		.then(assign_op)
		.then(juxt_expr.clone())
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

	// binding grammar
	let binds = |value: P<'token, I, Spanned<Expr>>, pat: P<'token, I, Spanned<Expr>>| {
		let value_tail = just(Token::Bind)
			.to(true)
			.or(just(Token::DoubleColon).to(false))
			.then(value.clone())
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
	};
	let (bind, destructure) = binds(juxt_expr.clone().boxed(), pat.clone());

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
		.then_ignore(adjacent.not())
		.then(
			expr.clone()
				.then(same_line.ignore_then(block_ast.clone()).or_not())
				.map(|(arg, body)| std::iter::once(arg).chain(body).collect::<Vec<_>>())
				.or(block_ast.clone().map(|body| vec![body])),
		)
		.map_with(|(name, args), ex| (Expr::MacroCall { name, args }, ex.span()));

	// statements that leave a place behind
	let place = destructure
		.or(bind.clone())
		.or(field_assign)
		.or(assign.clone())
		.or(index_assign)
		.or(map_delete)
		.boxed();

	let defer_stmt = just(Token::Defer)
		.ignore_then(just(Token::Or).or_not())
		.then(place.clone().or(juxt_expr.clone()))
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
	let stmt = doc
		.or(ret_stmt)
		.or(defer_stmt)
		.or(place.clone())
		.or(macro_def.clone())
		.or(macro_stmt)
		.or(juxt_expr.clone())
		.boxed();

	// blocks
	let do_body = just(Token::Do).ignore_then(stmt.clone()).map(|s| vec![s]);
	block.define(brace(stmt.clone().repeated().collect::<Vec<_>>()).or(do_body));

	let p = Parsers {
		access,
		def_name,
		dotted_name: dotted_name.boxed(),
		lit_path,
		same_line: same_line.boxed(),
		adjacent: adjacent.boxed(),
		annot: annot.clone().boxed(),
		type_expr,
		type_params,
		params,
		param_hole: param_hole.boxed(),
		ret: ret.boxed(),
		fn_ret,
		bind_default,
		annotation,
		annotations,
		unquote,
		pat,
		pat_name: pat_name.boxed(),
		bind,
		place,
		macro_def: macro_def.boxed(),
		block_ast: block_ast.boxed(),
		block: block.clone().boxed(),
		stmt,
		item: item.clone().boxed(),
		expr: expr.clone().boxed(),
	};
	expr.define(definition::definition(&p, binds, header_expr, header_cond, juxt_expr));
	item::item(&p, item, attr_macro, anon_fields)
}
