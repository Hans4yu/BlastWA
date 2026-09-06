# BlastWA Handoff

**Updated:** 2026-09-06 (third pass — Auto Reply + Templates made real)
**Branch:** main
**Base commit:** 4f3bca3 — command modularization fully complete, clippy clean

## Current Objective

Complete the command-modularization refactor (Priority 2 of the 2026-09-05 handoff), restore the launch-result combiner regression, and leave the tree ready for headful manual smoke testing.

## Milestone Status

### Committed (776bcb1 + 186f642, 2026-09-06): P1 command modularization COMPLETE
- All IPC commands live under `src-tauri/src/commands/` in ten modules: `accounts` (incl. launch/discovery, probe, shortcut removal, combiner tests), `campaigns`, `contacts`, `groups`, `autoreply`, `templates`, `logs`, `config`, `updater`, `profiles`.
- `main.rs` reduced 1664 → 205 lines: header, `AppCtx`, `parse_cli_profile`, `main()` with `invoke_handler` registration. No `*_impl` back-references remain.
- `combine_launch_and_discovery` (deleted in abdb7a5, tests orphaned) restored inside `commands/accounts.rs` and wired into `launch_session`'s failure path; the bin tests compile and pass again.
- `AppError` deduplicated in `src-tauri/src/error.rs` (exported via `lib.rs`).

### This pass (2026-09-06, later): architecture-wide dead-logic audit + attachment picker fix
- **Full audit performed:** every registered IPC command vs frontend invocation (45 commands — all wired except `update_wpp`), every Rust pub helper in core modules (only `JsInjector::poll_new_messages` dead — WAPI-based, and WAPI doesn't exist under the wa-js v4 bundle), every config field (`wpp_last_check_at` never written/read; `DataPaths.reports` created a Reports dir nothing ever used), every page's ids/functions/buttons (all wired), and the whole REST API surface (4 routes, all functional). The IPC key convention was re-verified via the **Context7 MCP server** (`@upstash/context7-mcp`, driven over stdio): Tauri v2 expects **camelCase** JS keys for snake_case Rust params — snake_case only with `rename_all = "snake_case"`; mismatches deserialize as missing. All multi-word payloads across pages were cross-checked programmatically — clean.
- **Fixed — `update_wpp` was unreachable:** Settings could *check* for WPP updates but nothing could apply them. The WPP.js panel now shows a human status line (was: raw JSON dump of a field that doesn't exist), a "Download & Install Update" button when a newer wa-js exists, and `check_wpp_update`/`update_wpp` persist `wpp_last_check_at` (shown as "Last checked" on load). Verified live: check → status + persistence; update → bundle installed, version refreshed.
- **Fixed — attachment picker "No file chosen" (user-reported):** the desktop UI intercepted the native `<input type=file>` and routed to the Tauri dialog (real path got picked), but the input's built-in chrome kept saying "No file chosen" forever — `files` can't be populated programmatically. The visible control is now a real "Choose File…" button + filename + a **Remove** button (previously a picked attachment could never be cleared); the native input stays hidden purely as the browser-dev fallback. Draft save/restore goes through one `renderAttachment()`. Verified live: dialog stub → name renders, draft persists, survives tab switch, Remove clears everything.
- **Removed dead code:** `JsInjector::poll_new_messages` (WAPI-based, 0 call sites, superseded by the auto-reply inbox), `DataPaths.reports` + the Reports dir creation (nothing ever wrote there).
- `check_sending_page.js` now pins the picker contract (button ids exist, native input must stay `display:none`).

### Committed (4f3bca3, 2026-09-06): P2 tooling & cleanup
- **Settings UI:** Local API panel shows the active `api_token` with a Copy button, and renders curl samples with the real port and `X-BlastWA-Token` header.
- **Live test script:** `scripts/full_live_test.js` now reads `TEST_TARGET`, `TEST_PHONE`, `TEST_ASSETS_DIR` env vars (sensible defaults preserved) instead of hardcoding phone numbers and a user-specific path.
- **Clippy:** component installed; all 10 workspace warnings fixed (unnecessary casts in setup, slice over `&mut Vec` in `jitter_order`, `unwrap_or_default` in settings, char-array split in contact_list, `SessionList` type alias in server.rs, `is_multiple_of` in human_behavior, `#[allow(clippy::too_many_arguments)]` on `run_campaign`). `cargo clippy --workspace --all-targets` is warning-free.

### This pass (2026-09-06): Auto Reply made real + Templates bug fixes
- **Auto Reply watcher shipped** (`src-tauri/src/autoreply/watcher.rs`, spawned in `main.rs`): the page's "stored but not yet answering" era is over. Every 3s it snapshots the live-sessions registry, attaches to each running account (never launches Chrome), arms a WPP `chat.new_message` listener (event verified against wa-js docs: `ChatEventTypes 'chat.new_message' → MsgModel`), drains the in-page `window.__bw_inbox` buffer, matches against saved rules (first armed rule wins), and replies with seen → typing → 0.7–1.8s pause → send. Incoming only (`self === 'in'`), 1:1 chats only (groups + broadcasts skipped), per-account message-id dedupe (cap 1000), per-account 120s cycle timeout so one dead port never stalls the loop.
- **Rules lost on tab switch — FIXED**: the SPA rebuilds pages on every navigation, which used to wipe unsaved rows. `autoreply.html` now autosaves (debounced 800ms) on every edit and fires a final save in the router cleanup on navigation away. Verified live: rule edited → tab switch → back → row restored from disk.
- **Rule engine hardened** (`rules.rs`): keyword matching is now case-insensitive; `Rule::is_armed()` requires keyword AND reply; `save_rules` drops non-armed rows and returns the saved count; `match_rule` refuses empty keywords (empty Contains matches every message = answers the whole inbox). Legacy rule files without `reply_message` still match — the watcher just skips reply-less rules at send time.
- **New IPC:** `save_rules` returns `{ok, saved, skipped}`; `autoreply_status` exposes watcher telemetry (armed rules, watching accounts, replies sent, last reply epoch) rendered in the page's live status strip (5s refresh).
- **Templates — edit no longer wipes the attachment**: `save_template` replaces the whole record, and the editor used to send no attachment at all; it now stashes `tpl.attachment_path` on Edit and re-sends it on Save. **Plus the IPC key trap:** the payload key must be `attachmentPath` (camelCase) or Tauri silently deserializes Rust's `attachment_path` param as None — the same class of bug as the old `humanPreset`/`humanModePreset` mismatch.
- **Templates editor got a live preview** ("Preview 3 samples" → `preview_spintax` with real contact variables) and the editor shows the kept attachment while editing.
- **New regression harness:** `scripts/check_autoreply_page.js` (mini-DOM vm harness) pins autosave-on-edit, save-on-navigation-cleanup, incomplete-row skipping, PascalCase match-type wire format, restore-without-dirty, delete-persists, and the status strip rendering. `verify_router_lifecycle.js` REQUIRED list updated: autoreply now binds all buttons via addEventListener, so only `init_autoreply` is an inline-handler global.

## Verification (2026-09-06, third pass)

```
cargo test --lib                                      # 73 passed (incl. new autoreply tests)
cargo test --workspace                                # 73 lib + 3 integration passed
cargo test -p blastwa --features gui --bin blastwa    # 3 bin passed
cargo build --release -p blastwa --features gui       # passed, deployed to %LOCALAPPDATA%\Programs\BlastWA
node scripts/verify_router_lifecycle.js               # PASSED
node scripts/check_checker_cache.js                   # PASSED
node scripts/check_groups_cache.js                    # PASSED
node scripts/check_sending_page.js                    # PASSED
node scripts/check_autoreply_page.js                  # PASSED (new)
live CDP probe (temp script)                          # 11/11 PASSED: autosave, tab-switch
                                                      # survival, status telemetry, template
                                                      # attachment persistence, spintax preview
```

## Remaining Tasks

### P0: Headful manual smoke test (blocked on live phone, not automation)
`cargo tauri dev -- --features gui`, then: Add Account (double-click must not duplicate), Open Browser, scan QR, verify badge transitions to Online with number, Remove Selected / Remove All (directories + `.lnk` cleanup), Settings → Health Diagnostics refresh, Settings → API token visible and copyable. Optional E2E: `TEST_TARGET=628xxx TEST_PHONE=628yyy node scripts/full_live_test.js`.

### P2 leftover (deliberately skipped)
- `cargo fmt --all`: rustfmt was not previously installed and a whole-repo reformat would produce a large review-noise diff. Install (`rustup component add rustfmt`) and run it as a dedicated standalone commit if desired.

## Known Pitfalls (unchanged, still load-bearing)

- **Always** run the desktop app with `cargo tauri dev -- --features gui` (plain `cargo tauri dev` fails on missing feature).
- **Always** run `cargo test -p blastwa --features gui --bin blastwa` alongside `cargo test --workspace`; the latter never compiles bin tests, which is how the combiner regression hid.
- Windows locks `accounts/<name>` while Chrome is open — removal/rename surfaces a clear error instead.
- `scripts/verify_router_lifecycle.js:148` asserts the literal `/stampName, esc \}/` pattern in `src/main.js` — do not reformat the `window.blastwa = {...}` assignment.
- `setup` crate: never set `test = true` (Windows elevation error 740).

## Exact Next Move

**Auto Reply + Templates shipped (2026-09-06, this pass).** Auto Reply now answers: the watcher (`autoreply::watcher::run`, spawned at app start) watches every account with a running Chrome session, buffers incoming 1:1 texts via WPP `chat.new_message`, and fires the first matching armed rule (case-insensitive) with a humanized seen→typing→pause→send sequence. Rules autosave on every edit and on tab switch — the "switched tab and my rules vanished" bug is dead. Templates: editing no longer silently deletes a stored attachment (and remember the Tauri IPC rule: JS payloads send camelCase, Rust params are snake_case). The still-true "not verified against a live session" list: QR scan → watcher attaches → real incoming message → real auto-reply. To test: arm a rule (e.g. Contains "test" → "hi"), open an account, message it from another phone.

Everything else from previous passes (installer/uninstall wizard, no-UAC manifest, accounts fixes, checker UI, selection) unchanged — details below and in git log.

Deployed artifacts: `target/release/blastwa.exe` → `%LOCALAPPDATA%\Programs\BlastWA\blastwa.exe` (currently installed and running). `target/release/setup.exe` remains the installer/uninstaller.

Two elevated stray windows from pre-fix testing (uninstall-helper / old uninstall.exe) may still sit on the desktop — close them via their Cancel button (elevated processes cannot be killed from a non-admin shell).

Still needs a live phone: QR scan → Online badge transition, real message send, **auto-reply firing against a live session**.
