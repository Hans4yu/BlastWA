use blastwa_core::campaign::group_grabber;

use tauri::State;

use super::super::AppCtx;

#[tauri::command]
pub(crate) async fn list_groups(
    account: String,
    ctx: State<'_, AppCtx>,
) -> Result<Vec<serde_json::Value>, String> {
    let injector = ctx.pipeline.get_injector_attached(&account).await.map_err(|e| e.to_string())?;
    let groups = group_grabber::list_groups(&injector)
        .await
        .map_err(|e| e.to_string())?;
    Ok(groups
        .into_iter()
        .map(|g| serde_json::json!({ "id": g.id, "name": g.name }))
        .collect())
}

#[tauri::command]
pub(crate) async fn grab_participants(
    account: String,
    group_id: String,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let injector = ctx.pipeline.get_injector_attached(&account).await.map_err(|e| e.to_string())?;
    let rows = group_grabber::grab_participants(&injector, &group_id)
        .await
        .map_err(|e| e.to_string())?;
    let count = rows.len();
    ctx.contacts.lock().unwrap().contacts.extend(rows);
    Ok(serde_json::json!({ "ok": true, "grabbed": count }))
}

#[tauri::command]
pub(crate) fn export_groups(
    path: String,
    groups: Vec<serde_json::Value>,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let _ = &ctx;
    let mut wtr = csv::Writer::from_path(&path).map_err(|e| e.to_string())?;
    wtr.write_record(["Group Name", "Group ID"])
        .map_err(|e| e.to_string())?;
    let mut count = 0usize;
    for g in &groups {
        let name = g.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let id = g.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        wtr.write_record([name, id]).map_err(|e| e.to_string())?;
        count += 1;
    }
    wtr.flush().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "exported": count }))
}

#[tauri::command]
pub(crate) async fn export_groups_xlsx(
    account: String,
    path: String,
    groups: Vec<serde_json::Value>,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    use rust_xlsxwriter::Workbook;

    let injector = ctx
        .pipeline
        .get_injector_attached(&account)
        .await
        .map_err(|e| e.to_string())?;

    let mut wb = Workbook::new();
    let mut used_sheet_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut exported_groups = 0usize;
    let mut exported_rows = 0usize;

    for g in &groups {
        let name = g.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let id = g.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        let parts = injector
            .get_group_participants(id)
            .await
            .map_err(|e| format!("{}: {e}", name))?;

        // excel sheet names: max 31 chars, no : \ / ? * [ ], must be unique
        let base: String = {
            let cleaned: String = name
                .chars()
                .map(|c| match c {
                    ':' | '\\' | '/' | '?' | '*' | '[' | ']' => ' ',
                    c => c,
                })
                .collect();
            let trimmed = cleaned.trim();
            if trimmed.is_empty() {
                "Group".to_string()
            } else {
                trimmed.chars().take(28).collect()
            }
        };
        let mut sheet_name = base.clone();
        let mut n = 2;
        while !used_sheet_names.insert(sheet_name.clone()) {
            sheet_name = format!("{} ({})", base, n);
            n += 1;
        }

        let sheet = wb.add_worksheet();
        sheet
            .set_name(&sheet_name)
            .map_err(|e| e.to_string())?;
        sheet.write(0, 0, "#").map_err(|e| e.to_string())?;
        sheet.write(0, 1, "Number").map_err(|e| e.to_string())?;
        sheet.write(0, 2, "Name").map_err(|e| e.to_string())?;
        for (i, (wa_id, cname)) in parts.iter().enumerate() {
            let number = blastwa_core::campaign::contact_list::normalize_number(
                wa_id.trim_end_matches("@c.us"),
            );
            if number.is_empty() {
                continue;
            }
            let r = (i + 1) as u32;
            sheet.write(r, 0, (i + 1) as u32).map_err(|e| e.to_string())?;
            sheet.write(r, 1, &number).map_err(|e| e.to_string())?;
            sheet.write(r, 2, cname.as_deref().unwrap_or("")).map_err(|e| e.to_string())?;
            exported_rows += 1;
        }
        exported_groups += 1;
    }

    wb.save(&path).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "groups": exported_groups, "rows": exported_rows }))
}
