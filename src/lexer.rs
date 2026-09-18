use std::fmt;

use chumsky::span::SimpleSpan;
use logos::{Lexer, Logos};

fn lex_block_comment(lex: &mut Lexer<Token>) {
	let src = lex.remainder().as_bytes();
	let mut depth = 1usize;
	let mut i = 0;
	while i < src.len() {
		match (src[i], src.get(i + 1).copied()) {
			(b'#', Some(b'{')) => {
				depth += 1;
				i += 2;
			}
			(b'}', Some(b'#')) => {
				i += 2;
				depth -= 1;
				if depth == 0 {
					break;
				}
			}
			_ => i += 1,
		}
	}
	lex.bump(i);
}

fn parse_radix(lex: &mut Lexer<Token>, radix: u32) -> Option<i64> {
	i64::from_str_radix(&lex.slice()[2..].replace('_', ""), radix).ok()
}

#[derive(Logos, Clone, PartialEq, Debug)]
#[logos(skip r"[ \t\r\n\f]+")]
pub enum Token {
	// an unrecognized lexeme, kept as a token so lexing never fails and the parser reports it
	Error(String),

	// literals
	#[regex(r"(true|false)", |lex| lex.slice().parse().ok())]
	Bool(bool),
	#[regex(r"[0-9][0-9_]*", |lex| lex.slice().replace('_', "").parse().ok())]
	#[regex(r"0[xX][0-9a-fA-F][0-9a-fA-F_]*", |lex| parse_radix(lex, 16))]
	#[regex(r"0[bB][01][01_]*", |lex| parse_radix(lex, 2))]
	#[regex(r"0[oO][0-7][0-7_]*", |lex| parse_radix(lex, 8))]
	Int(i64),
	#[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+\-]?[0-9]+)?", |lex| Some(lex.slice().replace('_', "")))]
	#[regex(r"[0-9][0-9_]*[eE][+\-]?[0-9]+", |lex| Some(lex.slice().replace('_', "")))]
	Float(String),
	#[regex(r#""([^"\\]|\\.)*""#, |lex| { let s = lex.slice(); s[1..s.len() - 1].to_string() })]
	String(String),
	#[regex(r#"r"[^"]*""#, |lex| { let s = lex.slice(); s[2..s.len() - 1].to_string() })]
	RawString(String),
	#[regex(r":\p{XID_Continue}+", |lex| lex.slice()[1..].to_string())]
	Atom(String),

	// keywords
	#[token("fn")]
	Fn,
	#[token("struct")]
	Struct,
	#[token("enum")]
	Enum,
	#[token("return")]
	Return,
	BareReturn,
	#[token("defer")]
	Defer,
	#[token("match")]
	Match,
	#[token("trait")]
	Trait,
	#[token("is")]
	Is,
	#[token("module")]
	Module,
	#[token("use")]
	Use,
	#[token("pub")]
	Pub,
	#[token("foreign")]
	Foreign,
	#[token("comp")]
	Comp,
	#[token("unsafe")]
	Unsafe,

	// control flow
	#[token("if")]
	If,
	#[token("else")]
	Else,
	#[token("do")]
	Do,
	#[token("loop")]
	Loop,
	#[token("break")]
	Break,
	#[token("continue")]
	Continue,
	#[token("in")]
	In,

	// identifiers
	#[token("mut")]
	Mut,
	#[token("move")]
	Move,
	#[token("none")]
	None,
	#[token("or")]
	Or,
	#[regex(r"[\p{XID_Start}_]\p{XID_Continue}*", |lex| lex.slice().to_string())]
	Ident(String),
	#[token(":=")]
	Bind,
	#[token("::")]
	DoubleColon,
	#[token("=")]
	Assign,
	#[token("=>")]
	FatArrow,

	// binary operators
	#[token("+")]
	Plus,
	#[token("-")]
	Minus,
	#[token("*")]
	Asterisk,
	#[token("**")]
	StarStar,
	#[token("/")]
	Slash,
	#[token("%")]
	Percent,

	// compound assignment operators
	#[token("+=")]
	PlusEq,
	#[token("-=")]
	MinusEq,
	#[token("*=")]
	StarEq,
	#[token("**=")]
	StarStarEq,
	#[token("/=")]
	SlashEq,
	#[token("%=")]
	PercentEq,

	// comparison operators
	#[token("==")]
	Eq,
	#[token("!=")]
	Ne,
	#[token("<<")]
	LtLt,
	#[token("<")]
	Lt,
	#[token(">")]
	Gt,
	#[token("<=")]
	Le,
	#[token(">=")]
	Ge,

	// logical operators
	#[token("&&")]
	AndAnd,
	#[token("&")]
	Amp,
	#[token("||")]
	OrOr,
	#[token("!")]
	Not,
	#[token("|")]
	Pipe,
	#[token("|>")]
	Pipeline,

	// grouping
	#[token("(")]
	LParen,
	#[token(")")]
	RParen,
	#[token("{")]
	LBrace,
	#[token("}")]
	RBrace,
	#[token("[")]
	LBracket,
	#[token("]")]
	RBracket,

	// delimiters
	#[token("via")]
	Via,
	#[token("..=")]
	DotDotEq,
	#[token("..")]
	DotDot,
	#[token(".")]
	Dot,
	SpaceDot,
	#[token(":")]
	Colon,
	#[token(",")]
	Comma,
	#[token("`")]
	Backtick,
	#[token("@")]
	At,
	#[token("$")]
	Dollar,
	#[token("?")]
	Question,
	#[token(";", logos::skip)]
	Semicolon,

	// comments
	#[token("#{", lex_block_comment)]
	BlockComment,
	#[regex(r"#([^{#\r\n][^\r\n]*)?", logos::skip)]
	Comment,
	#[regex(r"##( [^\r\n]+)?", |lex| {
		let s = lex.slice();
		s.get(3..).unwrap_or("").to_owned()
	}, allow_greedy = true)]
	Doc(String),
	DocBreak,
}

impl fmt::Display for Token {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match self {
			Token::Error(s) => write!(f, "{s}"),
			Token::BlockComment | Token::Comment | Token::DocBreak => write!(f, "comment"),
			Token::Doc(_) => write!(f, "doc"),
			Token::Bool(b) => write!(f, "{b}"),
			Token::Int(n) => write!(f, "{n}"),
			Token::Float(s) => write!(f, "{s}"),
			Token::String(s) => write!(f, "\"{s}\""),
			Token::RawString(s) => write!(f, "r\"{s}\""),
			Token::Fn => write!(f, "fn"),
			Token::Struct => write!(f, "struct"),
			Token::Enum => write!(f, "enum"),
			Token::Return | Token::BareReturn => write!(f, "return"),
			Token::Defer => write!(f, "defer"),
			Token::Match => write!(f, "match"),
			Token::Trait => write!(f, "trait"),
			Token::Is => write!(f, "is"),
			Token::Module => write!(f, "module"),
			Token::Use => write!(f, "use"),
			Token::Pub => write!(f, "pub"),
			Token::Foreign => write!(f, "foreign"),
			Token::Comp => write!(f, "comp"),
			Token::Unsafe => write!(f, "unsafe"),
			Token::If => write!(f, "if"),
			Token::Do => write!(f, "do"),
			Token::Else => write!(f, "else"),
			Token::Loop => write!(f, "loop"),
			Token::Break => write!(f, "break"),
			Token::Continue => write!(f, "continue"),
			Token::In => write!(f, "in"),
			Token::Mut => write!(f, "mut"),
			Token::Move => write!(f, "move"),
			Token::None => write!(f, "none"),
			Token::Or => write!(f, "or"),
			Token::Ident(name) => write!(f, "{name}"),
			Token::Bind => write!(f, ":="),
			Token::DoubleColon => write!(f, "::"),
			Token::Assign => write!(f, "="),
			Token::FatArrow => write!(f, "=>"),
			Token::Atom(name) => write!(f, ":{name}"),
			Token::Via => write!(f, "via"),
			Token::DotDotEq => write!(f, "..="),
			Token::DotDot => write!(f, ".."),
			Token::Dot | Token::SpaceDot => write!(f, "."),
			Token::Colon => write!(f, ":"),
			Token::Plus => write!(f, "+"),
			Token::Minus => write!(f, "-"),
			Token::Asterisk => write!(f, "*"),
			Token::StarStar => write!(f, "**"),
			Token::PlusEq => write!(f, "+="),
			Token::MinusEq => write!(f, "-="),
			Token::StarEq => write!(f, "*="),
			Token::StarStarEq => write!(f, "**="),
			Token::SlashEq => write!(f, "/="),
			Token::PercentEq => write!(f, "%="),
			Token::Slash => write!(f, "/"),
			Token::Percent => write!(f, "%"),
			Token::Eq => write!(f, "=="),
			Token::Ne => write!(f, "!="),
			Token::LtLt => write!(f, "<<"),
			Token::Lt => write!(f, "<"),
			Token::Gt => write!(f, ">"),
			Token::Le => write!(f, "<="),
			Token::Ge => write!(f, ">="),
			Token::AndAnd => write!(f, "&&"),
			Token::Amp => write!(f, "&"),
			Token::OrOr => write!(f, "||"),
			Token::Not => write!(f, "!"),
			Token::Pipe => write!(f, "|"),
			Token::Pipeline => write!(f, "|>"),
			Token::LParen => write!(f, "("),
			Token::RParen => write!(f, ")"),
			Token::LBrace => write!(f, "{{"),
			Token::RBrace => write!(f, "}}"),
			Token::LBracket => write!(f, "["),
			Token::RBracket => write!(f, "]"),
			Token::Comma => write!(f, ","),
			Token::Backtick => write!(f, "`"),
			Token::At => write!(f, "@"),
			Token::Dollar => write!(f, "$"),
			Token::Question => write!(f, "?"),
			Token::Semicolon => write!(f, ";"),
		}
	}
}

// Makes a logos pass, transforming errors into tokens so lexing always succeeds.
fn raw_lex(src: &str, base: usize) -> Vec<(Token, SimpleSpan)> {
	Token::lexer(src)
		.spanned()
		.filter_map(|(tok, sp)| {
			let span = (base + sp.start..base + sp.end).into();
			match tok {
				Ok(Token::BlockComment) => None,
				Ok(tok) => Some((tok, span)),
				Err(()) => Some((Token::Error(src[sp].to_string()), span)),
			}
		})
		.collect()
}

// Expands vars in strings.
fn expand_string(s: &str, span: SimpleSpan, src: &str) -> Vec<(Token, SimpleSpan)> {
	let bad = || vec![(Token::Error(src[span.start..span.end].to_string()), span)];
	let part = |toks: &mut Vec<(Token, SimpleSpan)>, t: Token| {
		if toks.len() > 1 {
			toks.push((Token::Plus, span));
		}
		toks.push((t, span));
	};
	let (mut toks, mut lit) = (vec![(Token::LParen, span)], String::new());
	let mut it = s.char_indices();
	while let Some((i, c)) = it.next() {
		match c {
			'{' | '}' if s[i + 1..].starts_with(c) => {
				lit.push(c);
				it.next();
			}
			'\\' => lit.push(match it.next().map(|(_, e)| e) {
				Some('n') => '\n',
				Some('t') => '\t',
				Some('r') => '\r',
				Some('0') => '\0',
				Some(e @ ('\\' | '"')) => e,
				_ => return bad(),
			}),
			'}' => return bad(),
			'{' => {
				let mut depth = 1;
				let end = it.by_ref().find_map(|(j, c)| {
					depth += (c == '{') as i32 - (c == '}') as i32;
					(depth == 0).then_some(j)
				});
				let Some(end) = end.filter(|&end| end > i + 1) else {
					return bad();
				};
				if !lit.is_empty() {
					part(&mut toks, Token::String(std::mem::take(&mut lit)));
				}
				part(&mut toks, Token::LParen);
				toks.extend(raw_lex(&s[i + 1..end], span.start + i + 2));
				let tail = [
					Token::RParen,
					Token::Dot,
					Token::Ident("str".into()),
					Token::LParen,
					Token::RParen,
				];
				toks.extend(tail.map(|t| (t, span)));
			}
			_ => lit.push(c),
		}
	}
	if toks.len() == 1 {
		return vec![(Token::String(lit), span)];
	}
	if !lit.is_empty() {
		part(&mut toks, Token::String(lit));
	}
	toks.push((Token::RParen, span));
	toks
}

// Lex `src`, shifting every span by `base` so a file lexed in isolation lands at its offset in the `Program`.
pub fn lex_at(src: &str, base: usize) -> Vec<(Token, SimpleSpan)> {
	lex(src)
		.into_iter()
		.map(|(t, s)| (t, (s.start + base..s.end + base).into()))
		.collect()
}

// Lex `src`.
// Converts errors into tokens so parsing stays recoverable.
// Inserts `DocBreak` between consecutive `Doc` tokens separated by at least one newline.
pub fn lex(src: &str) -> Vec<(Token, SimpleSpan)> {
	let raw = raw_lex(src, 0);
	let mut out = Vec::with_capacity(raw.len() + 4);
	for i in 0..raw.len() {
		let (tok, span) = &raw[i];
		if i > 0
			&& let (Token::Doc(_), Token::Doc(_)) = (&raw[i - 1].0, tok)
		{
			let gap = &src[raw[i - 1].1.end..span.start];
			if gap.bytes().filter(|&b| b == b'\n').count() > 1 {
				out.push((Token::DocBreak, (raw[i - 1].1.end..span.start).into()));
			}
		}
		match tok {
			Token::Return if raw.get(i + 1).is_some_and(|(_, next)| src[span.end..next.start].contains('\n')) => {
				out.push((Token::BareReturn, *span))
			}
			Token::Dot if i > 0 => {
				let gap = &src[raw[i - 1].1.end..span.start];
				let spaced = !gap.is_empty() && !gap.contains('\n');
				out.push((if spaced { Token::SpaceDot } else { Token::Dot }, *span));
			}
			Token::String(s) => out.extend(expand_string(s, *span, src)),
			Token::RawString(s) => out.push((Token::String(s.clone()), *span)),
			_ => out.push((tok.clone(), *span)),
		}
	}
	out
}
