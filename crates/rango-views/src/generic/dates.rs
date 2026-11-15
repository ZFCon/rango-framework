//! `views.generic.dates`: archive views.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Datelike, Duration, NaiveDate};
use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};
use rango_db::models::meta::FieldType;
use rango_db::models::{Model, QuerySet};
use rango_db::{Db, Value as Db_};
use rango_http::{HttpRequest, HttpResponse};
use rango_template::{Context, TemplateResponse, Value as T};
use rango_urls::ViewArgs;

use super::base::View;
use super::detail::{DetailView, QuerySetHook};
use super::util::{db_to_template, model_name_lower, model_value, not_found, url_kwargs_context};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateViewKind {
    ArchiveIndex,
    Year,
    Month,
    Week,
    Day,
    Today,
    Detail,
}

/// `ArchiveIndexView`, `YearArchiveView`, `MonthArchiveView`, `WeekArchiveView`, `DayArchiveView`,
/// `TodayArchiveView` and `DateDetailView` in one builder; pick with [`DateViewKind`].
pub struct DateView<M: Model> {
    pub kind: DateViewKind,
    pub date_field: String,
    pub queryset: QuerySetHook<M>,
    pub using: String,
    pub template_name: Option<String>,
    pub allow_future: bool,
    pub allow_empty: bool,
    pub make_object_list: bool,
    pub month_format: String,
    pub day_format: String,
    pub context_object_name: Option<String>,
    pub extra_context: IndexMap<String, T>,
    detail: DetailView<M>,
}

impl<M: Model> DateView<M> {
    pub fn new(kind: DateViewKind, date_field: &str) -> Self {
        Self {
            kind,
            date_field: date_field.to_owned(),
            queryset: Arc::new(|_| M::objects()),
            using: "default".into(),
            template_name: None,
            allow_future: false,
            allow_empty: kind == DateViewKind::ArchiveIndex,
            make_object_list: false,
            month_format: "%b".into(),
            day_format: "%d".into(),
            context_object_name: None,
            extra_context: IndexMap::new(),
            detail: DetailView::new(),
        }
    }
    pub fn template_name(mut self, n: &str) -> Self {
        self.template_name = Some(n.to_owned());
        self
    }
    pub fn allow_future(mut self, b: bool) -> Self {
        self.allow_future = b;
        self
    }
    pub fn allow_empty(mut self, b: bool) -> Self {
        self.allow_empty = b;
        self
    }
    pub fn make_object_list(mut self, b: bool) -> Self {
        self.make_object_list = b;
        self
    }
    pub fn month_format(mut self, f: &str) -> Self {
        self.month_format = f.to_owned();
        self
    }
    pub fn using(mut self, alias: &str) -> Self {
        self.using = alias.to_owned();
        self
    }
    pub fn queryset(mut self, f: impl Fn(&HttpRequest) -> QuerySet<M> + Send + Sync + 'static) -> Self {
        self.queryset = Arc::new(f);
        self
    }
    pub fn extra(mut self, key: &str, value: impl Into<T>) -> Self {
        self.extra_context.insert(key.to_owned(), value.into());
        self
    }

    fn arg_str(args: &ViewArgs, k: &str) -> Result<String> {
        args.kwargs.get(k).map(ToString::to_string).ok_or_else(|| Error::improperly_configured(format!("Generic date view must be called with a '{k}' in the URLconf.")))
    }

    fn parse_date(&self, args: &ViewArgs) -> Result<(NaiveDate, NaiveDate)> {
        let bad = |what: &str, v: &str| not_found(format!("Invalid date string '{v}' given format for {what}"));
        let ymd = |y: &str, m: &str, d: &str| -> Option<NaiveDate> {
            let fmt = format!("%Y {} {}", self.month_format, self.day_format);
            NaiveDate::parse_from_str(&format!("{y} {m} {d}"), &fmt).ok()
        };
        let next_month = |d: NaiveDate| if d.month() == 12 { NaiveDate::from_ymd_opt(d.year() + 1, 1, 1) } else { NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1) }.expect("valid date");
        match self.kind {
            DateViewKind::ArchiveIndex => Ok((NaiveDate::MIN, NaiveDate::MAX)),
            DateViewKind::Year => {
                let y = Self::arg_str(args, "year")?;
                let start = y.parse::<i32>().ok().and_then(|y| NaiveDate::from_ymd_opt(y, 1, 1)).ok_or_else(|| bad("year", &y))?;
                Ok((start, NaiveDate::from_ymd_opt(start.year() + 1, 1, 1).expect("valid date")))
            }
            DateViewKind::Month => {
                let (y, m) = (Self::arg_str(args, "year")?, Self::arg_str(args, "month")?);
                let start = ymd(&y, &m, "1").ok_or_else(|| bad("month", &format!("{y} {m}")))?;
                Ok((start, next_month(start)))
            }
            DateViewKind::Week => {
                let (y, w) = (Self::arg_str(args, "year")?, Self::arg_str(args, "week")?);
                // `week_format = "%U"`: weeks start on Sunday; week 0 is the days before the first Sunday.
                let jan1 = y.parse::<i32>().ok().and_then(|y| NaiveDate::from_ymd_opt(y, 1, 1)).ok_or_else(|| bad("year", &y))?;
                let w = w.parse::<i64>().map_err(|_| bad("week", &w))?;
                let first_sunday = jan1 + Duration::days(i64::from((7 - jan1.weekday().num_days_from_sunday()) % 7));
                let start = first_sunday + Duration::weeks(w - 1);
                Ok((start, start + Duration::weeks(1)))
            }
            DateViewKind::Day | DateViewKind::Detail => {
                let (y, m, d) = (Self::arg_str(args, "year")?, Self::arg_str(args, "month")?, Self::arg_str(args, "day")?);
                let start = ymd(&y, &m, &d).ok_or_else(|| bad("day", &format!("{y} {m} {d}")))?;
                Ok((start, start + Duration::days(1)))
            }
            DateViewKind::Today => {
                let today = rango_core::utils::timezone::now().date_naive();
                Ok((today, today + Duration::days(1)))
            }
        }
    }

    fn date_of(&self, obj: &M) -> Option<NaiveDate> {
        let meta = M::meta().ok()?;
        let f = meta.get_field(&self.date_field)?;
        match obj.to_data().get(&f.attname())? {
            Db_::Date(d) => Some(*d),
            Db_::DateTime(d) => Some(d.date_naive()),
            Db_::NaiveDateTime(d) => Some(d.date()),
            _ => None,
        }
    }

    fn bound(&self, d: NaiveDate) -> Result<Db_> {
        let meta = M::meta()?;
        let is_dt = meta.get_field(&self.date_field).is_some_and(|f| f.ty == FieldType::DateTime);
        Ok(if is_dt { Db_::DateTime(d.and_hms_opt(0, 0, 0).expect("midnight").and_utc()) } else { Db_::Date(d) })
    }
}

fn first_of(kind: DateViewKind, d: NaiveDate) -> NaiveDate {
    match kind {
        DateViewKind::Year => NaiveDate::from_ymd_opt(d.year(), 1, 1),
        DateViewKind::Month => NaiveDate::from_ymd_opt(d.year(), d.month(), 1),
        _ => Some(d),
    }
    .expect("valid date")
}

#[async_trait]
impl<M: Model> View for DateView<M> {
    async fn get(&self, req: HttpRequest, args: &ViewArgs) -> Result<HttpResponse> {
        let db = Db::using(&self.using);
        let meta = M::meta()?;
        let mut qs = (self.queryset)(&req);
        let (start, end) = self.parse_date(args)?;
        let today = rango_core::utils::timezone::now().date_naive();
        if !self.allow_future && start > today && self.kind != DateViewKind::ArchiveIndex {
            return Err(not_found("Future dates not allowed"));
        }
        if self.kind != DateViewKind::ArchiveIndex {
            qs = qs.filter_kw(&format!("{}__gte", self.date_field), self.bound(start)?).filter_kw(&format!("{}__lt", self.date_field), self.bound(end)?);
        }
        if !self.allow_future {
            qs = qs.filter_kw(&format!("{}__lt", self.date_field), self.bound(today + Duration::days(1))?);
        }
        let mut ctx = Context::new();
        for (k, v) in url_kwargs_context(args) {
            ctx.set(&k, v);
        }
        if self.kind == DateViewKind::Detail {
            let mut d = DetailView::<M>::new();
            d.using.clone_from(&self.using);
            d.pk_url_kwarg = self.detail.pk_url_kwarg.clone();
            d.slug_field = self.detail.slug_field.clone();
            let qs2 = qs;
            d.queryset = Arc::new(move |_| qs2.clone());
            let obj = d.get_object(&req, args, &db).await?;
            ctx = d.object_context(&req, args, &obj)?;
            let names = vec![self.template_name.clone().unwrap_or_else(|| format!("{}/{}_detail.html", meta.app_label, model_name_lower(&meta)))];
            return Ok(TemplateResponse::new(&req, names, ctx));
        }
        let desc = format!("-{}", self.date_field);
        let objects = qs.order_by(&[match self.kind {
            DateViewKind::ArchiveIndex => desc.as_str(),
            _ => self.date_field.as_str(),
        }]);
        let objects = if self.kind == DateViewKind::ArchiveIndex { objects.slice(0, Some(15)) } else { objects };
        let list: Vec<M> = objects.fetch(&db).await?;
        if list.is_empty() && !self.allow_empty {
            return Err(not_found("No objects available"));
        }
        // date_list: distinct years/months/days of the matching objects.
        let mut dates: Vec<NaiveDate> = list.iter().filter_map(|o| self.date_of(o)).map(|d| match self.kind {
            DateViewKind::ArchiveIndex => first_of(DateViewKind::Year, d),
            DateViewKind::Year => first_of(DateViewKind::Month, d),
            _ => d,
        }).collect();
        dates.sort();
        dates.dedup();
        if self.kind == DateViewKind::ArchiveIndex {
            dates.reverse();
        }
        let key = |d: NaiveDate| T::Date(d);
        match self.kind {
            DateViewKind::ArchiveIndex => {
                ctx.set("date_list", T::List(dates.iter().map(|d| key(*d)).collect()));
                ctx.set("latest", T::List(list.iter().map(model_value).collect()));
            }
            DateViewKind::Year => {
                ctx.set("date_list", T::List(dates.iter().map(|d| key(*d)).collect()));
                ctx.set("year", T::Date(start));
                ctx.set("previous_year", T::Date(NaiveDate::from_ymd_opt(start.year() - 1, 1, 1).expect("valid date")));
                if self.allow_future || end <= today {
                    ctx.set("next_year", T::Date(end));
                }
                if self.make_object_list {
                    ctx.set("object_list", T::List(list.iter().map(model_value).collect()));
                }
            }
            DateViewKind::Month => {
                ctx.set("date_list", T::List(dates.iter().map(|d| key(*d)).collect()));
                ctx.set("month", T::Date(start));
                ctx.set("previous_month", T::Date(first_of(DateViewKind::Month, start - Duration::days(1))));
                if self.allow_future || end <= today {
                    ctx.set("next_month", T::Date(end));
                }
            }
            DateViewKind::Week => {
                ctx.set("week", T::Date(start));
                ctx.set("previous_week", T::Date(start - Duration::weeks(1)));
                if self.allow_future || end <= today {
                    ctx.set("next_week", T::Date(end));
                }
            }
            DateViewKind::Day | DateViewKind::Today => {
                ctx.set("day", T::Date(start));
                ctx.set("previous_day", T::Date(start - Duration::days(1)));
                if self.allow_future || end <= today {
                    ctx.set("next_day", T::Date(end));
                }
            }
            DateViewKind::Detail => unreachable!(),
        }
        let name = self.context_object_name.clone().unwrap_or_else(|| format!("{}_list", model_name_lower(&meta)));
        let l = T::List(list.iter().map(model_value).collect());
        ctx.set("object_list", l.clone());
        ctx.set(name.as_str(), l);
        for (k, v) in &self.extra_context {
            ctx.set(k, v.clone());
        }
        let suffix = match self.kind {
            DateViewKind::ArchiveIndex => "_archive",
            DateViewKind::Year => "_archive_year",
            DateViewKind::Month => "_archive_month",
            DateViewKind::Week => "_archive_week",
            DateViewKind::Day | DateViewKind::Today => "_archive_day",
            DateViewKind::Detail => "_detail",
        };
        let names = vec![self.template_name.clone().unwrap_or_else(|| format!("{}/{}{suffix}.html", meta.app_label, model_name_lower(&meta)))];
        let _ = db_to_template;
        Ok(TemplateResponse::new(&req, names, ctx))
    }
}
