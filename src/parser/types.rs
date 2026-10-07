use super::{P, brace, bracket, ident, list1, loose_list, loose_list1, paren, spanned};
use crate::ast::{Access, Expr, Param, Spanned, TypeExpr};
use crate::lexer::Token;

use chumsky::{input::ValueInput, prelude::*};

// Construct a Result.
pub(super) fn result_of(ok: TypeExpr, err: Option<TypeExpr>) -> TypeExpr {
	let err = err.unwrap_or_else(|| TypeExpr::Name("Error".into()));
	TypeExpr::Generic("Result".into(), vec![ok, err])
}

// Generic type params.
pub(super) fn type_args<'token, I>(te: P<'token, I, TypeExpr>) -> P<'token, I, Vec<Spanned<TypeExpr>>>
where
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
	bracket(list1(spanned(select! { Token::Int(n) => TypeExpr::Const(n) }.or(te)))).boxed()
}

// Can be settled by the parser without the resolver.
pub(super) fn settled(args: &[Spanned<TypeExpr>]) -> bool {
	args.len() > 1 || !matches!(args[0].0, TypeExpr::Const(_))
}

// The type-expression grammar, boxed so its types stop at this fn boundary.
pub(super) fn type_expr<'token, I>(
	dotted_name: P<'token, I, String>,
	access: P<'token, I, Access>,
	same_line: P<'token, I, ()>,
	annotation: P<'token, I, Spanned<Expr>>,
	unquote: P<'token, I, Spanned<Expr>>,
	anon_fields: P<'token, I, Vec<Param>>,
) -> P<'token, I, TypeExpr>
where
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
	recursive(|te| {
		let base = recursive(|base| {
			let name = dotted_name.clone().map(TypeExpr::Name);
			let unit = just(Token::LParen).then(just(Token::RParen)).to(TypeExpr::unit());
			let tuple_field = ident().then_ignore(just(Token::Colon)).or_not().then(te.clone());
			let tuple = paren(loose_list1(tuple_field)).map(TypeExpr::Tuple);
			// arrays
			let len = select! { Token::Int(n) => Expr::Int(n) }.or(dotted_name.clone().map(Expr::Ident));
			let len = spanned(len).or(unquote.clone());
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
			let fn_ret = same_line.clone().ignore_then(base.clone()).or_not();
			let fn_type = just(Token::Fn)
				.ignore_then(paren(loose_list(fn_param)))
				.then(fn_ret)
				.map(|(params, ret)| TypeExpr::Fn(params, Box::new(ret.unwrap_or(TypeExpr::unit()))));
			// annotations
			let annotated = annotation
				.clone()
				.then(base.clone())
				.map(|(a, t)| TypeExpr::Annotated(vec![a], Box::new(t)));
			// options
			let option = just(Token::Question)
				.ignore_then(base.clone())
				.map(|t| TypeExpr::Generic("Option".into(), vec![t]));
			// varargs
			let variadic = just(Token::DotDot)
				.ignore_then(base.clone())
				.map(|t| TypeExpr::Variadic(Box::new(t)));
			// results
			let result = just(Token::Not)
				.ignore_then(base.clone().or_not())
				.map(|t| result_of(t.unwrap_or(TypeExpr::unit()), None));
			// shared refs
			let ref_type = just(Token::Caret).ignore_then(base.clone()).map(|t| TypeExpr::Ref(Box::new(t)));
			// atom(s)
			let atom = select! { Token::Atom(a) => TypeExpr::AtomSum(vec![a]) };

			// anonymous structs
			let anon_struct = just(Token::Struct)
				.ignore_then(brace(anon_fields.clone()))
				.map(TypeExpr::AnonStruct);

			// generic structs
			let generic_instance = ident()
				.then(type_args(te.clone().boxed()))
				.filter(|(_, args)| settled(args))
				.map(|(name, args)| TypeExpr::Generic(name, args.into_iter().map(|(t, _)| t).collect()));

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
				generic_instance,
				name,
				tuple,
				array,
			))
			.or(ref_type)
		});

		base.clone()
			.then(
				same_line
					.clone()
					.ignore_then(just(Token::Not))
					.ignore_then(base.or_not())
					.or_not(),
			)
			.map(|(e, ok)| match ok {
				Some(ok) => result_of(ok.unwrap_or(TypeExpr::unit()), Some(e)),
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
	.boxed()
}
