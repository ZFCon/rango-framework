//! `prefetch_related`: batch-load related rows and attach them to `RowData`.

use std::collections::HashMap;

use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};

use crate::connection::Executor;
use crate::models::base::{Dynamic, RowData};
use crate::models::fields_db::prep_param;
use crate::models::meta::{FieldType, ModelMeta, registry};
use crate::models::q::Q;
use crate::models::queryset::QuerySet;
use crate::value::Value;

fn key(v: &Value) -> String {
    format!("{v:?}")
}

async fn fetch_rows(db: &dyn Executor, label: &str, q: Q) -> Result<Vec<RowData>> {
    let qs = QuerySet::<Dynamic>::for_label(label).filter_q(q);
    Ok(qs.fetch_data(db).await?.0)
}

/// Prefetch `lookups` (`"choice_set"`, `"author"`, `"choice_set__votes_log"`) for `data`.
pub async fn prefetch(db: &dyn Executor, meta: &ModelMeta, data: &mut [RowData], lookups: &[String]) -> Result<()> {
    for lookup in lookups {
        Box::pin(prefetch_one(db, meta, data, lookup)).await?;
    }
    Ok(())
}

async fn prefetch_one(db: &dyn Executor, meta: &ModelMeta, data: &mut [RowData], lookup: &str) -> Result<()> {
    let (first, rest) = match lookup.split_once("__") {
        Some((a, b)) => (a, Some(b)),
        None => (lookup, None),
    };
    let reg = registry();
    let vendor = db.vendor();
    let pk_attname = meta.pk().attname();

    // Forward FK / O2O.
    if let Some(f) = meta.get_field(first).filter(|f| matches!(f.ty, FieldType::ForeignKey | FieldType::OneToOne) && f.name == first) {
        let target = reg.target_of(meta, f)?.clone();
        let fk_vals: Vec<Value> = data.iter().filter_map(|d| d.fields.get(&f.attname()).cloned()).filter(|v| !v.is_null()).collect();
        if fk_vals.is_empty() {
            return Ok(());
        }
        let rows = fetch_rows(db, &target.label(), Q::is_in("pk", fk_vals)).await?;
        let by_pk: HashMap<String, RowData> = rows.into_iter().map(|r| (key(r.fields.get(&target.pk().attname()).unwrap_or(&Value::Null)), r)).collect();
        for d in data.iter_mut() {
            if let Some(v) = d.fields.get(&f.attname()) {
                if let Some(r) = by_pk.get(&key(v)) {
                    d.related.insert(first.to_owned(), r.clone());
                }
            }
        }
        if let Some(rest) = rest {
            let mut children: Vec<RowData> = data.iter().filter_map(|d| d.related.get(first).cloned()).collect();
            Box::pin(prefetch_one(db, &target, &mut children, rest)).await?;
            let mut it = children.into_iter();
            for d in data.iter_mut() {
                if d.related.contains_key(first) {
                    if let Some(c) = it.next() {
                        d.related.insert(first.to_owned(), c);
                    }
                }
            }
        }
        return Ok(());
    }

    // Forward M2M.
    if let Some(f) = meta.get_field(first).filter(|f| f.ty == FieldType::ManyToMany) {
        let target = reg.target_of(meta, f)?.clone();
        let through = reg.through_of(meta, f)?.clone();
        let (from_col, to_col) = crate::models::query::m2m_fk_names(meta, &through, &target)?;
        attach_m2m(db, data, &pk_attname, meta, &through, &from_col, &to_col, &target, first).await?;
        return recurse(db, &target, data, first, rest).await;
    }

    // Reverse relations.
    for ro in reg.related_objects(&meta.label()) {
        if ro.accessor_name != first && ro.query_name != first {
            continue;
        }
        let owner = reg.get_or_err(&ro.model)?.clone();
        let fk = owner.get_field(&ro.field).expect("relation field").clone();
        let accessor = ro.accessor_name.clone();
        if fk.ty == FieldType::ManyToMany {
            let through = reg.through_of(&owner, &fk)?.clone();
            let (owner_col, meta_col) = crate::models::query::m2m_fk_names(&owner, &through, meta)?;
            attach_m2m(db, data, &pk_attname, meta, &through, &meta_col, &owner_col, &owner, &accessor).await?;
            return recurse(db, &owner, data, &accessor, rest).await;
        }
        let pks: Vec<Value> = data.iter().filter_map(|d| d.fields.get(&pk_attname).cloned()).collect();
        let rows = fetch_rows(db, &owner.label(), Q::is_in(&fk.name, pks)).await?;
        let mut grouped: HashMap<String, Vec<RowData>> = HashMap::new();
        for r in rows {
            if let Some(v) = r.fields.get(&fk.attname()) {
                grouped.entry(key(v)).or_default().push(r);
            }
        }
        for d in data.iter_mut() {
            let list = d.fields.get(&pk_attname).and_then(|p| grouped.get(&key(p))).cloned().unwrap_or_default();
            d.prefetched.insert(accessor.clone(), list);
        }
        return recurse(db, &owner, data, &accessor, rest).await;
    }
    let _ = vendor;
    Err(Error::Other(
        format!("Cannot find '{first}' on {} object, '{lookup}' is an invalid parameter to prefetch_related()", meta.object_name).into(),
    ))
}

async fn recurse(db: &dyn Executor, target: &ModelMeta, data: &mut [RowData], name: &str, rest: Option<&str>) -> Result<()> {
    let Some(rest) = rest else { return Ok(()) };
    let mut flat: Vec<RowData> = data.iter().flat_map(|d| d.prefetched.get(name).cloned().unwrap_or_default()).collect();
    Box::pin(prefetch_one(db, target, &mut flat, rest)).await?;
    let mut it = flat.into_iter();
    for d in data.iter_mut() {
        if let Some(list) = d.prefetched.get_mut(name) {
            for slot in list.iter_mut() {
                if let Some(n) = it.next() {
                    *slot = n;
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn attach_m2m(
    db: &dyn Executor,
    data: &mut [RowData],
    pk_attname: &str,
    meta: &ModelMeta,
    through: &ModelMeta,
    from_col: &str,
    to_col: &str,
    target: &ModelMeta,
    name: &str,
) -> Result<()> {
    let vendor = db.vendor();
    let pks: Vec<Value> = data.iter().filter_map(|d| d.fields.get(pk_attname).cloned()).collect();
    if pks.is_empty() {
        return Ok(());
    }
    let pk_field = meta.pk().clone();
    let ph = vec!["%s"; pks.len()].join(", ");
    let sql = format!(
        "SELECT {}, {} FROM {} WHERE {} IN ({ph})",
        vendor.quote(from_col),
        vendor.quote(to_col),
        vendor.quote(&through.db_table),
        vendor.quote(from_col)
    );
    let params: Vec<Value> = pks.iter().map(|p| prep_param(vendor, &pk_field, p.clone())).collect();
    let pairs = db.query(&sql, &params).await?;
    let mut by_owner: IndexMap<String, Vec<Value>> = IndexMap::new();
    let mut targets: Vec<Value> = Vec::new();
    for r in &pairs {
        let (a, b) = (r.values[0].clone(), r.values[1].clone());
        targets.push(b.clone());
        by_owner.entry(key(&a)).or_default().push(b);
    }
    let rows = if targets.is_empty() { vec![] } else { fetch_rows(db, &target.label(), Q::is_in("pk", targets)).await? };
    let by_pk: HashMap<String, RowData> = rows.into_iter().map(|r| (key(r.fields.get(&target.pk().attname()).unwrap_or(&Value::Null)), r)).collect();
    for d in data.iter_mut() {
        let list: Vec<RowData> = d
            .fields
            .get(pk_attname)
            .and_then(|p| by_owner.get(&key(p)))
            .map(|ts| ts.iter().filter_map(|t| by_pk.get(&key(t)).cloned()).collect())
            .unwrap_or_default();
        d.prefetched.insert(name.to_owned(), list);
    }
    Ok(())
}
