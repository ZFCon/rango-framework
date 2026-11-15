//! Cascading deletion (`db.models.deletion.Collector`).

use std::collections::HashSet;

use indexmap::IndexMap;
use rango_core::exceptions::{Error, Result};

use crate::connection::Executor;
use crate::models::base::{ProtectedError, RestrictedError, send_delete_signals};
use crate::models::fields_db::prep_param;
use crate::models::meta::{FieldType, OnDelete, registry};
use crate::value::Value;

struct Plan {
    /// (label, pks) in collection order; deleted in reverse so children go first.
    deletes: Vec<(String, Vec<Value>)>,
    /// (label, column, field type, set value, parent pks) for SET_NULL & co.
    updates: Vec<(String, crate::models::meta::Field, Value, Vec<Value>)>,
}

async fn pks_where(db: &dyn Executor, table: &str, pk_col: &str, fk_col: &str, fk_field: &crate::models::meta::Field, parents: &[Value]) -> Result<Vec<Value>> {
    let vendor = db.vendor();
    let mut out = Vec::new();
    for chunk in parents.chunks(500) {
        let placeholders = vec!["%s"; chunk.len()].join(", ");
        let sql = format!("SELECT {} FROM {} WHERE {} IN ({placeholders})", vendor.quote(pk_col), vendor.quote(table), vendor.quote(fk_col));
        let params: Vec<Value> = chunk.iter().map(|p| prep_param(vendor, fk_field, p.clone())).collect();
        for row in db.query(&sql, &params).await? {
            out.push(row.values.into_iter().next().unwrap_or(Value::Null));
        }
    }
    Ok(out)
}

async fn collect(db: &dyn Executor, label: &str, pks: Vec<Value>, plan: &mut Plan, seen: &mut HashSet<(String, String)>) -> Result<()> {
    if pks.is_empty() {
        return Ok(());
    }
    let reg = registry();
    let fresh: Vec<Value> = pks.into_iter().filter(|p| seen.insert((label.to_lowercase(), format!("{p:?}")))).collect();
    if fresh.is_empty() {
        return Ok(());
    }
    plan.deletes.push((label.to_owned(), fresh.clone()));
    for ro in reg.related_objects(label).to_vec() {
        let owner = reg.get_or_err(&ro.model)?.clone();
        let fk = owner.get_field(&ro.field).expect("relation").clone();
        if fk.ty == FieldType::ManyToMany {
            continue; // through rows are covered by the through model's own CASCADE FKs
        }
        let rel = fk.rel.as_ref().expect("relation").clone();
        let child_pks = pks_where(db, &owner.db_table, &owner.pk().column(), &fk.column(), &fk, &fresh).await?;
        if child_pks.is_empty() {
            continue;
        }
        match rel.on_delete {
            OnDelete::Cascade => {
                Box::pin(collect(db, &owner.label(), child_pks, plan, seen)).await?;
            }
            OnDelete::Protect => {
                return Err(Error::Other(Box::new(ProtectedError {
                    model: reg.get_or_err(label)?.object_name.clone(),
                    refs: format!("'{}.{}'", owner.object_name, fk.name),
                })));
            }
            OnDelete::Restrict => {
                return Err(Error::Other(Box::new(RestrictedError {
                    model: reg.get_or_err(label)?.object_name.clone(),
                    refs: format!("'{}.{}'", owner.object_name, fk.name),
                })));
            }
            OnDelete::SetNull => plan.updates.push((owner.label(), fk.clone(), Value::Null, fresh.clone())),
            OnDelete::SetValue(v) => plan.updates.push((owner.label(), fk.clone(), v, fresh.clone())),
            OnDelete::SetDefault => {
                let v = match &fk.default {
                    Some(crate::models::meta::Default_::Value(v)) => v.clone(),
                    Some(crate::models::meta::Default_::Callable(f)) => f(),
                    None => Value::Null,
                };
                plan.updates.push((owner.label(), fk.clone(), v, fresh.clone()));
            }
            OnDelete::DoNothing => {}
        }
    }
    Ok(())
}

/// Delete `pks` of model `label` and everything that cascades. Returns `(total, per label)`.
pub async fn delete_pks(db: &dyn Executor, label: &str, pks: Vec<Value>) -> Result<(usize, IndexMap<String, usize>)> {
    let reg = registry();
    let vendor = db.vendor();
    let mut plan = Plan { deletes: Vec::new(), updates: Vec::new() };
    collect(db, label, pks, &mut plan, &mut HashSet::new()).await?;

    for (l, field, value, parents) in &plan.updates {
        let owner = reg.get_or_err(l)?;
        for chunk in parents.chunks(500) {
            let ph = vec!["%s"; chunk.len()].join(", ");
            let sql = format!("UPDATE {} SET {} = %s WHERE {} IN ({ph})", vendor.quote(&owner.db_table), vendor.quote(&field.column()), vendor.quote(&field.column()));
            let mut params = vec![prep_param(vendor, field, value.clone())];
            params.extend(chunk.iter().map(|p| prep_param(vendor, field, p.clone())));
            db.execute(&sql, &params).await?;
        }
    }
    let mut counts: IndexMap<String, usize> = IndexMap::new();
    let mut total = 0;
    for (l, pks) in plan.deletes.iter().rev() {
        let meta = reg.get_or_err(l)?;
        send_delete_signals(l, pks, true, db.alias()).await;
        let pk_field = meta.pk().clone();
        for chunk in pks.chunks(500) {
            let ph = vec!["%s"; chunk.len()].join(", ");
            let sql = format!("DELETE FROM {} WHERE {} IN ({ph})", vendor.quote(&meta.db_table), vendor.quote(&pk_field.column()));
            let params: Vec<Value> = chunk.iter().map(|p| prep_param(vendor, &pk_field, p.clone())).collect();
            let n = db.execute(&sql, &params).await? as usize;
            total += n;
            *counts.entry(meta.label()).or_default() += n;
        }
        send_delete_signals(l, pks, false, db.alias()).await;
    }
    Ok((total, counts))
}
