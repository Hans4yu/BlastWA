# CAMPAIGN ENGINE KNOWLEDGE BASE

## OVERVIEW
The `campaign` module orchestrates bulk messaging execution, anti-ban humanized timing delays, number validation, group participant extraction, and CSV/XLSX delivery report generation.

## STRUCTURE
```
src-tauri/src/campaign/
├── checker.rs         # WhatsApp number presence and validity verification
├── contact_list.rs    # In-memory contact management & batch selection
├── group_grabber.rs   # WhatsApp group scraping via CDP injection
├── human_behavior.rs  # Randomized natural delays & anti-ban typing simulation
├── import.rs          # Multi-format contact importer (CSV, XLSX, VCF, TXT)
├── log_exporter.rs    # CSV and XLSX export formats with color status
├── mod.rs             # Submodule exposure & campaign orchestration traits
├── pipeline.rs        # Concurrent campaign task queue and queue transitions
└── sender.rs          # Message dispatching, media attachment handling, and status logging
```

## WHERE TO LOOK
| Task | File | Notes |
|---|---|---|
| Message Dispatch Loop | `sender.rs` | Implements retry logic, cooldown triggers, and failure handling |
| Delay & Humanization | `human_behavior.rs` | Gaussian distribution jitter and randomized typing pauses |
| Parsing Contact Files | `import.rs` | Supports headers auto-detection, phone sanitization |
| Report Output | `log_exporter.rs` | Uses `rust_xlsxwriter` and `csv` crate |

## CONVENTIONS
- All contact numbers are sanitized to international E.164 without `+` or leading zeros before passing to CDP.
- Campaign aborts check `CancellationToken` before each individual contact dispatch.
- Every message attempt emits a progress event to the Tauri UI.

## ANTI-PATTERNS
- **NEVER** hardcode fixed sleep intervals in `sender.rs`; always route delays through `human_behavior.rs`.
- **DO NOT** write unbuffered log records during active blasts; use batch logging or persistent line-delimited records (`campaigns.jsonl`).
