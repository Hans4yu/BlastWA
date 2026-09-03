# PROJECT KNOWLEDGE BASE: BlastWA

**Generated:** 2026-08-30
**Commit:** ece7063
**Branch:** main
**Tech Stack:** Rust (Tokio, Axum, Chromiumoxide, Serde) + Tauri v2 + Vanilla JS/HTML5/CSS (SPA router, Chrome CDP automation)

## OVERVIEW
BlastWA is a high-throughput, local WhatsApp bulk automation desktop application and installer suite. It integrates native Chrome CDP (Chrome DevTools Protocol) automation, adaptive WPPConnect runtime bootstrap, and an Axum loopback REST API with a secure Tauri v2 frontend and a Win32 GUI setup wizard (`setup.exe`).

## REPOSITORY STRUCTURE
```
blastwa/
├── Cargo.toml                   # Root workspace manifest (version 0.2.0, edition 2021)
├── setup/                       # Standalone Windows GUI installer wizard crate (setup.exe)
│   ├── Cargo.toml
│   └── src/main.rs              # Native Win32 wizard, folder picker, uninstaller & registry
├── scripts/                     # Automated UI capture, verification, and regression tests
│   ├── capture_ui.js            # Automated WebView2 UI screenshot capture runner
│   ├── full_live_test.js        # E2E live test runner driving UI & WhatsApp Web via CDP
│   └── verify_router_lifecycle.js
├── src/                         # Production frontend Web UI (Vanilla JS SPA)
│   ├── index.html               # Main window shell & profile launcher modal with shortcut checkbox
│   ├── main.js                  # SPA client router, epoch-guarded lifecycle, event listeners
│   ├── styles.css               # Desktop dark/light themes & layout
│   └── pages/                   # Modular SPA views (dashboard, sending, contacts, groups, etc.)
└── src-tauri/                   # Rust backend & Core automation engine
    ├── Cargo.toml               # Core dependencies + optional GUI feature
    ├── tauri.conf.json          # Tauri v2 window and bundle configuration
    ├── icons/                   # Modern multi-size PNGs and multi-layer icon.ico
    └── src/
        ├── lib.rs               # Headless blastwa_core library
        ├── main.rs              # Desktop GUI binary, 40+ IPC commands & desktop shortcut generator
        ├── account/             # Multi-profile Chrome session storage & registry
        ├── api/                 # Local loopback Axum REST API (/api/blast, /api/status, etc.)
        ├── autoreply/           # Automated keyword response rule engine
        ├── browser/             # Chrome detection, CDP WebSocket client, WPPConnect JS injector
        ├── campaign/            # Bulk sender, CSV/XLSX parser, humanized delays, log exporters
        ├── config/              # Persistent application settings (settings.json)
        ├── message/             # Spintax engine ({Hi|Hello}), template variables ([[name]])
        └── updater/             # Auto-updater for WPPConnect bundle assets
```

## WHERE TO LOOK
| Task / Feature | Location | Key Modules / Functions |
|---|---|---|
| Desktop GUI & IPC Handlers | `src-tauri/src/main.rs` | `fn main()`, Tauri `invoke_handler`, `open_profile_window` |
| Native GUI Installer Wizard | `setup/src/main.rs` | Win32 wizard, `pick_folder`, `register_windows_uninstaller` |
| Chrome CDP & Automation | `src-tauri/src/browser/` | `cdp_client.rs`, `js_injector.rs` (adaptive bootstrap, serde_json) |
| Campaign Execution Engine | `src-tauri/src/campaign/` | `sender.rs`, `pipeline.rs`, `human_behavior.rs` |
| Contact Import & Normalizer | `src-tauri/src/campaign/` | `import.rs` (XLSX date support), `contact_list.rs` (smart E.164) |
| Export Campaign History | `src-tauri/src/campaign/` | `log_exporter.rs` (`export_csv`, `export_xlsx` with status colors) |
| Local REST API Server | `src-tauri/src/api/` | `server.rs` (`serve`, `AppState`, `blast` wired to `pipeline.serve`) |
| Frontend Router & Lifecycle | `src/main.js` | `route()`, `listen()`, `runPageCleanups()`, `window.blastwa.esc()` |

## CODE MAP (HIGH-SIGNAL SYMBOLS)

| Symbol | Type | Location | Role / Responsibility |
|---|---|---|---|
| `AppState` | Struct | `src-tauri/src/api/server.rs` | Atomic campaign state (running, paused, sent, failed, stop_flag) |
| `Pipeline` | Struct | `src-tauri/src/campaign/pipeline.rs` | Session manager, CDP page pool, and `serve(rx)` REST worker |
| `JsInjector` | Struct | `src-tauri/src/browser/js_injector.rs` | Evaluates WPPConnect scripts using adaptive polling & serde_json |
| `normalize_number` | Function | `src-tauri/src/campaign/contact_list.rs` | Smart E.164 normalizer (converts `08xxx` to `628xxx`) |
| `create_desktop_profile_shortcut` | Function | `src-tauri/src/main.rs` | Generates `.lnk` shortcut with `--profile` flag on user Desktop |
| `LogEntry` | Struct | `src-tauri/src/campaign/log_exporter.rs` | Campaign record with timestamp, status, and error |
| `route` | Async Fn | `src/main.js` | Fetches HTML fragment, injects scripts, manages navigation epoch |
| `window.blastwa.esc` | Function | `src/main.js` | Shared HTML sanitization helper preventing XSS |

## CONVENTIONS
- **Network Security:** Local Axum server binds STRICTLY to loopback (`127.0.0.1` / `localhost`). Never bind `0.0.0.0`.
- **Decoupled Architecture:** `blastwa_core` compiles and tests in headless mode; Tauri GUI is gated behind the `gui` feature flag.
- **CDP Payload Safety:** Never format raw strings into JavaScript execution strings; always use `serde_json::to_string()`.
- **Frontend Script Execution:** SPA page fragments in `src/pages/*.html` are dynamically injected into `#content`. Never declare top-level `const`/`let` in page scripts.
- **Navigation Safety:** All Tauri listeners in page scripts MUST use the epoch-aware `listen()` wrapper or register teardown callbacks via `addCleanup()`.
- **Safe Directory Operations:** Never call `path.parent().unwrap()`; always check with `if let Some(parent) = path.parent()`.

## ANTI-PATTERNS (THIS PROJECT)
- **DO NOT** use unescaped string concatenation when evaluating JavaScript via CDP.
- **DO NOT** block Tokio worker threads with heavy synchronous file parsing; use `tokio::task::spawn_blocking`.
- **NEVER** modify `AppState` atomics directly without coordinating through the cancellation token and pipeline channels.
- **NEVER** re-declare global variables with `const`/`let` in the root scope of `src/pages/*.html` scripts.
- **NEVER** use hardcoded timeout ceilings for WPP.js bootstrap; use adaptive polling with active DOM readiness checks.

## COMMANDS
```bash
# Run unit & integration tests (headless core)
cargo test --lib

# Run full frontend router & modular cache verification tests
node scripts/verify_router_lifecycle.js
node scripts/check_checker_cache.js
node scripts/check_groups_cache.js
node scripts/check_sending_page.js

# Build release standalone GUI installer
cargo build --package blastwa-setup --release

# Build headless core binary
cargo build --package blastwa

# Run Tauri desktop app in dev mode
cargo tauri dev
```
