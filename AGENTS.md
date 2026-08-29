# PROJECT KNOWLEDGE BASE: BlastWA

**Generated:** 2026-08-29
**Repository:** BlastWA
**Tech Stack:** Rust (Tokio, Axum, Chromiumoxide, Serde) + Tauri v2 + Vanilla JS/HTML5/CSS (SPA router, Chrome CDP automation)

## OVERVIEW
BlastWA is a high-throughput, local WhatsApp marketing and bulk automation desktop application. It integrates native Chrome CDP (Chrome DevTools Protocol) automation, WPPConnect JS injection, and Axum loopback REST API with a lightweight, secure Tauri v2 frontend.

## REPOSITORY STRUCTURE
```
blastwa/
├── Cargo.toml                   # Root workspace manifest (edition 2021)
├── setup/                       # Profile and initial environment setup crate
├── src/                         # Frontend Web UI (Vanilla JS SPA)
│   ├── index.html               # Main window shell & persistent navigation
│   ├── main.js                  # SPA client router, epoch-guarded lifecycle, event listeners
│   ├── styles.css               # Desktop dark/light themes & layout
│   └── pages/                   # SPA view fragments loaded via route()
│       ├── dashboard.html       # Profile management, connection status, quick actions
│       ├── sending.html         # Campaign control, blast progress, real-time counters
│       ├── contacts.html        # Contact list management, validation, CSV/XLSX import
│       ├── groups.html          # WhatsApp group scraper & member extractor
│       ├── autoreply.html       # Automated keyword response rule engine
│       ├── templates.html       # Spintax & dynamic variable template editor
│       ├── log.html             # Historical delivery logs & exporter UI
│       └── settings.html        # Delay ranges, batch limits, and Chrome path config
└── src-tauri/                   # Rust backend & Core automation engine
    ├── Cargo.toml               # Core dependencies + optional GUI feature
    ├── tauri.conf.json          # Tauri v2 security, permissions, and window configuration
    └── src/
        ├── lib.rs               # Library entry point and IPC command registry
        ├── main.rs              # Application entry point & service coordinator
        ├── account/             # Multi-profile Chrome session storage & registry
        ├── api/                 # Local loopback Axum REST API (/api/blast, /api/status, etc.)
        ├── autoreply/           # Automated message triggers and rules evaluation
        ├── browser/             # Chrome detection, CDP WebSocket client, WPPConnect JS injector
        ├── campaign/            # Bulk sender, CSV/XLSX parser, humanized delays, log exporters
        ├── config/              # Persistent application settings (settings.json)
        ├── message/             # Spintax engine ({Hi|Hello}), template variables ({{name}})
        └── updater/             # Auto-updater for WPPConnect bundle assets
```

## WHERE TO LOOK
| Task / Feature | Location | Key Modules / Functions |
|---|---|---|
| IPC Command Bindings | `src-tauri/src/lib.rs` | `run()`, Tauri `invoke_handler` |
| Chrome CDP & Automation | `src-tauri/src/browser/` | `cdp_client.rs`, `js_injector.rs`, `chrome_detect.rs` |
| Campaign Execution Engine | `src-tauri/src/campaign/` | `sender.rs`, `pipeline.rs`, `human_behavior.rs` |
| Contact Import & Parsing | `src-tauri/src/campaign/` | `import.rs`, `contact_list.rs`, `checker.rs` |
| Export Campaign History | `src-tauri/src/campaign/` | `log_exporter.rs` (`export_csv`, `export_xlsx`) |
| Local REST API Server | `src-tauri/src/api/` | `server.rs` (`serve`, `AppState`, `blast`) |
| Spintax & Personalization | `src-tauri/src/message/` | `spintax.rs`, `variables.rs`, `template_library.rs` |
| Frontend Router & Lifecycle | `src/main.js` | `route()`, `listen()`, `runPageCleanups()` |

## CODE MAP (HIGH-SIGNAL SYMBOLS)

| Symbol | Type | Location | Role / Responsibility |
|---|---|---|---|
| `AppState` | Struct | `src-tauri/src/api/server.rs` | Atomic campaign state (running, paused, sent, failed, stop_flag) |
| `BlastRequest` | Struct | `src-tauri/src/api/server.rs` | DTO payload for external blast triggers |
| `CdpClient` | Struct | `src-tauri/src/browser/cdp_client.rs` | WebSocket connection to Chrome CDP endpoint |
| `JsInjector` | Struct | `src-tauri/src/browser/js_injector.rs` | Injects WPPConnect runtime & evaluates automation scripts |
| `LogEntry` | Struct | `src-tauri/src/campaign/log_exporter.rs` | Campaign result record with timestamp, status, and error |
| `route` | Async Fn | `src/main.js` | Fetches HTML fragment, injects scripts, manages navigation epoch |
| `runPageCleanups` | Function | `src/main.js` | Disposes previous page event listeners on navigation |
| `window.blastwa.esc` | Function | `src/main.js` | Shared HTML sanitization helper preventing XSS |

## CONVENTIONS
- **Network Security:** Local Axum server binds STRICTLY to loopback (`127.0.0.1` / `localhost`). Never bind `0.0.0.0`.
- **Decoupled Architecture:** `blastwa_core` compiles and tests in headless mode; Tauri GUI is gated behind the `gui` feature flag (`tauri = { version = "2", optional = true }`).
- **Frontend Script Execution:** SPA page fragments in `src/pages/*.html` are dynamically injected into `#content`. Scripts are executed via dynamic `<script>` creation to ensure clean scope and prevent double-initialization.
- **Navigation Safety:** All Tauri listeners in page scripts MUST use the epoch-aware `listen()` wrapper or register teardown callbacks via `addCleanup()`.
- **String Sanitization:** Pages MUST use `window.blastwa.esc()` and NEVER redeclare top-level `const esc` or `function esc`.

## ANTI-PATTERNS (THIS PROJECT)
- **DO NOT** use unescaped string concatenation when evaluating JavaScript via CDP; always use parameterized JSON serialization.
- **DO NOT** block Tokio worker threads with heavy synchronous file parsing; offload CSV/Excel operations via `tokio::task::spawn_blocking`.
- **NEVER** modify `AppState` atomics directly without coordinating through the cancellation token and pipeline channels.
- **NEVER** re-declare global variables with `const`/`let` in the root scope of `src/pages/*.html` scripts as classic script contexts persist across SPA route loads.

## BUILD & TEST COMMANDS
```bash
# Run unit & integration tests
cargo test

# Run frontend router lifecycle validation
node src/verify_router_lifecycle.js

# Build headless core binary
cargo build --package blastwa

# Run Tauri desktop app in dev mode
cargo tauri dev
```
