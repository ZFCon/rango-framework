//! `core.paginator`.
//!
//! Counting and slicing are async because the usual source is a database `QuerySet`
//! (which implements [`PageSource`] in the ORM milestone); `Vec<T>` works out of the box.

use std::future::Future;
use std::ops::Range;

use tokio::sync::OnceCell;

use crate::exceptions::Result;
use crate::utils::translation::gettext;

/// `InvalidPage` and subclasses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidPage {
    /// `PageNotAnInteger`.
    #[error("{0}")]
    PageNotAnInteger(String),
    /// `EmptyPage`.
    #[error("{0}")]
    EmptyPage(String),
}

/// Anything that can be counted and sliced into pages.
pub trait PageSource: Send + Sync {
    type Item: Send;
    fn count(&self) -> impl Future<Output = Result<usize>> + Send;
    fn slice(&self, range: Range<usize>) -> impl Future<Output = Result<Vec<Self::Item>>> + Send;
}

impl<T: Clone + Send + Sync> PageSource for Vec<T> {
    type Item = T;

    async fn count(&self) -> Result<usize> {
        Ok(self.len())
    }

    async fn slice(&self, range: Range<usize>) -> Result<Vec<T>> {
        Ok(self[range].to_vec())
    }
}

/// An entry of [`Paginator::get_elided_page_range`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageItem {
    Page(usize),
    Ellipsis,
}

/// Overridable messages (`error_messages=`).
#[derive(Debug, Clone, Default)]
pub struct ErrorMessages {
    pub invalid_page: Option<String>,
    pub min_page: Option<String>,
    pub no_results: Option<String>,
}

pub struct Paginator<S: PageSource> {
    object_list: S,
    pub per_page: usize,
    pub orphans: usize,
    pub allow_empty_first_page: bool,
    error_messages: ErrorMessages,
    count: OnceCell<usize>,
}

impl<S: PageSource> Paginator<S> {
    pub fn new(object_list: S, per_page: usize) -> Self {
        Self {
            object_list,
            // A page size of 0 would divide by zero; treat it as 1.
            per_page: per_page.max(1),
            orphans: 0,
            allow_empty_first_page: true,
            error_messages: ErrorMessages::default(),
            count: OnceCell::new(),
        }
    }

    pub fn orphans(mut self, orphans: usize) -> Self {
        self.orphans = orphans;
        self
    }

    pub fn allow_empty_first_page(mut self, allow: bool) -> Self {
        self.allow_empty_first_page = allow;
        self
    }

    pub fn error_messages(mut self, messages: ErrorMessages) -> Self {
        self.error_messages = messages;
        self
    }

    /// Total number of objects across all pages (cached).
    pub async fn count(&self) -> Result<usize> {
        self.count
            .get_or_try_init(|| self.object_list.count())
            .await
            .copied()
    }

    /// Total number of pages.
    pub async fn num_pages(&self) -> Result<usize> {
        let count = self.count().await?;
        if count == 0 && !self.allow_empty_first_page {
            return Ok(0);
        }
        let hits = count.saturating_sub(self.orphans).max(1);
        Ok(hits.div_ceil(self.per_page))
    }

    /// 1-based page numbers.
    pub async fn page_range(&self) -> Result<Range<usize>> {
        Ok(1..self.num_pages().await? + 1)
    }

    fn message(&self, which: &Option<String>, default: &str) -> String {
        which.clone().unwrap_or_else(|| gettext(default))
    }

    /// `validate_number()` for an already-numeric page.
    pub async fn validate_number(
        &self,
        number: i64,
    ) -> Result<std::result::Result<usize, InvalidPage>> {
        if number < 1 {
            return Ok(Err(InvalidPage::EmptyPage(self.message(
                &self.error_messages.min_page,
                "That page number is less than 1",
            ))));
        }
        if number as usize > self.num_pages().await? {
            return Ok(Err(InvalidPage::EmptyPage(self.message(
                &self.error_messages.no_results,
                "That page contains no results",
            ))));
        }
        Ok(Ok(number as usize))
    }

    /// `validate_number()` for user input such as `request.GET.get("page")`.
    pub async fn validate_number_str(
        &self,
        number: &str,
    ) -> Result<std::result::Result<usize, InvalidPage>> {
        match number.trim().parse::<i64>() {
            Ok(n) => self.validate_number(n).await,
            Err(_) => Ok(Err(InvalidPage::PageNotAnInteger(self.message(
                &self.error_messages.invalid_page,
                "That page number is not an integer",
            )))),
        }
    }

    /// `page(number)`.
    pub async fn page(
        &self,
        number: i64,
    ) -> Result<std::result::Result<Page<S::Item>, InvalidPage>> {
        let number = match self.validate_number(number).await? {
            Ok(n) => n,
            Err(e) => return Ok(Err(e)),
        };
        let count = self.count().await?;
        let bottom = (number - 1) * self.per_page;
        let mut top = bottom + self.per_page;
        if top + self.orphans >= count {
            top = count;
        }
        let object_list = self.object_list.slice(bottom..top).await?;
        Ok(Ok(Page {
            object_list,
            number,
            count,
            num_pages: self.num_pages().await?,
            per_page: self.per_page,
        }))
    }

    /// `get_page()`: always a valid page; bad input gives page 1, out of range gives the last page.
    pub async fn get_page(&self, number: Option<&str>) -> Result<Page<S::Item>> {
        let validated = match number {
            Some(n) => self.validate_number_str(n).await?,
            None => Err(InvalidPage::PageNotAnInteger(String::new())),
        };
        let number = match validated {
            Ok(n) => n,
            Err(InvalidPage::PageNotAnInteger(_)) => 1,
            Err(InvalidPage::EmptyPage(_)) => self.num_pages().await?,
        };
        // `None` only when there is no page at all (empty list with `allow_empty_first_page(false)`).
        self.page(number as i64).await?.map_err(|e| crate::Error::Other(format!("{e:?}").into()))
    }

    /// `get_elided_page_range()`: e.g. `1, 2, …, 40, 41, 42, 43, 44, 45, 46, …, 49, 50`.
    pub async fn get_elided_page_range(
        &self,
        number: i64,
        on_each_side: usize,
        on_ends: usize,
    ) -> Result<std::result::Result<Vec<PageItem>, InvalidPage>> {
        let number = match self.validate_number(number).await? {
            Ok(n) => n,
            Err(e) => return Ok(Err(e)),
        };
        let num_pages = self.num_pages().await?;
        let mut out = Vec::new();
        if num_pages <= (on_each_side + on_ends) * 2 {
            out.extend((1..=num_pages).map(PageItem::Page));
            return Ok(Ok(out));
        }
        if number > (1 + on_each_side + on_ends) + 1 {
            out.extend((1..=on_ends).map(PageItem::Page));
            out.push(PageItem::Ellipsis);
            out.extend((number - on_each_side..=number).map(PageItem::Page));
        } else {
            out.extend((1..=number).map(PageItem::Page));
        }
        if number + on_each_side + on_ends + 1 < num_pages {
            out.extend((number + 1..=number + on_each_side).map(PageItem::Page));
            out.push(PageItem::Ellipsis);
            out.extend((num_pages - on_ends + 1..=num_pages).map(PageItem::Page));
        } else {
            out.extend((number + 1..=num_pages).map(PageItem::Page));
        }
        Ok(Ok(out))
    }
}

/// `Paginator.ELLIPSIS`.
pub fn ellipsis() -> String {
    gettext("…")
}

/// One page of results. Carries the paginator's totals so it can outlive the paginator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub object_list: Vec<T>,
    pub number: usize,
    pub count: usize,
    pub num_pages: usize,
    pub per_page: usize,
}

impl<T> Page<T> {
    pub fn len(&self) -> usize {
        self.object_list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.object_list.is_empty()
    }

    pub fn has_next(&self) -> bool {
        self.number < self.num_pages
    }

    pub fn has_previous(&self) -> bool {
        self.number > 1
    }

    pub fn has_other_pages(&self) -> bool {
        self.has_previous() || self.has_next()
    }

    pub fn next_page_number(&self) -> std::result::Result<usize, InvalidPage> {
        if self.has_next() {
            Ok(self.number + 1)
        } else {
            Err(InvalidPage::EmptyPage(gettext(
                "That page contains no results",
            )))
        }
    }

    pub fn previous_page_number(&self) -> std::result::Result<usize, InvalidPage> {
        if self.number > 1 {
            Ok(self.number - 1)
        } else {
            Err(InvalidPage::EmptyPage(gettext(
                "That page number is less than 1",
            )))
        }
    }

    /// 1-based index of the first object on this page (0 if there are no objects).
    pub fn start_index(&self) -> usize {
        if self.count == 0 {
            0
        } else {
            self.per_page * (self.number - 1) + 1
        }
    }

    /// 1-based index of the last object on this page.
    pub fn end_index(&self) -> usize {
        if self.number == self.num_pages {
            self.count
        } else {
            self.number * self.per_page
        }
    }
}

impl<T> IntoIterator for Page<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.object_list.into_iter()
    }
}
