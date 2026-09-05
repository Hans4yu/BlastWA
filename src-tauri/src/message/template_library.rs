// template library (U15): reusable message templates with tags + attachment ref
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageTemplate {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub body: String,
    #[serde(default)]
    pub attachment_path: Option<String>,
    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
}

pub struct TemplateLibrary {
    path: PathBuf,
}

impl TemplateLibrary {
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("templates.json"),
        }
    }

    fn load_all(&self) -> Result<Vec<MessageTemplate>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        Ok(serde_json::from_str(&std::fs::read_to_string(&self.path)?)?)
    }

    fn save_all(&self, templates: &[MessageTemplate]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(crate::config::settings::atomic_write(
            &self.path,
            serde_json::to_string_pretty(templates)?.as_bytes(),
        )?)
    }

    pub fn create(
        &self,
        name: impl Into<String>,
        tags: Vec<String>,
        body: impl Into<String>,
        attachment_path: Option<String>,
    ) -> Result<MessageTemplate> {
        let now = Local::now();
        let t = MessageTemplate {
            id: Uuid::new_v4(),
            name: name.into(),
            tags,
            body: body.into(),
            attachment_path,
            created_at: now,
            updated_at: now,
        };
        let mut all = self.load_all()?;
        all.push(t.clone());
        self.save_all(&all)?;
        Ok(t)
    }

    pub fn list(&self) -> Result<Vec<MessageTemplate>> {
        self.load_all()
    }

    /// filter by tag or keyword in name
    pub fn search(&self, query: &str) -> Result<Vec<MessageTemplate>> {
        let q = query.to_lowercase();
        Ok(self
            .load_all()?
            .into_iter()
            .filter(|t| {
                t.name.to_lowercase().contains(&q)
                    || t.tags.iter().any(|tag| tag.to_lowercase().contains(&q))
            })
            .collect())
    }

    pub fn update(&self, updated: MessageTemplate) -> Result<()> {
        let mut all = self.load_all()?;
        if let Some(slot) = all.iter_mut().find(|t| t.id == updated.id) {
            *slot = updated;
            slot.updated_at = Local::now();
        }
        self.save_all(&all)
    }

    pub fn delete(&self, id: Uuid) -> Result<()> {
        let mut all = self.load_all()?;
        all.retain(|t| t.id != id);
        self.save_all(&all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> TemplateLibrary {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "blastwa_tpl_{}_{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TemplateLibrary::new(&dir)
    }

    #[test]
    fn create_list_search_delete_lifecycle() {
        let l = lib();
        l.create("Promo Ramadan", vec!["promo".into()], "{Hai|Hello} [[firstname]]", None).unwrap();
        l.create("Followup", vec!["sales".into()], "masih minat?", None).unwrap();

        assert_eq!(l.list().unwrap().len(), 2);

        let hits = l.search("promo").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "Promo Ramadan");

        let id = hits[0].id;
        l.delete(id).unwrap();
        assert_eq!(l.list().unwrap().len(), 1);
    }

    #[test]
    fn update_persists_body() {
        let l = lib();
        let t = l.create("A", vec![], "old", None).unwrap();
        let mut updated = t.clone();
        updated.body = "new".into();
        l.update(updated).unwrap();
        let back = l.list().unwrap();
        assert_eq!(back[0].body, "new");
    }

    #[test]
    fn spintax_saved_as_is() {
        let l = lib();
        let t = l.create("S", vec![], "{a|b} c", None).unwrap();
        assert_eq!(t.body, "{a|b} c");
    }
}
