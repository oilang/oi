use super::{P, brace, ident, loose_list, paren, spanned};
use crate::ast::{Expr, Spanned};
use crate::lexer::Token;

use chumsky::{input::ValueInput, prelude::*};

enum AnnTail {
	Fields(Vec<(Option<String>, Spanned<Expr>)>),
	Args(Vec<Spanned<Expr>>),
}

// The `@annotation` grammar.
pub(super) fn annotation<'token, I>(
	expr: P<'token, I, Spanned<Expr>>,
	dotted_name: P<'token, I, String>,
	adjacent: P<'token, I, ()>,
) -> P<'token, I, Spanned<Expr>>
where
	I: ValueInput<'token, Token = Token, Span = SimpleSpan>,
{
	let ann_entry = ident().then_ignore(just(Token::Assign)).or_not().then(expr.clone());
	let ann_tag = spanned(select! { Token::Atom(name) => Expr::Atom(name) });

	let ann_tail = just(Token::Dot)
		.ignore_then(brace(loose_list(ann_entry)))
		.map(AnnTail::Fields)
		.or(paren(loose_list(expr.clone())).map(AnnTail::Args));

	let ann_value = spanned(
		dotted_name
			.clone()
			.then_ignore(adjacent.clone().then(just(Token::Not)).not())
			.then(adjacent.clone().ignore_then(ann_tail).or_not())
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
	just(Token::At)
		.then_ignore(adjacent)
		.ignore_then(ann_tag.or(ann_unsafe).or(ann_value))
		.boxed()
}
