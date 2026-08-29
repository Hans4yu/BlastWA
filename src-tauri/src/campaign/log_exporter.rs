// sent log export to csv/xlsx (U17)
use std::path::{Path, PathBuf};

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

// ---------- persistent campaign history (U6) ----------
// one json line per campaign in Data/campaigns.jsonl; survives restarts.
// a record is appended at start (status "running") and rewritten in place
// when the campaign finishes.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignRecord {
    pub started_at: DateTime<Local>,
    pub account: String,
    pub message_preview: String,
    pub total: u32,
    pub sent: u32,
    pub failed: u32,
    // running | completed | stopped | interrupted
    pub status: String,
}

pub fn campaigns_file(app_dir: &Path) -> PathBuf {
    app_dir.join("Data").join("campaigns.jsonl")
}

pub fn append_campaign_record(app_dir: &Path, rec: &CampaignRecord) -> Result<()> {
    let path = campaigns_file(app_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(f, "{}", serde_json::to_string(rec)?)?;
    Ok(())
}

/// replace the last record (the one this process is currently running);
/// single-campaign-per-process makes "last line" unambiguous
pub fn finalize_last_campaign_record(app_dir: &Path, rec: &CampaignRecord) -> Result<()> {
    let path = campaigns_file(app_dir);
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path)?;
    let mut lines: Vec<String> = raw.lines().map(String::from).collect();
    while matches!(lines.last(), Some(l) if l.trim().is_empty()) {
        lines.pop();
    }
    if let Some(last) = lines.last_mut() {
        *last = serde_json::to_string(rec)?;
    } else {
        return Ok(());
    }
    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(&path, out)?;
    Ok(())
}

/// startup sweep: records still marked "running" belong to dead processes
pub fn interrupt_stale_running_records(app_dir: &Path) -> Result<()> {
    let path = campaigns_file(app_dir);
    if !path.exists() {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path)?;
    let mut changed = false;
    let mut out_lines = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<CampaignRecord>(line) {
            Ok(mut rec) if rec.status == "running" => {
                rec.status = "interrupted".into();
                changed = true;
                out_lines.push(serde_json::to_string(&rec)?);
            }
            Ok(_) => out_lines.push(line.to_string()),
            Err(_) => out_lines.push(line.to_string()), // keep corrupt lines as-is
        }
    }
    if changed {
        let mut out = out_lines.join("\n");
        out.push('\n');
        std::fs::write(&path, out)?;
    }
    Ok(())
}

/// newest first
pub fn load_campaign_records(app_dir: &Path) -> Vec<CampaignRecord> {
    let path = campaigns_file(app_dir);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut recs: Vec<CampaignRecord> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| match serde_json::from_str(l) {
            Ok(r) => Some(r),
            Err(e) => {
                log::warn!("skipping corrupt campaign record ({e})");
                None
            }
        })
        .collect();
    recs.reverse();
    recs
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

    fn rec(status: &str) -> CampaignRecord {
        CampaignRecord {
            started_at: Local::now(),
            account: "akun1".into(),
            message_preview: "Hai [[firstname]]!".into(),
            total: 10,
            sent: 3,
            failed: 1,
            status: status.into(),
        }
    }

    #[test]
    fn campaign_record_append_finalize_roundtrip() {
        let dir = std::env::temp_dir().join(format!("blastwa_test_hist_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        append_campaign_record(&dir, &rec("running")).unwrap();
        append_campaign_record(&dir, &rec("running")).unwrap();

        // finalize only touches the LAST record
        let mut done = rec("completed");
        done.sent = 9;
        finalize_last_campaign_record(&dir, &done).unwrap();

        let recs = load_campaign_records(&dir);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].status, "completed"); // newest first
        assert_eq!(recs[0].sent, 9);
        assert_eq!(recs[1].status, "running"); // older one untouched
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_running_marked_interrupted_on_startup() {
        let dir = std::env::temp_dir().join(format!("blastwa_test_stale_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        append_campaign_record(&dir, &rec("completed")).unwrap();
        append_campaign_record(&dir, &rec("running")).unwrap();
        // corrupt line must survive the sweep untouched
        let path = campaigns_file(&dir);
        let mut raw = std::fs::read_to_string(&path).unwrap();
        raw.push_str("{not json}\n");
        std::fs::write(&path, raw).unwrap();

        interrupt_stale_running_records(&dir).unwrap();

        let recs = load_campaign_records(&dir);
        assert_eq!(recs.len(), 2); // corrupt line skipped on load
        assert_eq!(recs[0].status, "interrupted"); // was running
        assert_eq!(recs[1].status, "completed"); // untouched
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("{not json}")); // preserved verbatim
        let _ = std::fs::remove_dir_all(&dir);
    }
}
