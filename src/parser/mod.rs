use crate::ast::{Access, Child, EnumVariant, Expr, Param, Span, Spanned, TypeExpr, TypeParam};
use crate::lexer::Token;

use chumsky::{Boxed, input::ValueInput, prelude::*, recursive::Indirect};

mod annotation;
mod definition;
mod item;
mod stmt;
mod types;

type Err<'t> = extra::Err<Rich<'t, Token>>;
// Every parser passed across a fn boundary in here wears this shape.
pub(super) type P<'t, I, O> = Boxed<'t, 't, I, O, Err<'t>>;
// A Recursive handle passed across a fn boundary so it can be defined there.
pub(super) type Rec<'t, I, O> = Recursive<Indirect<'t, 't, I, O, Err<'t>>>;

// trait aliases, so the token and error bounds are spelled once
pub(super) trait Tokens<'t>: ValueInput<'t, Token = Token, Span = SimpleSpan> {}
impl<'t, I: ValueInput<'t, Token = Token, Span = SimpleSpan>> Tokens<'t> for I {}
pub(super) trait Rule<'t, I: Tokens<'t>, O>: Parser<'t, I, O, Err<'t>> + Clone {}
impl<'t, I: Tokens<'t>, O, T: Parser<'t, I, O, Err<'t>> + Clone> Rule<'t, I, O> for T {}

fn ident<'token, I: Tokens<'token>>() -> impl Rule<'token, I, String> + Copy {
	select! { Token::Ident(name) => name }
}

fn paren<'token, I: Tokens<'token>, O>(p: impl Rule<'token, I, O>) -> impl Rule<'token, I, O> {
	p.delimited_by(just(Token::LParen), just(Token::RParen))
}

fn brace<'token, I: Tokens<'token>, O>(p: impl Rule<'token, I, O>) -> impl Rule<'token, I, O> {
	p.delimited_by(just(Token::LBrace), just(Token::RBrace))
}

fn bracket<'token, I: Tokens<'token>, O>(p: impl Rule<'token, I, O>) -> impl Rule<'token, I, O> {
	p.delimited_by(just(Token::LBracket), just(Token::RBracket))
}

fn list<'token, I: Tokens<'token>, O>(p: impl Rule<'token, I, O>) -> impl Rule<'token, I, Vec<O>> {
	p.separated_by(just(Token::Comma)).collect()
}

fn list1<'token, I: Tokens<'token>, O>(p: impl Rule<'token, I, O>) -> impl Rule<'token, I, Vec<O>> {
	p.separated_by(just(Token::Comma)).at_least(1).collect()
}

fn trailing_list<'token, I: Tokens<'token>, O>(p: impl Rule<'token, I, O>) -> impl Rule<'token, I, Vec<O>> {
	p.separated_by(just(Token::Comma)).allow_trailing().collect()
}

fn trailing_list1<'token, I: Tokens<'token>, O>(p: impl Rule<'token, I, O>) -> impl Rule<'token, I, Vec<O>> {
	p.separated_by(just(Token::Comma)).allow_trailing().at_least(1).collect()
}

fn loose_list<'token, I: Tokens<'token>, O>(p: impl Rule<'token, I, O>) -> impl Rule<'token, I, Vec<O>> {
	p.separated_by(just(Token::Comma).or_not()).allow_trailing().collect()
}

fn semi<'token, I: Tokens<'token>, O>(p: impl Rule<'token, I, O>) -> impl Rule<'token, I, O> {
	p.then_ignore(just(Token::Semicolon).or_not())
}

// Matrix literals.
fn matrix<'token, I: Tokens<'token>>(
	p: impl Rule<'token, I, Spanned<Expr>>,
) -> impl Rule<'token, I, Vec<Spanned<Expr>>> {
	let row = loose_list(p).map_with(|row, ex| (Expr::DotArray(None, row), ex.span()));
	row.separated_by(just(Token::Semicolon))
		.collect::<Vec<_>>()
		.map(|mut rows| match rows.as_mut_slice() {
			[(Expr::DotArray(_, row), _)] => std::mem::take(row),
			_ => rows,
		})
}

fn loose_list1<'token, I: Tokens<'token>, O>(p: impl Rule<'token, I, O>) -> impl Rule<'token, I, Vec<O>> {
	p.separated_by(just(Token::Comma).or_not())
		.allow_trailing()
		.at_least(1)
		.collect()
}

fn spanned<'token, I: Tokens<'token>, O>(p: impl Rule<'token, I, O>) -> impl Rule<'token, I, Spanned<O>> {
	p.map_with(|o, ex| (o, ex.span()))
}

// Whether a binder pair like `:=`/`::` is mutable.
fn binder<'token, I: Tokens<'token>>(pair: [Token; 2]) -> impl Rule<'token, I, bool> {
	one_of(pair).map(|t| matches!(t, Token::Bind | Token::Assign))
}

// A binding's tail after its name, as (mutable, type, value).
fn binding<'token, I: Tokens<'token>>(
	annot: impl Rule<'token, I, Spanned<TypeExpr>>,
	value: impl Rule<'token, I, Spanned<Expr>>,
) -> impl Rule<'token, I, (bool, Option<Spanned<TypeExpr>>, Option<Spanned<Expr>>)> {
	let typed = just(Token::Colon)
		.ignore_then(annot)
		.then(binder([Token::Assign, Token::Colon]).then(value.clone()).or_not());
	binder([Token::Bind, Token::DoubleColon])
		.then(value)
		.map(|(mutable, value)| (mutable, None, Some(value)))
		.or(typed.map(|(typ, tail)| match tail {
			Some((mutable, value)) => (mutable, Some(typ), Some(value)),
			None => (true, Some(typ), None),
		}))
}

// The parsers the grammars borrow from each other.
pub(super) struct Parsers<'token, I: Tokens<'token>> {
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
	pub(super) param_type: P<'token, I, (TypeExpr, Option<Spanned<Expr>>, bool)>,
	pub(super) annotation: P<'token, I, Spanned<Expr>>,
	pub(super) annotations: P<'token, I, Vec<Spanned<Expr>>>,
	pub(super) unquote: P<'token, I, Spanned<Expr>>,
	pub(super) pat: P<'token, I, Spanned<Expr>>,
	pub(super) bind: P<'token, I, Spanned<Expr>>,
	pub(super) place: P<'token, I, Spanned<Expr>>,
	pub(super) macro_def: P<'token, I, Spanned<Expr>>,
	pub(super) block_ast: P<'token, I, Spanned<Expr>>,
	pub(super) block: P<'token, I, Vec<Spanned<Expr>>>,
	pub(super) stmt: P<'token, I, Spanned<Expr>>,
	pub(super) item: P<'token, I, Spanned<Expr>>,
	pub(super) expr: P<'token, I, Spanned<Expr>>,
	pub(super) juxt_expr: P<'token, I, Spanned<Expr>>,
}

// One entry of a struct/enum/trait body.
enum Member {
	Field(Param),
	Fn(Spanned<Expr>),
	Variant(EnumVariant),
}

// Give each untyped param the type and default of the nearest typed one to its right.
fn group_untyped(group: &mut [Param]) {
	let mut typed: Option<Param> = None;
	for param in group.iter_mut().rev() {
		if !matches!(&param.typ, TypeExpr::Name(n) if n == "$?") {
			typed = Some(param.clone());
		} else if let Some(t) = &typed {
			(param.typ, param.default, param.mutable) = (t.typ.clone(), t.default.clone(), t.mutable);
		}
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
	group_untyped(&mut fields);
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
	let withs = params
		.iter()
		.filter(|p| p.with)
		.map(|p| (Expr::With(vec![(Expr::Ident(p.name.clone()), p.span)]), p.span));
	let pats = params.iter().filter_map(|p| {
		let value = Box::new((Expr::Ident(p.name.clone()), p.span));
		let pat = p.pattern.clone()?;
		Some((
			Expr::PatBind {
				pat,
				value,
				mutable: Some(false),
			},
			p.span,
		))
	});
	body.splice(0..0, pats.chain(copies).chain(withs));
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
			..TypeParam::default()
		});
		(vec![Param::new("$".into(), TypeExpr::Name("$I".into()), span)], false)
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

pub fn parser<'src, 'token, I>(src: &'src str, origin: usize) -> impl Parser<'token, I, Vec<Spanned<Expr>>, Err<'token>>
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
	let stmt = Recursive::declare();
	let place = Recursive::declare();
	let bind = Recursive::declare();

	// param access modifiers
	let access = choice((just(Token::Mut).to(Access::Mut), just(Token::Move).to(Access::Move))).boxed();

	let dotted_name = ident()
		.then(just(Token::Dot).ignore_then(ident()).or_not())
		.map(|(first, rest)| match rest {
			Some(second) => format!("{first}.{second}"),
			None => first,
		});

	// a guard on the source text between the previous token and the next
	let gap_guard = move |ok: fn(Option<&str>) -> bool, msg: &'static str| {
		empty().map_with(|_, ex| ex.span()).try_map(move |sp: Span, _| {
			let gap = Span::from(sp.end..sp.start);
			match ok(src.get(gap.start - origin..gap.end - origin)) {
				true => Ok(()),
				false => Err(Rich::custom(gap, msg)),
			}
		})
	};
	// the next token has no token gap following it
	let adjacent = gap_guard(|gap| gap == Some(""), "must immediately follow, with no space");
	// the next token opens on the same line
	let same_line = gap_guard(
		|gap| !gap.is_some_and(|g| g.contains(['\n', ';'])),
		"must continue on the same line",
	);
	// a token glued to the next one
	let sigil = move |tok| just(tok).then_ignore(adjacent);

	let unquote = sigil(Token::Percent)
		.ignore_then(ident().map(Expr::Unquote).or(brace(expr.clone()).map(|e| match e {
			(Expr::Spread(inner), _) => Expr::UnquoteSplat(inner),
			e => Expr::UnquoteExpr(Box::new(e)),
		})))
		.map_with(|e, ex| (e, ex.span()))
		.boxed();

	// annotations
	let annotation = annotation::annotation(expr.clone().boxed(), dotted_name.clone().boxed(), adjacent.boxed());
	let annotations = annotation.clone().repeated().at_least(1).collect::<Vec<_>>().boxed();

	// type annotations
	let type_expr = types::type_expr(
		dotted_name.clone().boxed(),
		access.clone(),
		same_line.boxed(),
		annotation.clone(),
		unquote.clone(),
		anon_fields.clone().boxed(),
		expr.clone().boxed(),
	);

	// bindings
	let annot = spanned(type_expr.clone());
	let default_value = expr.clone().map(|v| (TypeExpr::Infer(Box::new(v.clone())), Some(v))).boxed();
	let bind_default = just(Token::Bind).ignore_then(default_value.clone()).boxed();
	let param_type = binding(annot.clone(), expr.clone())
		.map(|(mutable, typ, default)| match typ {
			Some((typ, _)) => (typ, default.clone(), mutable && default.is_some()),
			None => (
				TypeExpr::Infer(Box::new(default.clone().expect("`:=` has a value"))),
				default,
				mutable,
			),
		})
		.boxed();
	let param = just(Token::With)
		.or_not()
		.then(access.clone().or_not())
		.then(ident())
		.then(param_type.clone().or_not())
		.map_with(|(((with, access), name), typed), ex| {
			let (typ, default, mutable) = match typed {
				Some((t, d, m)) => (Some(t), d, m),
				None => (None, None, false),
			};
			let typ = typ.unwrap_or_else(|| TypeExpr::Name("$?".into()));
			Param {
				default,
				access: access.unwrap_or_default(),
				mutable,
				with: with.is_some(),
				..Param::new(name, typ, ex.span())
			}
		});
	let param_hole = unquote
		.clone()
		.map_with(|u, ex| Param::new(String::new(), TypeExpr::Unquote(Box::new(u)), ex.span()));
	let name_hole = sigil(Token::Percent).ignore_then(brace(ident())).then(param_type.clone());
	let name_hole = name_hole.map_with(|(name, (typ, default, mutable)), ex| Param {
		default,
		mutable,
		..Param::new(format!("%{name}"), typ, ex.span())
	});
	let pat = stmt::patterns();
	let pat_param = pat.clone().then_ignore(just(Token::Colon)).then(type_expr.clone());
	let pat_param = pat_param.map_with(|(pattern, typ), ex| {
		let span: Span = ex.span();
		Param {
			pattern: Some(Box::new(pattern)),
			..Param::new(format!("${}", span.start), typ, span)
		}
	});
	let param = name_hole.or(param_hole.clone()).or(pat_param).or(param).boxed();
	// NOTE: a trailing comma forces a tuple even for one param
	let params = paren(
		(param.separated_by(just(Token::Comma)).collect::<Vec<_>>())
			.separated_by(just(Token::Semicolon))
			.collect::<Vec<_>>()
			.then(just(Token::Comma).or_not()),
	)
	.map(|(groups, trailing)| {
		let params: Vec<Param> = groups
			.into_iter()
			.flat_map(|mut group| {
				group_untyped(&mut group);
				group
			})
			.collect();
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
	let bound = dotted_name.clone().then(bracket(list(spanned(type_expr.clone()))).or_not());
	let type_param = ident()
		.then(just(Token::Colon).ignore_then(bound).or_not())
		.then(just(Token::DoubleColon).ignore_then(spanned(type_expr.clone())).or_not())
		.map(|((name, bound), default)| {
			let (bound, args) = bound.unzip();
			let bound_args = args.flatten().unwrap_or_default();
			TypeParam {
				name,
				bound,
				bound_args,
				default,
			}
		});

	// param grouping
	let type_params = bracket(list(type_param).separated_by(just(Token::Semicolon)).collect::<Vec<_>>())
		.map(|groups| {
			groups
				.into_iter()
				.flat_map(|mut group: Vec<TypeParam>| {
					for at in (1..group.len()).rev() {
						let (bare, next) = (&group[at - 1], &group[at]);
						if bare.bound.is_none() && bare.default.is_none() {
							group[at - 1] = TypeParam {
								name: bare.name.clone(),
								..next.clone()
							};
						}
					}
					group
				})
				.collect()
		})
		.or_not()
		.map(Option::unwrap_or_default)
		.boxed();

	let block_ast = block.clone().map_with(|body, ex| (Expr::Block(body), ex.span()));

	// macro bindings
	let hole_ident = sigil(Token::Percent)
		.ignore_then(ident().or(brace(ident())))
		.map(|n| format!("%{n}"));
	let def_name = ident().or(hole_ident.clone()).boxed();
	let path = ident().separated_by(just(Token::Dot)).at_least(1).collect::<Vec<_>>();
	let lit_path = path.map(|p| p.join(".")).or(hole_ident.clone()).boxed();

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

	// blocks
	let do_body = just(Token::Do).ignore_then(stmt.clone()).map(|s| vec![s]);
	block.define(brace(semi(stmt.clone()).repeated().collect::<Vec<_>>()).or(do_body));

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
		param_type,
		annotation,
		annotations,
		unquote,
		pat,
		bind: bind.clone().boxed(),
		place: place.clone().boxed(),
		macro_def: macro_def.boxed(),
		block_ast: block_ast.boxed(),
		block: block.clone().boxed(),
		stmt: stmt.clone().boxed(),
		item: item.clone().boxed(),
		expr: expr.clone().boxed(),
		juxt_expr: juxt_expr.clone().boxed(),
	};
	expr.define(definition::definition(&p, header_expr, header_cond, juxt_expr));
	stmt::stmt(&p, stmt, place, bind);
	item::item(&p, item, attr_macro, anon_fields)
}
