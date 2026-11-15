//! Query expressions (`db.models.expressions`, `functions`, `aggregates`).

use crate::models::q::Q;
use crate::models::query::Query;
use crate::value::{ToValue, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitLeftShift,
    BitRightShift,
}

impl Op {
    pub fn sql(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Mod => "%%",
            Op::Pow => "^",
            Op::BitAnd => "&",
            Op::BitOr => "|",
            Op::BitLeftShift => "<<",
            Op::BitRightShift => ">>",
        }
    }
}

/// Aggregate functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggKind {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    StdDev,
    Variance,
}

impl AggKind {
    pub fn function(self) -> &'static str {
        match self {
            AggKind::Count => "COUNT",
            AggKind::Sum => "SUM",
            AggKind::Avg => "AVG",
            AggKind::Min => "MIN",
            AggKind::Max => "MAX",
            AggKind::StdDev => "STDDEV_POP",
            AggKind::Variance => "VAR_POP",
        }
    }
}

/// A query expression.
#[derive(Clone, Debug)]
pub enum Expr {
    /// `F("path__to__field")` (or an annotation name).
    F(String),
    /// A literal bound as a parameter (`Value(...)`).
    Val(Value),
    /// `OuterRef("field")`: a field of the enclosing query.
    OuterRef(String),
    /// `Func`: `NAME(arg, ...)`.
    Func { name: String, args: Vec<Expr> },
    /// Arithmetic / bitwise combination (`F("a") + 1`).
    Combined(Box<Expr>, Op, Box<Expr>),
    Neg(Box<Expr>),
    Case { whens: Vec<(Q, Expr)>, default: Option<Box<Expr>> },
    /// `Count("x", distinct=..., filter=...)`; `Count("*")` is `Expr::Agg { expr: Star }`.
    Agg { kind: AggKind, expr: Box<Expr>, distinct: bool, filter: Option<Box<Q>> },
    Star,
    Subquery(Box<Query>),
    Exists(Box<Query>),
    /// `Cast(expr, output_field)`.
    Cast(Box<Expr>, crate::models::meta::FieldType),
    /// `expr.asc()` / `.desc()` with optional NULLS placement.
    Ordered { expr: Box<Expr>, descending: bool, nulls_first: Option<bool> },
    Raw(String, Vec<Value>),
}

impl Expr {
    pub fn f(path: &str) -> Expr {
        Expr::F(path.to_owned())
    }

    pub fn val(v: impl ToValue) -> Expr {
        Expr::Val(v.to_value())
    }

    pub fn func(name: &str, args: Vec<Expr>) -> Expr {
        Expr::Func { name: name.to_owned(), args }
    }

    pub fn asc(self) -> Expr {
        Expr::Ordered { expr: Box::new(self), descending: false, nulls_first: None }
    }

    pub fn desc(self) -> Expr {
        Expr::Ordered { expr: Box::new(self), descending: true, nulls_first: None }
    }

    pub fn nulls_first(self) -> Expr {
        match self {
            Expr::Ordered { expr, descending, .. } => Expr::Ordered { expr, descending, nulls_first: Some(true) },
            e => Expr::Ordered { expr: Box::new(e), descending: false, nulls_first: Some(true) },
        }
    }

    pub fn nulls_last(self) -> Expr {
        match self {
            Expr::Ordered { expr, descending, .. } => Expr::Ordered { expr, descending, nulls_first: Some(false) },
            e => Expr::Ordered { expr: Box::new(e), descending: false, nulls_first: Some(false) },
        }
    }

    fn bin(self, op: Op, rhs: impl Into<Expr>) -> Expr {
        Expr::Combined(Box::new(self), op, Box::new(rhs.into()))
    }

    pub fn add(self, rhs: impl Into<Expr>) -> Expr {
        self.bin(Op::Add, rhs)
    }
    pub fn sub(self, rhs: impl Into<Expr>) -> Expr {
        self.bin(Op::Sub, rhs)
    }
    pub fn mul(self, rhs: impl Into<Expr>) -> Expr {
        self.bin(Op::Mul, rhs)
    }
    pub fn div(self, rhs: impl Into<Expr>) -> Expr {
        self.bin(Op::Div, rhs)
    }
    pub fn rem(self, rhs: impl Into<Expr>) -> Expr {
        self.bin(Op::Mod, rhs)
    }
}

impl<T: ToValue> From<T> for Expr {
    fn from(v: T) -> Self {
        Expr::Val(v.to_value())
    }
}

macro_rules! impl_std_op {
    ($tr:ident, $m:ident, $op:expr) => {
        impl<R: Into<Expr>> std::ops::$tr<R> for Expr {
            type Output = Expr;
            fn $m(self, rhs: R) -> Expr {
                Expr::Combined(Box::new(self), $op, Box::new(rhs.into()))
            }
        }
    };
}
impl_std_op!(Add, add, Op::Add);
impl_std_op!(Sub, sub, Op::Sub);
impl_std_op!(Mul, mul, Op::Mul);
impl_std_op!(Div, div, Op::Div);
impl_std_op!(Rem, rem, Op::Mod);

impl std::ops::Neg for Expr {
    type Output = Expr;
    fn neg(self) -> Expr {
        Expr::Neg(Box::new(self))
    }
}

/// `F("name")`.
#[allow(non_snake_case)]
pub fn F(path: &str) -> Expr {
    Expr::F(path.to_owned())
}

/// `Value(v)`.
#[allow(non_snake_case)]
pub fn V(v: impl ToValue) -> Expr {
    Expr::val(v)
}

/// `OuterRef("name")`.
#[allow(non_snake_case)]
pub fn OuterRef(path: &str) -> Expr {
    Expr::OuterRef(path.to_owned())
}

fn agg(kind: AggKind, e: Expr) -> Expr {
    Expr::Agg { kind, expr: Box::new(e), distinct: false, filter: None }
}

#[allow(non_snake_case)]
pub fn Count(path: &str) -> Expr {
    agg(AggKind::Count, if path == "*" { Expr::Star } else { Expr::f(path) })
}
#[allow(non_snake_case)]
pub fn Sum(path: &str) -> Expr {
    agg(AggKind::Sum, Expr::f(path))
}
#[allow(non_snake_case)]
pub fn Avg(path: &str) -> Expr {
    agg(AggKind::Avg, Expr::f(path))
}
#[allow(non_snake_case)]
pub fn Min(path: &str) -> Expr {
    agg(AggKind::Min, Expr::f(path))
}
#[allow(non_snake_case)]
pub fn Max(path: &str) -> Expr {
    agg(AggKind::Max, Expr::f(path))
}

impl Expr {
    /// `Count("x", distinct=True)`.
    pub fn distinct(mut self) -> Expr {
        if let Expr::Agg { distinct, .. } = &mut self {
            *distinct = true;
        }
        self
    }

    /// `Count("x", filter=Q(...))`.
    pub fn filter(mut self, q: Q) -> Expr {
        if let Expr::Agg { filter, .. } = &mut self {
            *filter = Some(Box::new(q));
        }
        self
    }
}

macro_rules! simple_func {
    ($($fn_name:ident => $sql:literal),* $(,)?) => {$(
        #[allow(non_snake_case)]
        pub fn $fn_name(e: impl Into<Expr>) -> Expr { Expr::func($sql, vec![e.into()]) }
    )*};
}
simple_func!(Lower => "LOWER", Upper => "UPPER", Length => "LENGTH", Abs => "ABS", Trim => "TRIM", LTrim => "LTRIM", RTrim => "RTRIM",
    Ceil => "CEILING", Floor => "FLOOR", Sqrt => "SQRT");

#[allow(non_snake_case)]
pub fn Coalesce(args: Vec<Expr>) -> Expr {
    Expr::func("COALESCE", args)
}
#[allow(non_snake_case)]
pub fn Greatest(args: Vec<Expr>) -> Expr {
    Expr::func("GREATEST", args)
}
#[allow(non_snake_case)]
pub fn Least(args: Vec<Expr>) -> Expr {
    Expr::func("LEAST", args)
}
#[allow(non_snake_case)]
pub fn Concat(args: Vec<Expr>) -> Expr {
    Expr::func("CONCAT", args)
}
#[allow(non_snake_case)]
pub fn Substr(e: impl Into<Expr>, pos: i64, length: Option<i64>) -> Expr {
    let mut args = vec![e.into(), Expr::val(pos)];
    if let Some(l) = length {
        args.push(Expr::val(l));
    }
    Expr::func("SUBSTR", args)
}
#[allow(non_snake_case)]
pub fn Cast(e: impl Into<Expr>, ty: crate::models::meta::FieldType) -> Expr {
    Expr::Cast(Box::new(e.into()), ty)
}
/// `Case(When(q, then=...), ..., default=...)`.
#[allow(non_snake_case)]
pub fn Case(whens: Vec<(Q, Expr)>, default: Option<Expr>) -> Expr {
    Expr::Case { whens, default: default.map(Box::new) }
}
/// `Exists(queryset)`.
#[allow(non_snake_case)]
pub fn Exists(q: Query) -> Expr {
    Expr::Exists(Box::new(q))
}
/// `Subquery(queryset)`.
#[allow(non_snake_case)]
pub fn Subquery(q: Query) -> Expr {
    Expr::Subquery(Box::new(q))
}
