// csv/xlsx import with auto column mapping (U16)
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::campaign::contact_list::{normalize_number, ContactList};
use crate::message::variables::ContactRow;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColumnMapping {
    pub number_col: Option<String>,
    pub fullname_col: Option<String>,
    pub var1_col: Option<String>,
    pub var2_col: Option<String>,
    pub var3_col: Option<String>,
    pub var4_col: Option<String>,
    pub var5_col: Option<String>,
}

impl ColumnMapping {
    /// fuzzy auto-suggest from header row: "phone", "nomor hp", "name" etc.
    pub fn auto_suggest(headers: &[String]) -> Self {
        let find = |needles: &[&str]| -> Option<String> {
            for h in headers {
                let hl = h.to_lowercase();
                for n in needles {
                    if hl.contains(n) {
                        return Some(h.clone());
                    }
                }
            }
            None
        };
        ColumnMapping {
            number_col: find(&["phone", "number", "nomor", "no hp", "hp", "wa"]),
            fullname_col: find(&["name", "nama"]),
            var1_col: find(&["var1", "kolom1"]),
            var2_col: find(&["var2", "kolom2"]),
            var3_col: find(&["var3", "kolom3"]),
            var4_col: find(&["var4", "kolom4"]),
            var5_col: find(&["var5", "kolom5"]),
        }
    }

    fn map_row(&self, headers: &[String], row: &[String]) -> Option<ContactRow> {
        let get = |col: &Option<String>| -> String {
            col.as_ref()
                .and_then(|c| headers.iter().position(|h| h == c))
                .and_then(|i| row.get(i).cloned())
                .unwrap_or_default()
        };
        let raw_number = get(&self.number_col);
        let number = normalize_number(&raw_number);
        if number.is_empty() {
            return None;
        }
        Some(ContactRow {
            number,
            fullname: get(&self.fullname_col),
            firstname: String::new(),
            middlename: String::new(),
            lastname: String::new(),
            var1: get(&self.var1_col),
            var2: get(&self.var2_col),
            var3: get(&self.var3_col),
            var4: get(&self.var4_col),
            var5: get(&self.var5_col),
        })
    }
}

// name splitting lives in ContactRow::from_fullname so importers and the
// txt loader cannot drift apart on what a middle name is
fn split_names(row: ContactRow) -> ContactRow {
    let split = ContactRow::from_fullname(&row.number, &row.fullname);
    ContactRow {
        firstname: split.firstname,
        middlename: split.middlename,
        lastname: split.lastname,
        ..row
    }
}

/// sniff whether file is csv or xlsx by extension and read accordingly.
/// returns (headers, rows).
pub fn read_table(path: &Path) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "csv" | "txt" => read_csv(path),
        "xlsx" | "xls" => read_xlsx(path),
        other => Err(anyhow::anyhow!("unsupported format: {other}")),
    }
}

fn read_csv(path: &Path) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    // excel csv exports are often windows-1252; try utf-8 first
    let text = match String::from_utf8(bytes.clone()) {
        Ok(s) => s,
        Err(_) => {
            let (s, _, _) = encoding_rs::WINDOWS_1252.decode(&bytes);
            s.into_owned()
        }
    };
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .has_headers(false)
        .from_reader(text.as_bytes());

    let mut rows: Vec<Vec<String>> = Vec::new();
    for rec in rdr.records() {
        let rec = rec.context("csv record parse")?;
        rows.push(rec.iter().map(|s| s.to_string()).collect());
    }
    if rows.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let headers = rows.remove(0);
    Ok((headers, rows))
}

fn read_xlsx(path: &Path) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    use calamine::Reader;
    let mut book =
        calamine::open_workbook_auto(path).with_context(|| format!("opening {}", path.display()))?;
    let range = book
        .worksheet_range_at(0)
        .ok_or_else(|| anyhow::anyhow!("empty workbook"))?
        .map_err(|e| anyhow::anyhow!("xlsx read error: {e}"))?;

    let mut all: Vec<Vec<String>> = Vec::new();
    for row in range.rows() {
        all.push(
            row.iter()
                .map(|cell| match cell {
                    calamine::Data::Float(f) => {
                        // avoid "81234567890.0" artifacts from numeric cells
                        if f.fract() == 0.0 && (0.0..1e15).contains(f) {
                            format!("{}", *f as u64)
                        } else {
                            format!("{f}")
                        }
                    }
                    calamine::Data::Int(i) => format!("{i}"),
                    calamine::Data::String(s) => s.clone(),
                    calamine::Data::Bool(b) => format!("{b}"),
                    calamine::Data::DateTime(dt) => format!("{dt}"),
                    calamine::Data::DateTimeIso(s) => s.clone(),
                    _ => String::new(),
                })
                .collect(),
        );
    }
    if all.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let headers = all.remove(0);
    Ok((headers, all))
}

/// full import pipeline: read table, apply mapping, build contact list.
/// `first_row_is_header=false` means synthetic Col1..ColN headers.
pub fn import_contacts(
    path: &Path,
    mapping: &ColumnMapping,
    first_row_is_header: bool,
    remove_dupes: bool,
) -> Result<(Vec<String>, ContactList)> {
    let (mut headers, mut rows) = read_table(path)?;
    if !first_row_is_header {
        let width = rows.first().map(|r| r.len()).unwrap_or(1);
        headers = (1..=width).map(|i| format!("Col{i}")).collect();
    } else if headers.is_empty() {
        rows.clear();
    }

    let mut list = ContactList::default();
    for row in &rows {
        if let Some(mut c) = mapping.map_row(&headers, row) {
            c = split_names(c);
            list.contacts.push(c);
        }
    }
    if remove_dupes {
        list.filter_duplicates();
    }
    Ok((headers, list))
}

/// preview first n parsed rows for the UI confirm dialog
pub fn preview_rows(
    path: &Path,
    mapping: &ColumnMapping,
    first_row_is_header: bool,
    n: usize,
) -> Result<Vec<ContactRow>> {
    let (_, list) = import_contacts(path, mapping, first_row_is_header, false)?;
    Ok(list.contacts.into_iter().take(n).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_suggest_finds_common_headers() {
        let headers: Vec<String> = ["Nomor HP", "Nama Lengkap", "Kota"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let m = ColumnMapping::auto_suggest(&headers);
        assert_eq!(m.number_col.as_deref(), Some("Nomor HP"));
        assert_eq!(m.fullname_col.as_deref(), Some("Nama Lengkap"));
    }

    #[test]
    fn csv_import_end_to_end() {
        let tmp = std::env::temp_dir().join("blastwa_test.csv");
        std::fs::write(
            &tmp,
            "phone,name,var1\n+62 812-3456-7890,Budi Santoso,promo\n08123456789,Ani,\n",
        )
        .unwrap();

        let headers: Vec<String> = vec!["phone".into(), "name".into(), "var1".into()];
        let mapping = ColumnMapping {
            number_col: Some(headers[0].clone()),
            fullname_col: Some(headers[1].clone()),
            var1_col: Some(headers[2].clone()),
            ..Default::default()
        };

        let (_, list) = import_contacts(&tmp, &mapping, true, true).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list.contacts[0].number, "6281234567890");
        assert_eq!(list.contacts[0].firstname, "Budi");
        assert_eq!(list.contacts[0].lastname, "Santoso");
        assert_eq!(list.contacts[1].var1, "");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn missing_number_column_skips_row() {
        let m = ColumnMapping::default(); // no mapping at all
        let row = vec!["x".to_string()];
        assert!(m.map_row(&["a".into()], &row).is_none());
    }
}
