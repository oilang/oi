use std::fmt;

use chumsky::span::SimpleSpan;

pub type Span = SimpleSpan;
pub type Spanned<T> = (T, Span);
pub type Bounds<'a> = (Option<&'a Spanned<Expr>>, Option<&'a Spanned<Expr>>, bool);

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Expr {
	// literals
	Bool(bool),
	Int(i64),
	Float(f64),
	String(String),
	Atom(String),
	Ident(String),
	Dollar,
	None,
	Foreign,

	// `[mods] name [type] := value`
	Bind {
		mutable: bool,
		name: String,
		typ: Option<Spanned<TypeExpr>>,
		value: Option<Box<Spanned<Expr>>>,
	},

	// `name = value`
	Assign {
		name: String,
		value: Box<Spanned<Expr>>,
	},

	// pattern bindings
	PatBind {
		pat: Box<Spanned<Expr>>,
		value: Box<Spanned<Expr>>,
		// `None` means assigning to existing locals
		mutable: Option<bool>,
	},

	// `..expr`
	Spread(Box<Spanned<Expr>>),

	// functions
	Fn {
		name: String,
		type_params: Vec<TypeParam>,
		params: Vec<Param>,
		params_tuple: bool,
		ret: Option<Spanned<TypeExpr>>,
		body: Vec<Spanned<Expr>>,
	},

	// `fn [captures]? (params)? ret? { body }`
	AnonFn {
		captures: Option<Vec<Capture>>,
		params: Vec<Param>,
		params_tuple: bool,
		ret: Option<Spanned<TypeExpr>>,
		body: Vec<Spanned<Expr>>,
	},

	Call {
		name: String,
		type_args: Vec<Spanned<TypeExpr>>,
		args: Vec<Spanned<Expr>>,
	},

	MethodCall {
		recv: Box<Spanned<Expr>>,
		method: String,
		type_args: Vec<Spanned<TypeExpr>>,
		args: Vec<Spanned<Expr>>,
	},

	Apply {
		callee: Box<Spanned<Expr>>,
		args: Vec<Spanned<Expr>>,
	},

	Return(Option<Box<Spanned<Expr>>>),

	// `Target.(args)`
	Cast {
		target: Spanned<TypeExpr>,
		args: Vec<Spanned<Expr>>,
	},

	// macros

	// `name! :: fn(params) Ast { body }`
	MacroDef {
		name: String,
		params: Vec<Param>,
		ret: Option<Spanned<TypeExpr>>,
		body: Vec<Spanned<Expr>>,
	},

	// `name!(args)`, `name! expr`
	MacroCall {
		name: String,
		args: Vec<Spanned<Expr>>,
	},
	// quasi-quote
	Quote(Vec<Spanned<Expr>>),
	// `%name`
	Unquote(String),
	// `%{expr}`
	UnquoteExpr(Box<Spanned<Expr>>),
	// `%{..expr}`
	UnquoteSplat(Box<Spanned<Expr>>),
	// `%{expr}`
	UnquoteBind(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
	// a macro expansion scoped block
	Block(Vec<Spanned<Expr>>),

	Comp(Box<Spanned<Expr>>),
	Unsafe(Box<Spanned<Expr>>),

	ArgMod(Access, Box<Spanned<Expr>>),

	// control flow
	If {
		cond: Box<Spanned<Expr>>,
		then: Vec<Spanned<Expr>>,
		els: Option<Vec<Spanned<Expr>>>,
	},
	Loop {
		cond: Option<Box<Spanned<Expr>>>,
		body: Vec<Spanned<Expr>>,
	},
	For {
		pat: Box<Spanned<Expr>>,
		iter: Box<Spanned<Expr>>,
		body: Vec<Spanned<Expr>>,
	},
	Break,
	Continue,

	// structures

	// tuples
	Tuple(Vec<(Option<String>, Spanned<Expr>)>),
	Field {
		tuple: Box<Spanned<Expr>>,
		field: String,
	},

	// arrays
	Array(Vec<Spanned<Expr>>),
	// `.[ ..expr ]`, `T.[ ..expr ]`
	DotArray(Option<Spanned<TypeExpr>>, Vec<Spanned<Expr>>),
	// `.( ..expr )`
	DotTuple(Vec<Spanned<Expr>>),
	// `collection[index]`
	// TODO: handle negative indices
	Index {
		collection: Box<Spanned<Expr>>,
		index: Box<Spanned<Expr>>,
	},
	// `collection[range]`
	Slice {
		collection: Box<Spanned<Expr>>,
		range: Option<Box<Spanned<Expr>>>,
	},
	// `name[index] = value`
	IndexAssign {
		name: String,
		index: Box<Spanned<Expr>>,
		value: Box<Spanned<Expr>>,
	},
	// `name << value`
	Append {
		name: String,
		value: Box<Spanned<Expr>>,
	},
	// `name.delete[key]`
	MapDelete {
		name: String,
		key: Box<Spanned<Expr>>,
	},

	// `match subject { pattern, ... { body } ... else { body } }`
	Match {
		subject: Box<Spanned<Expr>>,
		arms: Vec<MatchArm>,
		else_body: Option<Vec<Spanned<Expr>>>,
	},

	// `pattern => body`
	Arm(MatchArm),

	// `value |> step`
	Pipe {
		value: Box<Spanned<Expr>>,
		step: Box<Spanned<Expr>>,
	},

	// `value or { body }`
	OrElse {
		value: Box<Spanned<Expr>>,
		body: Vec<Spanned<Expr>>,
	},

	// `value?`, unwraps `?T`/`!T`
	Propagate(Box<Spanned<Expr>>),

	// structs
	// `Name :: struct {}`
	StructDef {
		name: String,
		type_params: Vec<TypeParam>,
		fields: Vec<Param>,
		fills: Vec<Spanned<Expr>>,
	},
	// `Name {}`
	StructLit {
		name: String,
		type_args: Vec<Spanned<TypeExpr>>,
		fields: Vec<(Option<String>, Spanned<Expr>)>,
	},
	// `&Name {}`
	Ref(Box<Spanned<Expr>>),
	// `{ k = v }`
	Record(Vec<(Spanned<Expr>, Spanned<Expr>)>),
	// `[ k = v, ]`
	Map(Vec<(Spanned<Expr>, Spanned<Expr>)>),
	// `name.field = value`
	FieldAssign {
		name: String,
		field: String,
		value: Box<Spanned<Expr>>,
	},

	// `Type :< { fills }`
	Claim {
		typ: String,
		type_params: Vec<TypeParam>,
		traits: Vec<TraitRef>,
		via: Option<String>,
		fills: Vec<Spanned<Expr>>,
	},

	// `trait Name {}`
	TraitDef {
		name: String,
		type_params: Vec<TypeParam>,
		supers: Vec<String>,
		fields: Vec<Param>,
		methods: Vec<Spanned<Expr>>,
	},

	// `type Name = TypeExpr`
	TypeAlias {
		name: String,
		typ: TypeExpr,
	},

	TypePat(TypeExpr),

	Range {
		start: Box<Spanned<Expr>>,
		end: Option<Box<Spanned<Expr>>>,
		inclusive: bool,
	},

	// `Name : backing? : enum {}`
	EnumDef {
		name: String,
		backing: Option<Spanned<TypeExpr>>,
		type_params: Vec<TypeParam>,
		variants: Vec<EnumVariant>,
		fills: Vec<Spanned<Expr>>,
	},
	// `.variant`, `.variant(args)`
	EnumShorthand {
		variant: String,
		args: Vec<Spanned<Expr>>,
	},

	// modules

	// `module name`
	Module(String),
	// `use path`, `name :: use path.{ local :: remote }`
	Use {
		name: Option<Spanned<String>>,
		path: Vec<Spanned<String>>,
		group: Option<Vec<UseItem>>,
	},
	// `pub expr`
	Pub(Box<Spanned<Expr>>),

	// `@annotation`
	Annotated(Vec<Annotation>, Box<Spanned<Expr>>),

	// operators

	// `T is not? Trait`
	Is {
		subject: Box<Spanned<Expr>>,
		trait_name: String,
		negated: bool,
	},

	// unary
	Negative(Box<Spanned<Expr>>),

	// arithmetic, comparison, logical, membership
	Binary(BinOp, Box<Spanned<Expr>>, Box<Spanned<Expr>>),
	Not(Box<Spanned<Expr>>),

	// meta
	Doc(Vec<String>),
}

// A child of `Expr`.
pub enum Child<'a> {
	List(&'a mut Vec<Spanned<Expr>>),
	One(&'a mut Spanned<Expr>),
}

use Child::{List, One};

impl Expr {
	// Get Bounds from range literals.
	pub fn bounds(&self) -> Option<Bounds<'_>> {
		match self {
			Expr::Range { start, end, inclusive } => Some((Some(start), end.as_deref(), *inclusive)),
			Expr::Spread(end) => Some((None, Some(end), false)),
			_ => None,
		}
	}

	// The name a top-level definition binds.
	pub fn def_name(&self) -> Option<&str> {
		match self {
			Expr::Fn { name, .. }
			| Expr::StructDef { name, .. }
			| Expr::EnumDef { name, .. }
			| Expr::TypeAlias { name, .. }
			| Expr::TraitDef { name, .. } => Some(name),
			_ => None,
		}
	}

	// Visit every direct child, in whichever shape it's stored.
	pub fn for_children(&mut self, mut f: impl FnMut(Child)) {
		match self {
			Expr::Bind { typ, value, .. } => {
				typ.iter_mut().for_each(|(t, _)| t.holes(|e| f(One(e))));
				value.iter_mut().for_each(|v| f(One(v)));
			}
			Expr::Return(value) => value.iter_mut().for_each(|v| f(One(v))),
			Expr::Assign { value: v, .. }
			| Expr::PatBind { value: v, .. }
			| Expr::ArgMod(_, v)
			| Expr::Spread(v)
			| Expr::Ref(v)
			| Expr::Pub(v)
			| Expr::Propagate(v)
			| Expr::Negative(v)
			| Expr::Not(v)
			| Expr::Field { tuple: v, .. }
			| Expr::Append { value: v, .. }
			| Expr::FieldAssign { value: v, .. }
			| Expr::MapDelete { key: v, .. }
			| Expr::Is { subject: v, .. }
			| Expr::UnquoteExpr(v)
			| Expr::UnquoteSplat(v)
			| Expr::Comp(v)
			| Expr::Unsafe(v) => f(One(v)),
			Expr::Annotated(anns, v) => {
				f(List(anns));
				f(One(v));
			}
			Expr::Cast { target, args } => {
				target.0.holes(|e| f(One(e)));
				f(List(args));
			}
			Expr::Index {
				collection: a,
				index: b,
			}
			| Expr::IndexAssign { index: a, value: b, .. }
			| Expr::Pipe { value: a, step: b }
			| Expr::UnquoteBind(a, b)
			| Expr::Binary(_, a, b) => {
				f(One(a));
				f(One(b));
			}
			Expr::Fn { params, ret, body, .. }
			| Expr::AnonFn { params, ret, body, .. }
			| Expr::MacroDef { params, ret, body, .. } => {
				let sig = params.iter_mut().map(|p| &mut p.typ).chain(ret.iter_mut().map(|(t, _)| t));
				sig.for_each(|t| t.holes(|e| f(One(e))));
				f(List(body));
			}
			Expr::StructDef { fields, fills, .. } => {
				fields.iter_mut().for_each(|p| p.typ.holes(|e| f(One(e))));
				f(List(fills));
			}
			Expr::Block(body)
			| Expr::Quote(body)
			| Expr::Claim { fills: body, .. }
			| Expr::EnumDef { fills: body, .. }
			| Expr::TraitDef { methods: body, .. } => f(List(body)),
			Expr::Call { args, .. }
			| Expr::MacroCall { args, .. }
			| Expr::EnumShorthand { args, .. }
			| Expr::Array(args)
			| Expr::DotArray(_, args)
			| Expr::DotTuple(args) => args.iter_mut().for_each(|a| f(One(a))),
			Expr::MethodCall { recv, args, .. } | Expr::Apply { callee: recv, args } => {
				f(One(recv));
				args.iter_mut().for_each(|a| f(One(a)));
			}
			Expr::If { cond, then, els } => {
				f(One(cond));
				f(List(then));
				els.iter_mut().for_each(|e| f(List(e)));
			}
			Expr::Loop { cond, body } => {
				cond.iter_mut().for_each(|c| f(One(c)));
				f(List(body));
			}
			Expr::For { iter: v, body, .. } | Expr::OrElse { value: v, body } => {
				f(One(v));
				f(List(body));
			}
			Expr::Tuple(fields) | Expr::StructLit { fields, .. } => fields.iter_mut().for_each(|(_, v)| f(One(v))),
			Expr::Record(entries) | Expr::Map(entries) => entries.iter_mut().for_each(|(k, v)| {
				f(One(k));
				f(One(v));
			}),
			Expr::Slice { collection, range } => {
				[Some(collection), range.as_mut()].into_iter().flatten().for_each(|x| f(One(x)))
			}
			Expr::Range { start, end, .. } => [Some(start), end.as_mut()].into_iter().flatten().for_each(|x| f(One(x))),
			Expr::Match {
				subject,
				arms,
				else_body,
			} => {
				f(One(subject));
				for arm in arms {
					f(List(&mut arm.patterns));
					f(List(&mut arm.body));
				}
				else_body.iter_mut().for_each(|e| f(List(e)));
			}
			Expr::Arm(arm) => {
				f(List(&mut arm.patterns));
				f(List(&mut arm.body));
			}
			Expr::Bool(_)
			| Expr::Int(_)
			| Expr::Float(_)
			| Expr::String(_)
			| Expr::Atom(_)
			| Expr::Ident(_)
			| Expr::Dollar
			| Expr::None
			| Expr::Foreign
			| Expr::Break
			| Expr::Continue
			| Expr::Unquote(_)
			| Expr::TypeAlias { .. }
			| Expr::TypePat(_)
			| Expr::Module(_)
			| Expr::Use { .. }
			| Expr::Doc(_) => {}
		}
	}

	// Apply `f` to this expression and every one beneath it.
	pub fn walk(&mut self, f: &mut impl FnMut(&mut Expr)) {
		f(self);
		self.for_children(|c| match c {
			List(list) => list.iter_mut().for_each(|(e, _)| e.walk(f)),
			One((e, _)) => e.walk(f),
		});
	}

	pub fn try_children<E>(&mut self, mut f: impl FnMut(Child) -> Result<(), E>) -> Result<(), E> {
		let mut err = None;
		self.for_children(|c| {
			if err.is_none() {
				err = f(c).err();
			}
		});
		err.map_or(Ok(()), Err)
	}

	// Every referenced identifier.
	pub fn idents(&self, out: &mut std::collections::HashSet<String>) {
		self.clone().walk(&mut |e| match e {
			Expr::Ident(n) => {
				out.insert(n.clone());
			}
			Expr::AnonFn {
				captures: Some(list), ..
			} => {
				for c in list {
					let (Capture::ReadOnly(n) | Capture::Mut(n) | Capture::Move(n)) = c;
					out.insert(n.clone());
				}
			}
			_ => {}
		});
	}

	// Type-directed literals take their type from where they sit.
	pub fn anon(&self) -> bool {
		match self {
			Expr::Int(_)
			| Expr::Float(_)
			| Expr::Atom(_)
			| Expr::None
			| Expr::EnumShorthand { .. }
			| Expr::DotArray(None, _)
			| Expr::DotTuple(_)
			| Expr::Record(_)
			| Expr::AnonFn { ret: None, .. } => true,
			Expr::StructLit { name, .. } => name.is_empty(),
			Expr::Negative(e) => e.0.anon(),
			_ => false,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
	Add,
	Sub,
	Mul,
	Div,
	Mod,
	Pow,
	Eq,
	Ne,
	Lt,
	Gt,
	Le,
	Ge,
	And,
	Or,
	In,
}

impl fmt::Display for BinOp {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			BinOp::Add => "+",
			BinOp::Sub => "-",
			BinOp::Mul => "*",
			BinOp::Pow => "**",
			BinOp::Div => "/",
			BinOp::Mod => "%",
			BinOp::Eq => "==",
			BinOp::Ne => "!=",
			BinOp::Lt => "<",
			BinOp::Gt => ">",
			BinOp::Le => "<=",
			BinOp::Ge => ">=",
			BinOp::And => "&&",
			BinOp::Or => "||",
			BinOp::In => "in",
		})
	}
}

// Type annotation.
#[derive(Debug, Clone)]
pub enum TypeExpr {
	Name(String),
	Tuple(Vec<(Option<String>, TypeExpr)>),
	Array(Box<TypeExpr>),
	FixedArray(Box<TypeExpr>, Box<Spanned<Expr>>),
	Fn(Vec<(Option<String>, Access, TypeExpr)>, Box<TypeExpr>),
	Annotated(Vec<Annotation>, Box<TypeExpr>),
	Option(Box<TypeExpr>),
	Result(Box<TypeExpr>, Option<Box<TypeExpr>>),
	AtomSum(Vec<String>),
	Sum(Vec<TypeExpr>),
	TupleStruct(String, Vec<(Option<String>, TypeExpr)>),
	Map(Box<TypeExpr>, Box<TypeExpr>),
	Generic(String, Vec<TypeExpr>),
	Ref(Box<TypeExpr>),
	AnonStruct(Vec<Param>),
	Variadic(Box<TypeExpr>),
	Unquote(Box<Spanned<Expr>>),
}

impl TypeExpr {
	// Visit this type and every type nested in it.
	pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut TypeExpr)) {
		f(self);
		match self {
			TypeExpr::Array(t)
			| TypeExpr::FixedArray(t, _)
			| TypeExpr::Annotated(_, t)
			| TypeExpr::Option(t)
			| TypeExpr::Ref(t)
			| TypeExpr::Variadic(t) => t.walk_mut(f),
			TypeExpr::Result(t, e) => {
				t.walk_mut(f);
				e.iter_mut().for_each(|e| e.walk_mut(f));
			}
			TypeExpr::Map(k, v) => {
				k.walk_mut(f);
				v.walk_mut(f);
			}
			TypeExpr::Fn(params, ret) => {
				params.iter_mut().for_each(|(.., t)| t.walk_mut(f));
				ret.walk_mut(f);
			}
			TypeExpr::Tuple(fields) | TypeExpr::TupleStruct(_, fields) => {
				fields.iter_mut().for_each(|(_, t)| t.walk_mut(f));
			}
			TypeExpr::Sum(types) | TypeExpr::Generic(_, types) => types.iter_mut().for_each(|t| t.walk_mut(f)),
			TypeExpr::AnonStruct(fields) => fields.iter_mut().for_each(|p| p.typ.walk_mut(f)),
			TypeExpr::Name(_) | TypeExpr::AtomSum(_) | TypeExpr::Unquote(_) => {}
		}
	}

	// Every unquote in this type.
	pub fn holes(&mut self, mut f: impl FnMut(&mut Spanned<Expr>)) {
		self.walk_mut(&mut |t| {
			if let TypeExpr::Unquote(e) = t {
				f(e);
			}
		});
	}

	// The type an expression could be naming.
	pub fn from_expr(e: &Expr) -> Option<TypeExpr> {
		match e {
			Expr::Ident(n) => Some(TypeExpr::Name(n.clone())),
			Expr::TypePat(t) => Some(t.clone()),
			Expr::Tuple(fields) if !fields.is_empty() && fields.iter().all(|(n, _)| n.is_none()) => fields
				.iter()
				.map(|(_, v)| Some((None, TypeExpr::from_expr(&v.0)?)))
				.collect::<Option<_>>()
				.map(TypeExpr::Tuple),
			Expr::Index { collection, index } => match &collection.0 {
				Expr::Ident(n) => Some(TypeExpr::Generic(n.clone(), vec![TypeExpr::from_expr(&index.0)?])),
				_ => None,
			},
			_ => None,
		}
	}
}

#[derive(Debug, Clone, Default)]
// One arm of a `match` expression.
// `patterns` are compared to the subject (OR'd together).
// `binding @` names the subject value for the arm body.
// `body` runs when any pattern matches.
pub struct MatchArm {
	pub binding: Option<String>,
	pub patterns: Vec<Spanned<Expr>>,
	pub body: Vec<Spanned<Expr>>,
}

#[derive(Debug, Clone)]
pub struct UseItem {
	pub local: Spanned<String>,
	pub rename_of: Option<Spanned<String>>,
}

impl UseItem {
	pub fn remote(&self) -> &Spanned<String> {
		self.rename_of.as_ref().unwrap_or(&self.local)
	}
}

#[derive(Debug, Clone, Default)]
pub struct EnumVariant {
	pub name: String,
	pub disc: Option<i64>,
	pub raw: Option<String>,
	pub payload: Vec<Spanned<TypeExpr>>,
	pub names: Vec<String>,
}

// Capture list entry of an anon fn.
#[derive(Debug, Clone)]
pub enum Capture {
	ReadOnly(String),
	Mut(String),
	Move(String),
}

// A claimed trait and its type arguments.
pub type TraitRef = (String, Vec<Spanned<TypeExpr>>);

// Generic type parameter.
#[derive(Debug, Clone)]
pub struct TypeParam {
	pub name: String,
	pub bound: Option<String>,
	pub default: Option<Spanned<TypeExpr>>,
}

// Params access modifiers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Access {
	#[default]
	Read,
	Mut,
	Move,
}

impl std::fmt::Display for Access {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		f.write_str(match self {
			Access::Read => "read",
			Access::Mut => "mut",
			Access::Move => "move",
		})
	}
}

// A function parameter or struct field declaration.
#[derive(Debug, Clone)]
pub struct Param {
	pub name: String,
	pub typ: TypeExpr,
	pub span: Span,
	pub default: Option<Spanned<Expr>>,
	pub access: Access,
	pub mutable: bool,
	pub public: bool,
	pub annotations: Vec<Annotation>,
}

// A value attached to a definition or field.
// TODO: name this more generally and use it everywhere
pub type Annotation = Spanned<Expr>;
