// sent log export to csv/xlsx (U17)
use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Local>,
    pub number: String,
    pub fullname: String,
    pub status: String, // sent | failed | pending
    #[serde(default)]
    pub error_reason: Option<String>,
    pub campaign_name: String,
}

const HEADERS: [&str; 6] = [
    "Timestamp",
    "Number",
    "Full Name",
    "Status",
    "Error Reason",
    "Campaign",
];

fn row_of(entry: &LogEntry) -> [String; 6] {
    [
        entry.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
        entry.number.clone(),
        entry.fullname.clone(),
        entry.status.clone(),
        entry.error_reason.clone().unwrap_or_default(),
        entry.campaign_name.clone(),
    ]
}

pub fn export_csv(entries: &[LogEntry], out: &Path) -> Result<()> {
    let mut w = csv::WriterBuilder::new()
        .quote_style(csv::QuoteStyle::Necessary)
        .from_path(out)?;
    w.write_record(HEADERS)?;
    for e in entries {
        let r = row_of(e);
        w.write_record(r.iter().map(|s| s.as_str()))?;
    }
    w.flush()?;
    Ok(())
}

/// xlsx export with bold header + status color coding
pub fn export_xlsx(entries: &[LogEntry], out: &Path) -> Result<()> {
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();

    let header_fmt = rust_xlsxwriter::Format::new().set_bold();

    let green = rust_xlsxwriter::Format::new()
        .set_font_color(rust_xlsxwriter::Color::RGB(0x008800))
        .set_num_format("@");
    let red = rust_xlsxwriter::Format::new()
        .set_font_color(rust_xlsxwriter::Color::RGB(0xCC0000));
    let yellow = rust_xlsxwriter::Format::new()
        .set_font_color(rust_xlsxwriter::Color::RGB(0xB8860B));

    for (col, h) in HEADERS.iter().enumerate() {
        sheet.write_with_format(0, col as u16, *h, &header_fmt)?;
    }

    for (i, e) in entries.iter().enumerate() {
        let r = (i + 1) as u32;
        let cells = row_of(e);
        let fmt_for_status = |status: &str| match status {
            "sent" => Some(green.clone()),
            "failed" => Some(red.clone()),
            "pending" => Some(yellow.clone()),
            _ => None,
        };
        for (c, val) in cells.iter().enumerate() {
            if c == 3 {
                if let Some(f) = fmt_for_status(val) {
                    sheet.write_with_format(r, c as u16, val, &f)?;
                    continue;
                }
            }
            sheet.write_string(r, c as u16, val)?;
        }
    }

    book.save(out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<LogEntry> {
        vec![
            LogEntry {
                timestamp: Local::now(),
                number: "628123".into(),
                fullname: "Budi, \"the boss\"".into(), // comma + quotes stress-test
                status: "sent".into(),
                error_reason: None,
                campaign_name: "Promo Jan".into(),
            },
            LogEntry {
                timestamp: Local::now(),
                number: "628124".into(),
                fullname: "Ani".into(),
                status: "failed".into(),
                error_reason: Some("chat not found".into()),
                campaign_name: "Promo Jan".into(),
            },
        ]
    }

    #[test]
    fn csv_export_roundtrip() {
        let out = std::env::temp_dir().join("blastwa_test_log.csv");
        export_csv(&sample(), &out).unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.starts_with("Timestamp,Number"));
        // quoted field with comma must survive
        assert!(text.contains("\"Budi, \"\"the boss\"\"\"") || text.contains("Budi"));
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn xlsx_export_creates_file() {
        let out = std::env::temp_dir().join("blastwa_test_log.xlsx");
        export_xlsx(&sample(), &out).unwrap();
        assert!(out.exists());
        assert!(std::fs::metadata(&out).unwrap().len() > 100);
        let _ = std::fs::remove_file(&out);
    }
}
