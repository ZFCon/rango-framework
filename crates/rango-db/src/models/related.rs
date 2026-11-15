//! Many-to-many and reverse-FK manager operations (`add`, `remove`, `set`, `clear`).

use rango_core::exceptions::Result;

use crate::connection::Executor;
use crate::models::fields_db::prep_param;
use crate::models::meta::registry;
use crate::models::query::m2m_fk_names;
use crate::models::signals::{M2M_CHANGED, M2mAction, M2mEvent};
use crate::value::Value;

struct M2m {
    table: String,
    owner_col: String,
    target_col: String,
    owner_pk: crate::models::meta::Field,
    target_pk: crate::models::meta::Field,
}

fn resolve(label: &str, field: &str) -> Result<M2m> {
    let reg = registry();
    let owner = reg.get_or_err(label)?.clone();
    let f = owner
        .get_field(field)
        .filter(|f| f.ty == crate::models::meta::FieldType::ManyToMany)
        .ok_or_else(|| rango_core::exceptions::Error::field_error(format!("{label} has no many-to-many field '{field}'")))?
        .clone();
    let target = reg.target_of(&owner, &f)?.clone();
    let through = reg.through_of(&owner, &f)?.clone();
    let (owner_col, target_col) = m2m_fk_names(&owner, &through, &target)?;
    Ok(M2m { table: through.db_table.clone(), owner_col, target_col, owner_pk: owner.pk().clone(), target_pk: target.pk().clone() })
}

async fn signal(label: &str, field: &str, pk: &Value, set: &[Value], action: M2mAction) {
    M2M_CHANGED
        .send(Some(label), M2mEvent { action, label: label.to_owned(), field: field.to_owned(), pk: pk.clone(), pk_set: set.to_vec() })
        .await;
}

/// `instance.field.add(*targets)`: skips pairs that already exist.
pub async fn add(db: &dyn Executor, label: &str, field: &str, owner_pk: &Value, targets: &[Value]) -> Result<usize> {
    let m = resolve(label, field)?;
    let vendor = db.vendor();
    signal(label, field, owner_pk, targets, M2mAction::PreAdd).await;
    let existing = existing_targets(db, &m, owner_pk, targets).await?;
    let mut added = 0;
    for t in targets {
        if existing.contains(&format!("{t:?}")) {
            continue;
        }
        let sql = format!("INSERT INTO {} ({}, {}) VALUES (%s, %s)", vendor.quote(&m.table), vendor.quote(&m.owner_col), vendor.quote(&m.target_col));
        db.execute(&sql, &[prep_param(vendor, &m.owner_pk, owner_pk.clone()), prep_param(vendor, &m.target_pk, t.clone())]).await?;
        added += 1;
    }
    signal(label, field, owner_pk, targets, M2mAction::PostAdd).await;
    Ok(added)
}

async fn existing_targets(db: &dyn Executor, m: &M2m, owner_pk: &Value, targets: &[Value]) -> Result<std::collections::HashSet<String>> {
    if targets.is_empty() {
        return Ok(Default::default());
    }
    let vendor = db.vendor();
    let ph = vec!["%s"; targets.len()].join(", ");
    let sql = format!("SELECT {} FROM {} WHERE {} = %s AND {} IN ({ph})", vendor.quote(&m.target_col), vendor.quote(&m.table), vendor.quote(&m.owner_col), vendor.quote(&m.target_col));
    let mut params = vec![prep_param(vendor, &m.owner_pk, owner_pk.clone())];
    params.extend(targets.iter().map(|t| prep_param(vendor, &m.target_pk, t.clone())));
    Ok(db.query(&sql, &params).await?.into_iter().map(|r| format!("{:?}", r.values[0])).collect())
}

/// `instance.field.remove(*targets)`.
pub async fn remove(db: &dyn Executor, label: &str, field: &str, owner_pk: &Value, targets: &[Value]) -> Result<u64> {
    let m = resolve(label, field)?;
    let vendor = db.vendor();
    if targets.is_empty() {
        return Ok(0);
    }
    signal(label, field, owner_pk, targets, M2mAction::PreRemove).await;
    let ph = vec!["%s"; targets.len()].join(", ");
    let sql = format!("DELETE FROM {} WHERE {} = %s AND {} IN ({ph})", vendor.quote(&m.table), vendor.quote(&m.owner_col), vendor.quote(&m.target_col));
    let mut params = vec![prep_param(vendor, &m.owner_pk, owner_pk.clone())];
    params.extend(targets.iter().map(|t| prep_param(vendor, &m.target_pk, t.clone())));
    let n = db.execute(&sql, &params).await?;
    signal(label, field, owner_pk, targets, M2mAction::PostRemove).await;
    Ok(n)
}

/// `instance.field.clear()`.
pub async fn clear(db: &dyn Executor, label: &str, field: &str, owner_pk: &Value) -> Result<u64> {
    let m = resolve(label, field)?;
    let vendor = db.vendor();
    signal(label, field, owner_pk, &[], M2mAction::PreClear).await;
    let sql = format!("DELETE FROM {} WHERE {} = %s", vendor.quote(&m.table), vendor.quote(&m.owner_col));
    let n = db.execute(&sql, &[prep_param(vendor, &m.owner_pk, owner_pk.clone())]).await?;
    signal(label, field, owner_pk, &[], M2mAction::PostClear).await;
    Ok(n)
}

/// `instance.field.set(targets)`: make the relation exactly `targets`.
pub async fn set(db: &dyn Executor, label: &str, field: &str, owner_pk: &Value, targets: &[Value]) -> Result<()> {
    let m = resolve(label, field)?;
    let vendor = db.vendor();
    let sql = format!("SELECT {} FROM {} WHERE {} = %s", vendor.quote(&m.target_col), vendor.quote(&m.table), vendor.quote(&m.owner_col));
    let current: Vec<Value> = db
        .query(&sql, &[prep_param(vendor, &m.owner_pk, owner_pk.clone())])
        .await?
        .into_iter()
        .map(|r| r.values.into_iter().next().unwrap_or(Value::Null))
        .collect();
    let want: std::collections::HashSet<String> = targets.iter().map(|t| format!("{t:?}")).collect();
    let have: std::collections::HashSet<String> = current.iter().map(|t| format!("{t:?}")).collect();
    let to_remove: Vec<Value> = current.into_iter().filter(|c| !want.contains(&format!("{c:?}"))).collect();
    let to_add: Vec<Value> = targets.iter().filter(|t| !have.contains(&format!("{t:?}"))).cloned().collect();
    remove(db, label, field, owner_pk, &to_remove).await?;
    add(db, label, field, owner_pk, &to_add).await?;
    Ok(())
}

/// Primary keys currently related to `owner_pk` through the many-to-many `field` (`values_list("pk")`).
pub async fn current_ids(db: &dyn Executor, label: &str, field: &str, owner_pk: &Value) -> Result<Vec<Value>> {
    let m = resolve(label, field)?;
    let vendor = db.vendor();
    let sql = format!("SELECT {} FROM {} WHERE {} = %s ORDER BY {}", vendor.quote(&m.target_col), vendor.quote(&m.table), vendor.quote(&m.owner_col), vendor.quote(&m.target_col));
    let rows = db.query(&sql, &[prep_param(vendor, &m.owner_pk, owner_pk.clone())]).await?;
    Ok(rows.into_iter().map(|r| r.values.into_iter().next().unwrap_or(Value::Null)).collect())
}
