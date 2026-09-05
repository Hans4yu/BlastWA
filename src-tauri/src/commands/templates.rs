use blastwa_core::message::spintax;
use blastwa_core::message::template_library::MessageTemplate;

use tauri::State;

use super::super::AppCtx;

#[tauri::command]
pub(crate) fn list_templates(ctx: State<'_, AppCtx>) -> Result<Vec<MessageTemplate>, String> {
    ctx.templates.list().map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn search_templates(query: String, ctx: State<'_, AppCtx>) -> Result<Vec<MessageTemplate>, String> {
    ctx.templates.search(&query).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn save_template(
    id: Option<String>,
    name: String,
    tags: Option<Vec<String>>,
    body: String,
    attachment_path: Option<String>,
    ctx: State<'_, AppCtx>,
) -> Result<MessageTemplate, String> {
    // an id means "edit this one": creating instead would leave the original
    // behind and duplicate the row
    if let Some(raw) = id.filter(|s| !s.is_empty()) {
        let uuid = uuid::Uuid::parse_str(&raw).map_err(|e| format!("bad template id: {e}"))?;
        let existing = ctx
            .templates
            .list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|t| t.id == uuid)
            .ok_or_else(|| format!("template {raw} not found"))?;
        let updated = MessageTemplate {
            name,
            tags: tags.unwrap_or_default(),
            body,
            attachment_path,
            ..existing
        };
        ctx.templates
            .update(updated.clone())
            .map_err(|e| e.to_string())?;
        return Ok(updated);
    }
    ctx.templates
        .create(name, tags.unwrap_or_default(), body, attachment_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn delete_template(id: uuid::Uuid, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    ctx.templates.delete(id).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
pub(crate) fn preview_spintax(
    text: String,
    ctx: State<'_, AppCtx>,
) -> Result<Vec<String>, String> {
    // render against the first contacts so [[firstname]] etc. show real
    // sample values; falls back to spintax-only when the list is empty
    let samples = ctx.contacts.lock().unwrap();
    if samples.contacts.is_empty() {
        return Ok(spintax::preview_spins(&text, 3));
    }
    let mut out = Vec::new();
    for c in samples.contacts.iter().take(3) {
        out.push(blastwa_core::message::variables::apply_variables(
            &spintax::spin(&text),
            c,
        ));
    }
    Ok(out)
}
