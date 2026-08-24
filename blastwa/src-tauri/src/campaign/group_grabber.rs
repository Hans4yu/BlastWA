// group grabber: list joined groups + pull participant lists into contacts
use crate::browser::js_injector::JsInjector;
use crate::campaign::contact_list::normalize_number;
use crate::message::variables::ContactRow;

#[derive(Debug, Clone, serde::Serialize)]
pub struct WaGroup {
    pub id: String,
    pub name: String,
}

pub async fn list_groups(injector: &JsInjector) -> anyhow::Result<Vec<WaGroup>> {
    let raw = injector.get_all_groups().await?;
    Ok(raw.into_iter().map(|(id, name)| WaGroup { id, name }).collect())
}

/// pull participants of one group as ContactRows ready for campaigns.
/// wa ids look like 628123...@c.us — strip the suffix back to bare digits.
pub async fn grab_participants(
    injector: &JsInjector,
    group_id: &str,
) -> anyhow::Result<Vec<ContactRow>> {
    let ids = injector.get_group_participants(group_id).await?;
    let mut rows = Vec::with_capacity(ids.len());
    for wa_id in ids {
        let number = normalize_number(wa_id.trim_end_matches("@c.us"));
        if number.is_empty() || wa_id.ends_with("@g.us") {
            continue; // skip sub-groups inside groups
        }
        rows.push(ContactRow::from_fullname(&number, ""));
    }
    Ok(rows)
}
