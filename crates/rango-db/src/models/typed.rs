//! Typed field handles, relation wrappers and the pieces `#[derive(Model)]` generates against.

use std::marker::PhantomData;

use rango_core::exceptions::{Error, Result};

use crate::connection::Executor;
use crate::models::base::{Model, RowData};
use crate::models::expressions::Expr;
use crate::models::q::{Q, Rhs};
use crate::models::queryset::QuerySet;
use crate::value::{ToValue, Value};

/// A model field (or traversed path) usable in lookups: `f.question_text().startswith("x")`.
#[derive(Clone, Debug)]
pub struct Path<T> {
    pub path: String,
    _t: PhantomData<fn() -> T>,
}

impl<T> Path<T> {
    pub fn new(path: impl Into<String>) -> Self {
        Path { path: path.into(), _t: PhantomData }
    }

    fn lk(&self, lookup: &str, v: impl ToValue) -> Q {
        Q::lookup(&format!("{}__{lookup}", self.path), v)
    }

    pub fn eq(&self, v: impl ToValue) -> Q {
        Q::lookup(&self.path, v)
    }
    pub fn exact(&self, v: impl ToValue) -> Q {
        self.eq(v)
    }
    pub fn iexact(&self, v: impl ToValue) -> Q {
        self.lk("iexact", v)
    }
    pub fn contains(&self, v: impl ToValue) -> Q {
        self.lk("contains", v)
    }
    pub fn icontains(&self, v: impl ToValue) -> Q {
        self.lk("icontains", v)
    }
    pub fn startswith(&self, v: impl ToValue) -> Q {
        self.lk("startswith", v)
    }
    pub fn istartswith(&self, v: impl ToValue) -> Q {
        self.lk("istartswith", v)
    }
    pub fn endswith(&self, v: impl ToValue) -> Q {
        self.lk("endswith", v)
    }
    pub fn iendswith(&self, v: impl ToValue) -> Q {
        self.lk("iendswith", v)
    }
    pub fn gt(&self, v: impl ToValue) -> Q {
        self.lk("gt", v)
    }
    pub fn gte(&self, v: impl ToValue) -> Q {
        self.lk("gte", v)
    }
    pub fn lt(&self, v: impl ToValue) -> Q {
        self.lk("lt", v)
    }
    pub fn lte(&self, v: impl ToValue) -> Q {
        self.lk("lte", v)
    }
    pub fn regex(&self, v: impl ToValue) -> Q {
        self.lk("regex", v)
    }
    pub fn iregex(&self, v: impl ToValue) -> Q {
        self.lk("iregex", v)
    }
    pub fn isnull(&self, v: bool) -> Q {
        self.lk("isnull", v)
    }
    /// `field__in=[...]`.
    pub fn is_in<V: ToValue>(&self, values: impl IntoIterator<Item = V>) -> Q {
        Q::is_in(&self.path, values)
    }
    /// `field__range=(lo, hi)`.
    pub fn range(&self, lo: impl ToValue, hi: impl ToValue) -> Q {
        Q::lookup_rhs(&format!("{}__range", self.path), Rhs::Range(lo.to_value(), hi.to_value()))
    }
    /// Compare with another field / expression: `f.votes().gt_expr(F("goal"))`.
    pub fn cmp_expr(&self, lookup: &str, e: Expr) -> Q {
        Q::compare_expr(&format!("{}__{lookup}", self.path), e)
    }

    fn tr<U>(&self, name: &str) -> Path<U> {
        Path::new(format!("{}__{name}", self.path))
    }
    pub fn year(&self) -> Path<i64> {
        self.tr("year")
    }
    pub fn month(&self) -> Path<i64> {
        self.tr("month")
    }
    pub fn day(&self) -> Path<i64> {
        self.tr("day")
    }
    pub fn week_day(&self) -> Path<i64> {
        self.tr("week_day")
    }
    pub fn quarter(&self) -> Path<i64> {
        self.tr("quarter")
    }
    pub fn hour(&self) -> Path<i64> {
        self.tr("hour")
    }
    pub fn minute(&self) -> Path<i64> {
        self.tr("minute")
    }
    pub fn second(&self) -> Path<i64> {
        self.tr("second")
    }
    pub fn lower(&self) -> Path<String> {
        self.tr("lower")
    }
    pub fn upper(&self) -> Path<String> {
        self.tr("upper")
    }
    pub fn length(&self) -> Path<i64> {
        self.tr("length")
    }

    /// Ordering string for `order_by`: `"-path"`.
    pub fn desc(&self) -> String {
        format!("-{}", self.path)
    }
    pub fn asc(&self) -> String {
        self.path.clone()
    }
}

/// `ForeignKey` / `OneToOneField` value: the target pk plus the related instance when loaded
/// by `select_related`/`prefetch_related`.
#[derive(Clone, Debug)]
pub struct ForeignKey<T: Model> {
    id: Value,
    cache: Option<Box<T>>,
}

impl<T: Model + PartialEq> PartialEq for ForeignKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

pub type OneToOne<T> = ForeignKey<T>;

impl<T: Model> ForeignKey<T> {
    pub fn new(id: impl ToValue) -> Self {
        ForeignKey { id: id.to_value(), cache: None }
    }

    /// Point at an existing instance (and cache it).
    pub fn to(instance: &T) -> Self
    where
        T: Clone,
    {
        ForeignKey { id: instance.pk_value(), cache: Some(Box::new(instance.clone())) }
    }

    pub fn null() -> Self {
        ForeignKey { id: Value::Null, cache: None }
    }

    /// The raw foreign-key value (`question_id`).
    pub fn id(&self) -> &Value {
        &self.id
    }

    pub fn is_null(&self) -> bool {
        self.id.is_null()
    }

    /// The related instance if it was loaded already.
    pub fn cached(&self) -> Option<&T> {
        self.cache.as_deref()
    }

    /// Related instance: the cached one, else a query (`instance.question` in the reference implementation).
    pub async fn get(&self, db: &dyn Executor) -> Result<T> {
        if let Some(c) = &self.cache {
            return Ok(T::from_data(RowData { fields: c.to_data(), ..RowData::default() })?);
        }
        if self.id.is_null() {
            return Err(Error::ObjectDoesNotExist { model: Some(T::label()), message: format!("{} matching query does not exist.", T::label()) });
        }
        QuerySet::<T>::new().filter_q(Q::lookup("pk", self.id.clone())).get(db).await
    }

    #[doc(hidden)]
    pub fn from_data(id: Option<&Value>, related: Option<&RowData>) -> Result<Self> {
        Ok(ForeignKey { id: id.cloned().unwrap_or(Value::Null), cache: related.cloned().map(|r| T::from_data(r)).transpose()?.map(Box::new) })
    }
}

impl<T: Model> ToValue for ForeignKey<T> {
    fn to_value(&self) -> Value {
        self.id.clone()
    }
}

/// `ManyToManyField` value holder: filled by `prefetch_related`. Use the generated
/// `<field>()` accessor for `add`/`remove`/`set`/`clear`/`all`.
#[derive(Clone, Debug)]
pub struct ManyToMany<T: Model> {
    cache: Option<Vec<T>>,
}

impl<T: Model> Default for ForeignKey<T> {
    fn default() -> Self {
        ForeignKey::null()
    }
}

impl<T: Model> Default for ManyToMany<T> {
    fn default() -> Self {
        ManyToMany { cache: None }
    }
}

impl<T: Model + PartialEq> PartialEq for ManyToMany<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T: Model> ManyToMany<T> {
    /// Instances loaded by `prefetch_related`, if any.
    pub fn prefetched(&self) -> Option<&[T]> {
        self.cache.as_deref()
    }

    #[doc(hidden)]
    pub fn from_prefetched(rows: Option<&Vec<RowData>>) -> Result<Self> {
        Ok(ManyToMany { cache: rows.map(|r| r.iter().cloned().map(T::from_data).collect::<Result<Vec<_>>>()).transpose()? })
    }
}

/// Generated `<m2m field>()` accessor: the relation manager for one instance.
pub struct ManyAccessor<T: Model> {
    pub owner_label: String,
    pub field: &'static str,
    pub pk: Value,
    pub reverse_query_name: String,
    pub _t: PhantomData<fn() -> T>,
}

impl<T: Model> ManyAccessor<T> {
    /// `instance.field.all()` as a queryset.
    pub fn all(&self) -> QuerySet<T> {
        QuerySet::<T>::new().filter_q(Q::lookup(&self.reverse_query_name, self.pk.clone()))
    }

    pub async fn add(&self, db: &dyn Executor, items: &[&T]) -> Result<usize> {
        let ids: Vec<Value> = items.iter().map(|i| i.pk_value()).collect();
        crate::models::related::add(db, &self.owner_label, self.field, &self.pk, &ids).await
    }

    pub async fn remove(&self, db: &dyn Executor, items: &[&T]) -> Result<u64> {
        let ids: Vec<Value> = items.iter().map(|i| i.pk_value()).collect();
        crate::models::related::remove(db, &self.owner_label, self.field, &self.pk, &ids).await
    }

    pub async fn set(&self, db: &dyn Executor, items: &[&T]) -> Result<()> {
        let ids: Vec<Value> = items.iter().map(|i| i.pk_value()).collect();
        crate::models::related::set(db, &self.owner_label, self.field, &self.pk, &ids).await
    }

    pub async fn clear(&self, db: &dyn Executor) -> Result<u64> {
        crate::models::related::clear(db, &self.owner_label, self.field, &self.pk).await
    }
}

/// Root of a model's generated `…Fields` type.
pub trait FieldsRoot: Default + Send + Sync {}
impl<F: Default + Send + Sync> FieldsRoot for F {}

/// A generated `…Fields` type that can be re-rooted under a relation path.
pub trait PrefixedFields: Default {
    fn with_prefix(prefix: String) -> Self;
    fn prefix(&self) -> &str;
}
