use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;
use std::sync::{LazyLock, Mutex};

use chumsky::span::SimpleSpan;
use logos::{Lexer, Logos};
use rustc_hash::FxHashSet;

// An interned string, so tokens copy and compare by pointer.
#[derive(Clone, Copy, Eq)]
pub struct Sym(&'static str);

impl From<&str> for Sym {
	fn from(text: &str) -> Self {
		// NOTE: leaks each lexeme for the process, though that's prolly fine for a compiler run
		static TABLE: LazyLock<Mutex<FxHashSet<&'static str>>> = LazyLock::new(Mutex::default);
		// skips the lock for lexemes this thread has seen
		thread_local!(static SEEN: RefCell<FxHashSet<&'static str>> = RefCell::default());
		Sym(SEEN.with_borrow_mut(|seen| match seen.get(text) {
			Some(&sym) => sym,
			None => {
				let mut table = TABLE.lock().unwrap();
				let sym = table.get(text).copied().unwrap_or_else(|| Box::leak(text.into()));
				table.insert(sym);
				seen.insert(sym);
				sym
			}
		}))
	}
}

impl std::ops::Deref for Sym {
	type Target = str;
	fn deref(&self) -> &str {
		self.0
	}
}

impl PartialEq for Sym {
	fn eq(&self, other: &Sym) -> bool {
		std::ptr::eq(self.0, other.0)
	}
}

impl PartialEq<str> for Sym {
	fn eq(&self, other: &str) -> bool {
		self.0 == other
	}
}

impl fmt::Display for Sym {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		self.0.fmt(f)
	}
}

impl fmt::Debug for Sym {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		self.0.fmt(f)
	}
}

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

// Dedent a `"""` body that starts on its own line, to its shallowest line.
fn dedent(s: &str) -> String {
	let Some((_, body)) = s.split_once('\n').filter(|(head, _)| head.trim().is_empty()) else {
		return s.into();
	};
	let indent = |l: &str| l.len() - l.trim_start().len();
	let closer = indent(body.rsplit('\n').next().unwrap_or_default());
	let cols = body.lines().filter(|l| !l.trim().is_empty()).map(indent);
	let n = cols.chain([closer]).min().unwrap_or(0);
	let body = body.trim_end_matches([' ', '\t']);
	body.lines().map(|l| l.get(n..).unwrap_or("")).collect::<Vec<_>>().join("\n")
}

// Body of an N-quote literal, closed by the first run of N.
fn lex_long(lex: &mut Lexer<Token>, escaped: bool) -> Option<Sym> {
	let n = lex.slice().bytes().filter(|&b| b == b'"').count();
	let rest = lex.remainder();
	let (b, mut i) = (rest.as_bytes(), 0);
	while let Some(w) = b.get(i..i + n) {
		if w.iter().all(|&c| c == b'"') {
			lex.bump(i + n);
			return Some(dedent(&rest[..i]).as_str().into());
		}
		i += 1 + usize::from(escaped && w[0] == b'\\');
	}
	lex.bump(rest.len());
	None
}

// Bit pattern, so the whole 128-bit range lexes.
fn parse_radix(digits: &str, radix: u32) -> Option<i128> {
	u128::from_str_radix(&digits.replace('_', ""), radix).ok().map(|n| n as i128)
}

// One escape, where `i` is the backslash's byte offset into a string and `it` sits just past it.
fn escape(s: &str, i: usize, it: &mut std::str::CharIndices) -> Option<char> {
	Some(match it.next()?.1 {
		'n' => '\n',
		't' => '\t',
		'r' => '\r',
		'0' => '\0',
		'e' => '\x1b',
		'a' => '\x07',
		'b' => '\x08',
		'f' => '\x0c',
		'v' => '\x0b',
		e @ ('\\' | '"' | '\'') => e,
		k @ ('x' | 'u') => {
			let rest = &s[i + 2..];
			let hex = match k {
				'x' => rest.get(..2),
				_ => rest.strip_prefix('{').and_then(|r| r.split_once('}')).map(|(h, _)| h),
			};
			let hex = hex.filter(|h| (1..=6).contains(&h.len()))?;
			let code = hex.chars().try_fold(0u32, |n, c| Some(n * 16 + c.to_digit(16)?));
			let c = code.filter(|&n| k == 'u' || n < 0x80).and_then(char::from_u32)?;
			it.nth(hex.len() + 2 * (k == 'u') as usize - 1);
			c
		}
		_ => return None,
	})
}

// Lex a rune literal.
fn lex_rune(lex: &mut Lexer<Token>) -> Option<i64> {
	let body = &lex.slice()[1..lex.slice().len() - 1];
	let mut it = body.char_indices();
	let c = match it.next()? {
		(i, '\\') => escape(body, i, &mut it)?,
		(_, c) => c,
	};
	it.next().is_none().then_some(c as i64)
}

#[derive(Logos, Clone, Copy, PartialEq, Debug)]
#[logos(skip r"[ \t\r\n\f]+")]
pub enum Token {
	// an unrecognized lexeme, kept as a token so lexing never fails and the parser reports it
	Error(Sym),

	// literals
	#[regex(r"(true|false)", |lex| lex.slice().parse().ok())]
	Bool(bool),
	#[regex(r"[0-9][0-9_]*", |lex| parse_radix(lex.slice(), 10))]
	#[regex(r"0[xX][0-9a-fA-F][0-9a-fA-F_]*", |lex| parse_radix(&lex.slice()[2..], 16))]
	#[regex(r"0[bB][01][01_]*", |lex| parse_radix(&lex.slice()[2..], 2))]
	#[regex(r"0[oO][0-7][0-7_]*", |lex| parse_radix(&lex.slice()[2..], 8))]
	Int(i128),
	#[regex(r"'([^'\\]|\\.)*'", lex_rune)]
	Rune(i64),
	#[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+\-]?[0-9]+)?", |lex| Some(Sym::from(lex.slice().replace('_', "").as_str())))]
	#[regex(r"[0-9][0-9_]*[eE][+\-]?[0-9]+", |lex| Some(Sym::from(lex.slice().replace('_', "").as_str())))]
	Float(Sym),
	#[regex(r#""{3,}"#, |lex| lex_long(lex, true))]
	#[regex(r#""([^"\\]|\\.)*""#, |lex| Sym::from(&lex.slice()[1..lex.slice().len() - 1]))]
	String(Sym),
	#[regex(r#"r"{3,}"#, |lex| lex_long(lex, false))]
	#[regex(r#"r"[^"]*""#, |lex| Sym::from(&lex.slice()[2..lex.slice().len() - 1]))]
	RawString(Sym),
	#[regex(r#"p"[^"]*""#, |lex| Sym::from(&lex.slice()[2..lex.slice().len() - 1]))]
	PathString(Sym),
	#[regex(r":\p{XID_Continue}+", |lex| Sym::from(&lex.slice()[1..]))]
	Atom(Sym),

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
	#[token("fallthrough")]
	Fallthrough,
	#[token("defer")]
	Defer,
	#[token("cancel")]
	Cancel,
	#[token("discard")]
	Discard,
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
	#[token("with")]
	With,

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
	#[token("or")]
	Or,
	#[token("and")]
	And,
	#[regex(r"[\p{XID_Start}_]\p{XID_Continue}*", |lex| Sym::from(lex.slice()))]
	Ident(Sym),
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
	SpaceMinus,
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
	#[token("&=")]
	AmpEq,
	#[token("|=")]
	PipeEq,
	#[token("~=")]
	TildeEq,
	#[token("<<=")]
	LtLtEq,
	#[token(">>=")]
	GtGtEq,

	// comparison operators
	#[token("==")]
	Eq,
	#[token("!=")]
	Ne,
	#[token("<")]
	Lt,
	#[token(">")]
	Gt,
	#[token("<=")]
	Le,
	#[token(">=")]
	Ge,

	// bitwise operators
	#[token("<<")]
	LtLt,
	#[token(">>")]
	GtGt,
	#[token("~")]
	Tilde,

	// logical operators
	#[token("&&")]
	AndAnd,
	#[token("&")]
	Amp,
	#[token("||")]
	OrOr,
	#[token("!")]
	Not,
	NotIn,
	NotIs,
	#[token("|")]
	Pipe,
	#[token("|>")]
	Pipeline,

	// channels
	#[token("<-")]
	ArrowL,

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
	#[token("^")]
	Caret,
	#[token(";")]
	Semicolon,

	// comments
	#[token("#{", lex_block_comment)]
	BlockComment,
	#[regex(r"#([^{#\r\n][^\r\n]*)?")]
	Comment,
	#[regex(r"##( [^\r\n]+)?", |lex| {
		let s = lex.slice();
		Sym::from(s.get(3..).unwrap_or(""))
	}, allow_greedy = true)]
	Doc(Sym),
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
			Token::Rune(n) => write!(f, "{n}"),
			Token::Float(s) => write!(f, "{s}"),
			Token::String(s) => write!(f, "\"{s}\""),
			Token::RawString(s) => write!(f, "r\"{s}\""),
			Token::PathString(s) => write!(f, "p\"{s}\""),
			Token::Fn => write!(f, "fn"),
			Token::Struct => write!(f, "struct"),
			Token::Enum => write!(f, "enum"),
			Token::Return | Token::BareReturn => write!(f, "return"),
			Token::Fallthrough => write!(f, "fallthrough"),
			Token::Defer => write!(f, "defer"),
			Token::Cancel => write!(f, "cancel"),
			Token::Discard => write!(f, "discard"),
			Token::Match => write!(f, "match"),
			Token::Trait => write!(f, "trait"),
			Token::Is => write!(f, "is"),
			Token::Module => write!(f, "module"),
			Token::Use => write!(f, "use"),
			Token::Pub => write!(f, "pub"),
			Token::Foreign => write!(f, "foreign"),
			Token::Comp => write!(f, "comp"),
			Token::Unsafe => write!(f, "unsafe"),
			Token::With => write!(f, "with"),
			Token::If => write!(f, "if"),
			Token::Do => write!(f, "do"),
			Token::Else => write!(f, "else"),
			Token::Loop => write!(f, "loop"),
			Token::Break => write!(f, "break"),
			Token::Continue => write!(f, "continue"),
			Token::In => write!(f, "in"),
			Token::Mut => write!(f, "mut"),
			Token::Move => write!(f, "move"),
			Token::Or => write!(f, "or"),
			Token::And => write!(f, "and"),
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
			Token::Minus | Token::SpaceMinus => write!(f, "-"),
			Token::Asterisk => write!(f, "*"),
			Token::StarStar => write!(f, "**"),
			Token::PlusEq => write!(f, "+="),
			Token::MinusEq => write!(f, "-="),
			Token::StarEq => write!(f, "*="),
			Token::StarStarEq => write!(f, "**="),
			Token::SlashEq => write!(f, "/="),
			Token::PercentEq => write!(f, "%="),
			Token::AmpEq => write!(f, "&="),
			Token::PipeEq => write!(f, "|="),
			Token::TildeEq => write!(f, "~="),
			Token::LtLtEq => write!(f, "<<="),
			Token::GtGtEq => write!(f, ">>="),
			Token::Slash => write!(f, "/"),
			Token::Percent => write!(f, "%"),
			Token::Eq => write!(f, "=="),
			Token::Ne => write!(f, "!="),
			Token::Lt => write!(f, "<"),
			Token::Gt => write!(f, ">"),
			Token::Le => write!(f, "<="),
			Token::Ge => write!(f, ">="),
			Token::LtLt => write!(f, "<<"),
			Token::GtGt => write!(f, ">>"),
			Token::Tilde => write!(f, "~"),
			Token::AndAnd => write!(f, "&&"),
			Token::Amp => write!(f, "&"),
			Token::OrOr => write!(f, "||"),
			Token::Not => write!(f, "!"),
			Token::NotIn => write!(f, "!in"),
			Token::NotIs => write!(f, "!is"),
			Token::Pipe => write!(f, "|"),
			Token::Pipeline => write!(f, "|>"),
			Token::ArrowL => write!(f, "<-"),
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
			Token::Caret => write!(f, "^"),
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
				Ok(Token::BlockComment | Token::Comment) => None,
				Ok(tok) => Some((tok, span)),
				Err(()) => Some((Token::Error(src[sp].into()), span)),
			}
		})
		.collect()
}

// Expands vars in strings.
fn expand_string(s: &str, span: SimpleSpan, text: &str) -> Vec<(Token, SimpleSpan)> {
	let bad = || vec![(Token::Error(text.into()), span)];
	let part = |toks: &mut Vec<(Token, SimpleSpan)>, t: Token| {
		if toks.len() > 1 {
			toks.push((Token::Plus, span));
		}
		toks.push((t, span));
	};
	// width of the open delimiter
	let open = text.bytes().take_while(|&b| b == b'"').count();
	let (mut toks, mut lit) = (vec![(Token::LParen, span)], String::new());
	let mut it = s.char_indices();
	while let Some((i, c)) = it.next() {
		match c {
			'{' | '}' if s[i + 1..].starts_with(c) => {
				lit.push(c);
				it.next();
			}
			'\\' => match escape(s, i, &mut it) {
				Some(e) => lit.push(e),
				None => return bad(),
			},
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
					part(&mut toks, Token::String(std::mem::take(&mut lit).as_str().into()));
				}
				part(&mut toks, Token::LParen);
				toks.extend(raw_lex(&s[i + 1..end], span.start + open + i + 1));
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
		return vec![(Token::String(lit.as_str().into()), span)];
	}
	if !lit.is_empty() {
		part(&mut toks, Token::String(lit.as_str().into()));
	}
	toks.push((Token::RParen, span));
	toks
}

// Look for certain macros and imports before parsing.
pub fn prescan(toks: &[(Token, SimpleSpan)]) -> (Vec<String>, Vec<String>) {
	let (mut raw, mut imports) = (vec![], vec![]);
	let at = |i: usize| toks.get(i).map(|(t, _)| t);
	for (i, (tok, _)) in toks.iter().enumerate() {
		match (tok, at(i + 1)) {
			(Token::Use, Some(Token::Ident(m))) => imports.push(m.to_string()),
			(Token::Ident(n), Some(Token::Not))
				if at(i + 2) == Some(&Token::DoubleColon)
					&& at(i + 3) == Some(&Token::Fn)
					&& toks[i + 4..]
						.iter()
						.take_while(|(t, _)| *t != Token::RParen)
						.any(|(t, _)| matches!(t, Token::Ident(p) if p == "Tokens")) =>
			{
				raw.push(n.to_string())
			}
			_ => {}
		}
	}
	(raw, imports)
}

// Rewrite macros that produce tokens into the normal call form.
pub fn splice_raw(
	toks: Vec<(Token, SimpleSpan)>,
	raw: &HashSet<String>,
	src: &str,
	base: usize,
) -> Result<Vec<(Token, SimpleSpan)>, SimpleSpan> {
	let mut out = Vec::with_capacity(toks.len());
	let mut i = 0;
	while i < toks.len() {
		let open = match (&toks[i].0, toks.get(i + 1), toks.get(i + 2)) {
			(Token::Ident(n), Some((Token::Not, bang)), Some((Token::LBrace, open))) if raw.contains(&**n) => {
				out.extend_from_slice(&toks[i..i + 2]);
				out.push((Token::LParen, (bang.end..bang.end).into()));
				out.push((Token::LBracket, *open));
				*open
			}
			_ => {
				out.push(toks[i]);
				i += 1;
				continue;
			}
		};
		let (mut depth, mut covered) = (1i32, open.end);
		i += 3;
		loop {
			let (tok, span) = toks.get(i).ok_or(open)?;
			i += 1;
			if span.start < covered {
				continue;
			}
			covered = span.end;
			// `%name` / `%{expr}` escapes back to Oi
			if *tok == Token::Percent
				&& let Some((next, at)) = toks.get(i)
				&& at.start == span.end
			{
				let group = *next == Token::LBrace;
				let (from, mut d) = (i + usize::from(group), i32::from(group));
				i += 1;
				while d > 0 {
					let (t, _) = toks.get(i).ok_or(open)?;
					i += 1;
					d += i32::from(*t == Token::LBrace) - i32::from(*t == Token::RBrace);
				}
				out.extend_from_slice(&toks[from..i - usize::from(group)]);
				out.push((Token::Comma, *span));
				covered = toks[i - 1].1.end;
				continue;
			}
			depth += i32::from(*tok == Token::LBrace) - i32::from(*tok == Token::RBrace);
			if depth == 0 {
				out.push((Token::RBracket, *span));
				out.push((Token::RParen, *span));
				break;
			}
			out.push((Token::String(src[span.start - base..span.end - base].into()), *span));
			out.push((Token::Comma, *span));
		}
	}
	Ok(out)
}

// Lex `src`, shifting every span by `base` so a file lexed in isolation lands at its offset in the `Program`.
// Converts errors into tokens so parsing stays recoverable.
// Inserts `DocBreak` between consecutive `Doc` tokens separated by at least one newline.
pub fn lex_at(src: &str, base: usize) -> Vec<(Token, SimpleSpan)> {
	let raw = raw_lex(src, base);
	let text = |from: usize, to: usize| &src[from - base..to - base];
	let mut out = Vec::with_capacity(raw.len() + 4);
	let mut fused = false;
	for i in 0..raw.len() {
		let (tok, span) = &raw[i];
		if std::mem::take(&mut fused) {
			continue;
		}
		if i > 0
			&& let (Token::Doc(_), Token::Doc(_)) = (&raw[i - 1].0, tok)
		{
			let gap = text(raw[i - 1].1.end, span.start);
			if gap.bytes().filter(|&b| b == b'\n').count() > 1 {
				out.push((Token::DocBreak, (raw[i - 1].1.end..span.start).into()));
			}
		}
		match tok {
			Token::Return
				if raw
					.get(i + 1)
					.is_some_and(|(_, n)| text(span.end, n.start).contains(['\n', ';'])) =>
			{
				out.push((Token::BareReturn, *span))
			}
			Token::Dot if i > 0 => {
				let gap = text(raw[i - 1].1.end, span.start);
				let spaced = !gap.is_empty() && !gap.contains('\n');
				out.push((if spaced { Token::SpaceDot } else { Token::Dot }, *span));
			}
			// asymmetric spacing is a negation
			Token::Minus if i > 0 => {
				let before = text(raw[i - 1].1.end, span.start);
				let after = raw.get(i + 1).map(|(_, next)| text(span.end, next.start));
				let lead = !before.is_empty() && after == Some("");
				out.push((if lead { Token::SpaceMinus } else { Token::Minus }, *span));
			}
			Token::Not if i > 0 && !text(raw[i - 1].1.end, span.start).is_empty() => match raw.get(i + 1) {
				Some((fusable @ (Token::In | Token::Is), next)) if next.start == span.end => {
					let fused_tok = if *fusable == Token::In {
						Token::NotIn
					} else {
						Token::NotIs
					};
					out.push((fused_tok, (span.start..next.end).into()));
					fused = true;
				}
				_ => out.push((*tok, *span)),
			},
			Token::String(s) => out.extend(expand_string(s, *span, text(span.start, span.end))),
			Token::RawString(s) => out.push((Token::String(*s), *span)),
			_ => out.push((*tok, *span)),
		}
	}
	out
}
