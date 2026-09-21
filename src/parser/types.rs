use super::{P, brace, bracket, ident, loose_list, paren, spanned};
use crate::ast::{Access, Expr, Param, Spanned, TypeExpr};
use crate::lexer::Token;

use chumsky::{input::ValueInput, prelude::*};

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
			let fn_ret = same_line.clone().ignore_then(base.clone()).or_not();
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
				.ignore_then(base.clone().or_not())
				.map(|t| TypeExpr::Result(Box::new(t.unwrap_or(TypeExpr::Tuple(vec![]))), None));
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
			.then(same_line.clone().ignore_then(just(Token::Not)).ignore_then(base).or_not())
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
	.boxed()
}
