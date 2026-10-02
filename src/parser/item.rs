use super::{
	Member, P, Parsers, Rec, brace, bracket, fn_def, ident, list, loose_list, named_ret, paren, spanned, split_members,
};
use crate::ast::{Access, EnumVariant, Expr, Param, Spanned, TypeExpr, TypeParam, UseItem};
use crate::lexer::Token;

use chumsky::{input::ValueInput, prelude::*};

// The item definition grammar.
pub(super) fn item<'token, I>(
	p: &Parsers<'token, I>,
	mut item: Rec<'token, I, Spanned<Expr>>,
	mut attr_macro: Rec<'token, I, Spanned<Expr>>,
	mut anon_fields: Rec<'token, I, Vec<Param>>,
) -> P<'token, I, Vec<Spanned<Expr>>>
where
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
	let item_head = p
		.def_name
		.clone()
		.then(p.type_params.clone())
		.then_ignore(just(Token::DoubleColon));
	let fill_docs = select! { Token::Doc(_) => () }.or(just(Token::DocBreak).ignored()).repeated();

	// fn defs
	let func = item_head
		.clone()
		.then_ignore(just(Token::Fn))
		.then(p.params.clone())
		.then(p.fn_ret.clone())
		.then(p.block.clone())
		.then_ignore(just(Token::Pipeline).not())
		.validate(|(((head, params), (bound, ret)), body), ex, emitter| {
			if let Some((name, ((_, at), _))) = &bound
				&& params.0.iter().any(|p| p.name == *name)
			{
				emitter.emit(Rich::custom(*at, format!("duplicate argument `{name}`")));
			}
			let ret = bound.as_ref().map(|(_, (typ, _))| typ.clone()).or(ret);
			fn_def(head, Some(params), ret, named_ret(bound, body, ex.span()), ex.span())
		})
		.boxed();

	// struct defs
	let field_typed = just(Token::Colon)
		.ignore_then(p.type_expr.clone())
		.then(just(Token::Assign).ignore_then(p.expr.clone()).or_not());
	let struct_field = just(Token::Pub)
		.or_not()
		.then(ident())
		.then(field_typed.or(p.bind_default.clone()))
		.then(
			p.same_line
				.clone()
				.ignore_then(p.annotation.clone())
				.repeated()
				.collect::<Vec<_>>(),
		)
		.map_with(|(((public, name), (typ, default)), annotations), ex| Param {
			name,
			typ,
			span: ex.span(),
			default,
			access: Access::Read,
			mutable: false,
			public: public.is_some(),
			annotations,
		})
		.boxed();
	let struct_field = p.param_hole.clone().or(struct_field).boxed();
	anon_fields.define(loose_list(struct_field.clone()));
	// embedded structs
	let embedded = just(Token::Pub).or_not().then(ident()).map_with(|(public, name), ex| Param {
		typ: TypeExpr::Name(name.clone()),
		name,
		span: ex.span(),
		default: None,
		access: Access::Read,
		mutable: false,
		public: public.is_some(),
		annotations: vec![],
	});
	let struct_def = item_head
		.clone()
		.then_ignore(just(Token::Struct))
		.then(brace(
			loose_list(
				fill_docs.clone().ignore_then(
					struct_field
						.clone()
						.map(Member::Field)
						.or(func.clone().map(Member::Fn))
						.or(attr_macro.clone().map(Member::Fn))
						.or(embedded.map(Member::Field)),
				),
			)
			.then_ignore(fill_docs.clone()),
		))
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
		.then(p.type_expr.clone())
		.map(|(n, t)| (Some(n), t))
		.or(p.type_expr.clone().map(|t| (None, t)));
	let plain_head = ident().then(p.type_params.clone()).then_ignore(just(Token::DoubleColon));
	let tuple_struct_def = plain_head
		.clone()
		.then_ignore(just(Token::Struct))
		.then(paren(
			ts_field
				.separated_by(just(Token::Comma))
				.allow_trailing()
				.at_least(1)
				.collect::<Vec<_>>(),
		))
		.map_with(|((name, type_params), fields), ex| {
			let typ = TypeExpr::TupleStruct(name.clone(), fields);
			(Expr::TypeAlias { name, type_params, typ }, ex.span())
		})
		.boxed();

	// enum defs
	let disc = just(Token::Assign).ignore_then(p.expr.clone()).map(|e| match e {
		(Expr::String(s), _) => (None, Some(s)),
		e => (Some(e), None),
	});
	let fields = brace(
		loose_list(
			fill_docs
				.clone()
				.ignore_then(ident().then_ignore(just(Token::Colon)).then(p.annot.clone())),
		)
		.then_ignore(fill_docs.clone()),
	);
	let backing = just(Token::DoubleColon).to(None).or(just(Token::Colon)
		.ignore_then(p.annot.clone())
		.then_ignore(just(Token::Colon))
		.map(Some));
	let payload = paren(
		p.annot
			.clone()
			.separated_by(just(Token::Comma))
			.allow_trailing()
			.collect::<Vec<_>>(),
	);
	let variant = ident()
		.then(
			payload
				.map(|p| (vec![], p))
				.or(fields.map(|fs| fs.into_iter().unzip()))
				.or_not(),
		)
		.then(disc.or_not())
		.map_with(|((name, body), assign), ex| {
			let (names, payload) = body.unwrap_or_default();
			let (disc, raw) = assign.unwrap_or((None, None));
			EnumVariant {
				name,
				span: ex.span(),
				disc,
				raw,
				payload,
				names,
			}
		});
	let enum_def = p
		.def_name
		.clone()
		.then(p.type_params.clone())
		.then(backing)
		.then_ignore(just(Token::Enum))
		.then(brace(
			loose_list(
				fill_docs.clone().ignore_then(
					func.clone()
						.or(p.unquote.clone())
						.map(Member::Fn)
						.or(variant.map(Member::Variant)),
				),
			)
			.then_ignore(fill_docs.clone()),
		))
		.validate(|(((name, type_params), backing), members), ex, emitter| {
			let (_, fills, variants) = split_members(members);
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
			TypeExpr::Ref(t) => expr_shaped(t),
			_ => false,
		}
	}
	let type_alias = plain_head
		.then(p.type_expr.clone())
		.filter(|(_, typ)| !expr_shaped(typ))
		.then_ignore(
			one_of([
				Token::LBrace,
				Token::LParen,
				Token::LBracket,
				Token::Dot,
				Token::SpaceDot,
				Token::DotDot,
				Token::Question,
				Token::Pipeline,
				Token::DoubleColon,
				Token::Bind,
				Token::Assign,
			])
			.not(),
		)
		.map_with(|((name, type_params), typ), ex| (Expr::TypeAlias { name, type_params, typ }, ex.span()));

	// trait definitions
	let slot_fn = ident()
		.then_ignore(just(Token::Colon))
		.then_ignore(just(Token::Fn))
		.then(p.params.clone())
		.then(p.ret.clone())
		.map_with(|((name, params), ret), ex| fn_def((name, vec![]), Some(params), ret, vec![], ex.span()));
	let supers = just(Token::DoubleColon)
		.to(vec![])
		.or(just(Token::Colon).ignore_then(list(ident())).then_ignore(just(Token::Colon)));
	let default_fn = func.clone().map(|(mut e, sp)| {
		if let Expr::Fn { body, .. } = &mut e
			&& body.is_empty()
		{
			body.push((Expr::Tuple(vec![]), sp));
		}
		(e, sp)
	});
	let trait_def = ident()
		.then(p.type_params.clone())
		.then(supers)
		.then_ignore(just(Token::Trait))
		.then(brace(loose_list(choice((
			slot_fn.map(Member::Fn),
			struct_field.clone().map(Member::Field),
			default_fn.map(Member::Fn),
		)))))
		.map_with(|(((name, type_params), supers), members), ex| {
			let (fields, methods, _) = split_members(members);
			(
				Expr::TraitDef {
					name,
					type_params,
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
		.then(p.block.clone())
		.map_with(|(name, body), ex| fn_def((name, vec![]), Some((vec![], false)), None, body, ex.span()));
	// associated consts
	let const_fill = ident()
		.then_ignore(just(Token::DoubleColon))
		.then(p.expr.clone())
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
	let fill = just(Token::Pub)
		.or_not()
		.then(func.clone().or(p.unquote.clone()).or(bare_fill).or(const_fill))
		.map_with(|(pub_, f), ex| match pub_ {
			Some(_) => (Expr::Pub(Box::new(f)), ex.span()),
			None => f,
		});
	let fill = p.annotations.clone().or_not().then(fill).map_with(|(anns, f), ex| match anns {
		Some(anns) => (Expr::Annotated(anns, Box::new(f)), ex.span()),
		None => f,
	});
	let fill = attr_macro.clone().or(fill);
	// field amendments
	let fill = fill.map(Member::Fn).or(struct_field.clone().map(Member::Field));
	let fill_block = brace(
		fill_docs
			.clone()
			.ignore_then(fill)
			.repeated()
			.collect::<Vec<_>>()
			.then_ignore(fill_docs),
	)
	.map(|ms| {
		let (fields, fills, _) = split_members(ms);
		(fills, fields)
	});
	let via = just(Token::Via).ignore_then(ident()).or_not();
	let trait_ref = ident()
		.then(bracket(list(spanned(p.type_expr.clone()))).or_not())
		.map(|(name, args)| (name, args.unwrap_or_default()));
	// arrays and maps
	let bracket_head = bracket(ident().or_not()).then(ident()).map(|(k, v)| {
		let names: Vec<_> = k
			.into_iter()
			.chain([v])
			.map(|name| TypeParam {
				name,
				bound: None,
				default: None,
			})
			.collect();
		(if names.len() == 1 { "array" } else { "map" }.into(), names)
	});
	let head = p.def_name.clone().then(p.type_params.clone()).or(bracket_head).boxed();
	let claim = head
		.clone()
		.then_ignore(just(Token::Colon))
		.then(list(trait_ref.clone()))
		.then(via.clone())
		.then_ignore(just(Token::Lt))
		.then(fill_block)
		.or(head
			.then_ignore(just(Token::Colon))
			.then_ignore(just(Token::Lt))
			.then(trait_ref.separated_by(just(Token::Comma)).at_least(1).collect::<Vec<_>>())
			.then(via)
			.map(|(head, via)| ((head, via), (vec![], vec![]))))
		.map_with(|((((typ, type_params), traits), via), (fills, fields)), ex| {
			(
				Expr::Claim {
					typ,
					type_params,
					traits,
					via,
					fills,
					fields,
				},
				ex.span(),
			)
		})
		.boxed();

	let def = func
		.or(tuple_struct_def)
		.or(struct_def)
		.or(enum_def)
		.or(trait_def)
		.or(claim)
		.or(type_alias)
		.boxed();
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
		.ignore_then(def.clone().or(use_decl.clone()).or(p.bind.clone()).or(p.macro_def.clone()))
		.map_with(|d, ex| (Expr::Pub(Box::new(d)), ex.span()))
		.boxed();
	// annotations
	let annotated = p
		.annotations
		.clone()
		.then(just(Token::Pub).or_not())
		.then(def.clone().or(p.bind.clone()))
		.map_with(|((anns, public), item), ex| {
			let item = match public {
				Some(_) => (Expr::Pub(Box::new(item)), ex.span()),
				None => item,
			};
			(Expr::Annotated(anns, Box::new(item)), ex.span())
		});
	item.define(annotated.clone().or(public.clone()).or(def.clone()));

	// annotation macros
	let attr = just(Token::At)
		.then_ignore(p.adjacent.clone())
		.ignore_then(p.dotted_name.clone())
		.then_ignore(p.adjacent.clone())
		.then_ignore(just(Token::Not))
		.then(spanned(p.adjacent.clone().ignore_then(paren(loose_list(p.expr.clone())))).or_not())
		.then(item.clone().or(p.bind.clone()))
		.map_with(|((name, args), item), ex| {
			let mut args_v = vec![item];
			if let Some((elems, span)) = args {
				args_v.push((Expr::Array(elems), span));
			}
			(Expr::MacroCall { name, args: args_v }, ex.span())
		});
	attr_macro.define(attr);

	attr_macro
		.or(annotated)
		.or(def)
		.or(public)
		.or(module_decl)
		.or(use_decl)
		.or(p.stmt.clone())
		.repeated()
		.collect()
		.then_ignore(end())
		.boxed()
}
