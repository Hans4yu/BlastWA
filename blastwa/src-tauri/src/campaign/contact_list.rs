use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::message::variables::ContactRow;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContactList {
    pub contacts: Vec<ContactRow>,
}

/// normalize a raw phone string into bare digits with country code intent:
/// strips +, spaces, dashes, parens; keeps leading zeros out.
pub fn normalize_number(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    // drop a single leading zero (local trunk format)
    if let Some(stripped) = cleaned.strip_prefix('0') {
        return stripped.to_string();
    }
    cleaned
}

impl ContactList {
    pub fn load_txt(path: &Path) -> Result<Self> {
        let content = std::fs::read(path)
            .with_context(|| format!("reading {}", path.display()))?;
        // tolerate utf-16 BOM from windows notepad exports
        let text = decode_text(&content);
        let mut contacts = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split('|').map(|p| p.trim()).collect();
            let number = normalize_number(parts.first().copied().unwrap_or(""));
            if number.is_empty() {
                continue;
            }
            let fullname = parts.get(1).copied().unwrap_or("");
            let mut row = ContactRow::from_fullname(&number, fullname);
            for (i, slot) in [&mut row.var1, &mut row.var2, &mut row.var3, &mut row.var4, &mut row.var5]
                .iter_mut()
                .enumerate()
            {
                **slot = parts.get(2 + i).copied().unwrap_or("").to_string();
            }
            contacts.push(row);
        }
        Ok(ContactList { contacts })
    }

    pub fn save_json(&self, path: &Path) -> Result<()> {
        Ok(std::fs::write(path, serde_json::to_string_pretty(self)?)?)
    }

    pub fn load_json(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// dedupe by normalized number, keeping first occurrence
    pub fn filter_duplicates(&mut self) {
        let mut seen = HashSet::new();
        self.contacts.retain(|c| seen.insert(c.number.clone()));
    }

    pub fn len(&self) -> usize {
        self.contacts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.contacts.is_empty()
    }
}

fn decode_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (s, _, _) = encoding_rs::UTF_16LE.decode(&bytes[2..]);
        return s.into_owned();
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (s, _, _) = encoding_rs::UTF_16BE.decode(&bytes[2..]);
        return s.into_owned();
    }
    match String::from_utf8(bytes.to_vec()) {
        Ok(s) => s,
        Err(_) => {
            let (s, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            s.into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_formatting() {
        assert_eq!(normalize_number("+62 812-3456-7890"), "6281234567890");
        assert_eq!(normalize_number("(021) 555-1234"), "215551234");
        assert_eq!(normalize_number("08123456789"), "8123456789");
    }

    #[test]
    fn parse_pipe_columns() {
        let tmp = std::env::temp_dir().join("blastwa_test_contacts.txt");
        std::fs::write(&tmp, "628123|Budi Santoso|promo|x\n\n+628124|Ani|\n").unwrap();
        let list = ContactList::load_txt(&tmp).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list.contacts[0].fullname, "Budi Santoso");
        assert_eq!(list.contacts[0].var1, "promo");
        assert_eq!(list.contacts[0].firstname, "Budi");
        // second row has only number
        assert_eq!(list.contacts[1].fullname, "Ani");
        assert_eq!(list.contacts[1].var5, "");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn dedupe_keeps_first() {
        let mut list = ContactList {
            contacts: vec![
                ContactRow::from_fullname("111", "First"),
                ContactRow::from_fullname("222", "Second"),
                ContactRow::from_fullname("111", "Dup"),
            ],
        };
        list.filter_duplicates();
        assert_eq!(list.len(), 2);
        assert_eq!(list.contacts[0].fullname, "First");
    }
}
