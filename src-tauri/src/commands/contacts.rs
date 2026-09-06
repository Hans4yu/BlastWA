use blastwa_core::campaign::checker::{check_numbers, CheckOutcome};
use blastwa_core::campaign::contact_list::ContactList;
use blastwa_core::campaign::import as csv_import;
use blastwa_core::message::variables::ContactRow;

use tauri::{Emitter, State};

use super::super::AppCtx;

#[tauri::command]
pub(crate) fn get_contacts(ctx: State<'_, AppCtx>) -> Result<Vec<serde_json::Value>, String> {
    let list = ctx.contacts.lock().unwrap();
    Ok(list
        .contacts
        .iter()
        .map(|c| {
            serde_json::json!({
                "number": c.number, "fullname": c.fullname,
                "var1": c.var1, "var2": c.var2, "var3": c.var3,
                "var4": c.var4, "var5": c.var5,
            })
        })
        .collect())
}

#[tauri::command]
pub(crate) fn clear_contacts(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    ctx.contacts.lock().unwrap().contacts.clear();
    Ok(serde_json::json!({ "ok": true }))
}

/// remove specific numbers from the send list — backs the contact table's
/// shift/ctrl-click "Delete Selected" flow
#[tauri::command]
pub(crate) fn remove_contacts(
    numbers: Vec<String>,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let kill: std::collections::HashSet<&String> = numbers.iter().collect();
    let mut list = ctx.contacts.lock().unwrap();
    let before = list.len();
    list.contacts.retain(|c| !kill.contains(&c.number));
    let removed = before - list.len();
    Ok(serde_json::json!({ "ok": true, "removed": removed }))
}

#[tauri::command]
pub(crate) fn import_contacts(
    path: String,
    remove_dupes: Option<bool>,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let p = std::path::Path::new(&path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mut list = match ext.as_str() {
        "txt" => ContactList::load_txt(p).map_err(|e| e.to_string())?,
        "csv" | "xlsx" | "xls" => {
            let headers = csv_import::read_table(p).map_err(|e| e.to_string())?.0;
            let mapping = csv_import::ColumnMapping::auto_suggest(&headers);
            csv_import::import_contacts(p, &mapping, true, remove_dupes.unwrap_or(true))
                .map_err(|e| e.to_string())?
                .1
        }
        other => {
            return Err(format!(
                "\"{path}\" is not a contact file — import accepts .txt, .csv, .xlsx or .xls"
            ))
        }
    };
    if remove_dupes.unwrap_or(true) {
        list.filter_duplicates();
    }
    let count = list.len();
    *ctx.contacts.lock().unwrap() = list;
    Ok(serde_json::json!({ "ok": true, "imported": count }))
}

#[tauri::command]
pub(crate) async fn check_numbers_cmd(
    account: String,
    ctx: State<'_, AppCtx>,
    app: tauri::AppHandle,
) -> Result<Vec<CheckOutcome>, String> {
    let injector = ctx.pipeline.get_injector(&account).await.map_err(|e| e.to_string())?;
    let numbers: Vec<String> = ctx
        .contacts
        .lock()
        .unwrap()
        .contacts
        .iter()
        .map(|c| c.number.clone())
        .collect();
    let outcomes = check_numbers(&injector, &numbers, |checked, tot, outcome| {
        // stream each result so the contacts page can render live
        let _ = app.emit(
            "check_progress",
            serde_json::json!({
                "checked": checked,
                "total": tot,
                "number": outcome.number,
                "exists": outcome.exists,
                "kind": outcome.kind,
            }),
        );
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(outcomes)
}

/// keep only the listed (checker-validated) numbers in the send list (U9)
#[tauri::command]
pub(crate) fn keep_contacts_only(valid_numbers: Vec<String>, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let mut list = ctx.contacts.lock().unwrap();
    list.contacts.retain(|c| valid_numbers.contains(&c.number));
    let kept = list.len();
    Ok(serde_json::json!({ "ok": true, "kept": kept }))
}

/// generate candidate numbers under a prefix range into the send list (U18).
/// output feeds the checker, never straight into a campaign blast.
/// `total_length` pads the range suffix with leading zeros so every
/// candidate lands on one exact digit count (11/12/13...): without it a
/// 0..99 range mixes 12- and 13-digit numbers because 0 renders as "0".
#[tauri::command]
pub(crate) fn add_generated_contacts(
    prefix: String,
    range_start: u64,
    range_end: u64,
    total_length: Option<u32>,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    const MAX_RANGE: u64 = 1000;
    let digits: String = prefix.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 6 {
        return Err("prefix must carry at least 6 digits".into());
    }
    if range_end < range_start {
        return Err("range end is below range start".into());
    }
    if range_end - range_start + 1 > MAX_RANGE {
        return Err(format!("range too large (max {} per batch)", MAX_RANGE));
    }
    let pad = match total_length {
        Some(t) => {
            let t = t as usize;
            if t <= digits.len() {
                return Err(format!(
                    "target length {} must be longer than the prefix ({} digits)",
                    t,
                    digits.len()
                ));
            }
            t - digits.len()
        }
        None => 0,
    };
    // a padded suffix must actually fit the requested range
    if pad > 0 {
        let max_suffix = 10u64.saturating_pow(pad as u32);
        if range_end >= max_suffix {
            return Err(format!(
                "range end {} does not fit in {} suffix digits (max {})",
                range_end, pad, max_suffix - 1
            ));
        }
    }
    let mut list = ctx.contacts.lock().unwrap();
    let mut added = 0usize;
    for n in range_start..=range_end {
        let num = if pad > 0 {
            format!("{}{:0width$}", digits, n, width = pad)
        } else {
            format!("{}{}", digits, n)
        };
        if list.contacts.iter().any(|c| c.number == num) {
            continue;
        }
        list.contacts.push(ContactRow::from_fullname(&num, ""));
        added += 1;
    }
    Ok(serde_json::json!({ "ok": true, "added": added }))
}

/// csv of only the checker rows that answered yes; the frontend filters,
/// this command re-filters anyway so the file contract cannot drift
#[tauri::command]
pub(crate) async fn export_valid_numbers(
    path: String,
    outcomes: Vec<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let mut wtr = csv::Writer::from_path(&path).map_err(|e| e.to_string())?;
    wtr.write_record(["Number", "Type"])
        .map_err(|e| e.to_string())?;
    let mut count = 0usize;
    for o in &outcomes {
        let exists = o.get("exists").and_then(|v| v.as_bool()).unwrap_or(false);
        if !exists {
            continue;
        }
        let number = o.get("number").and_then(|v| v.as_str()).unwrap_or("");
        if number.is_empty() {
            continue;
        }
        let kind = o.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        wtr.write_record([number, kind]).map_err(|e| e.to_string())?;
        count += 1;
    }
    wtr.flush().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "exported": count }))
}

/// csv of the current send list numbers — the number generator feeds this
/// list, so exporting it is how a generated batch leaves the app
#[tauri::command]
pub(crate) fn export_contacts_csv(path: String, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let list = ctx.contacts.lock().unwrap();
    let mut wtr = csv::Writer::from_path(&path).map_err(|e| e.to_string())?;
    wtr.write_record(["Number", "Full Name", "Var1", "Var2", "Var3", "Var4", "Var5"])
        .map_err(|e| e.to_string())?;
    for c in &list.contacts {
        wtr.write_record([
            &c.number, &c.fullname, &c.var1, &c.var2, &c.var3, &c.var4, &c.var5,
        ])
        .map_err(|e| e.to_string())?;
    }
    wtr.flush().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "exported": list.len() }))
}

/// U14: pull contacts saved in a whatsapp account's phonebook into the list
#[tauri::command]
pub(crate) async fn import_wa_contacts(
    account: String,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let injector = ctx
        .pipeline
        .get_injector_attached(&account)
        .await
        .map_err(|e| e.to_string())?;
    let wa_contacts = injector.list_wa_contacts().await.map_err(|e| e.to_string())?;
    let mut list = ctx.contacts.lock().unwrap();
    let mut added = 0usize;
    for (number, name) in wa_contacts {
        if number.is_empty() || list.contacts.iter().any(|c| c.number == number) {
            continue;
        }
        list.contacts.push(ContactRow::from_fullname(&number, &name));
        added += 1;
    }
    Ok(serde_json::json!({ "ok": true, "added": added }))
}

/// U16: products in an account's own whatsapp catalog
#[tauri::command]
pub(crate) async fn list_catalog_products(
    account: String,
    ctx: State<'_, AppCtx>,
) -> Result<Vec<serde_json::Value>, String> {
    let injector = ctx
        .pipeline
        .get_injector_attached(&account)
        .await
        .map_err(|e| e.to_string())?;
    let products = injector
        .get_catalog_products()
        .await
        .map_err(|e| e.to_string())?;
    Ok(products
        .into_iter()
        .map(|(id, name, description)| {
            serde_json::json!({ "id": id, "name": name, "description": description })
        })
        .collect())
}
