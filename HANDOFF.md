# BlastWA Handoff

**Updated:** 2026-09-02
**Branch:** main
**Base commit:** ece7063

## Current Objective

Fix the installed `setup.exe` flow where Open Browser returned `chrome cdp endpoint not found after launch` for a named profile.

## Confirmed Root Cause

`setup.exe` writes the detected Chrome path/version to the classic `APPDATA\\BlastWA\\config.json`. A named profile loads `APPDATA\\BlastWA\\profiles\\<name>\\config.json` instead. For `Hp_ABN__Ibu_`, that file was absent, so `chrome_path` was empty. Chrome never spawned, and `launch_session()` discarded the spawn error before reporting the generic CDP discovery failure.

## Implemented Fix

- `src-tauri/src/config/settings.rs`: active profiles inherit only missing `chrome_path` and `chrome_version` from the classic config; malformed/missing profile files are handled without rewriting profile data; explicit values and unrelated settings remain local.
- `src-tauri/src/main.rs`: launch/discovery result combination preserves the underlying Chrome spawn error.
- `src/pages/settings.html`: removed the obsolete `blastwa-init.exe` guidance.
- `scripts/verify_router_lifecycle.js`: added direct assertions for the current settings guidance.
- `docs/plans/2026-09-02-001-fix-profile-chrome-config-plan.md`: completed plan and optimization review.

## Verification

- `cargo test --lib`: 62 passed.
- `cargo test --features gui --bin blastwa`: 3 passed.
- `cargo check --all-targets --features gui`: passed; existing warnings remain in unrelated pre-existing code.
- Frontend lifecycle, checker cache, groups cache, and sending checks: passed.
- Release `blastwa.exe` build: passed after the final source changes.
- Release `blastwa-setup` build: passed; setup sources were unchanged by the browser fix.
- Playwright runtime QA: a fresh named profile displayed the real Chrome path/version, `open_browser` returned `{ ok: true, port: 9222 }`, and WhatsApp Web was present on the isolated Chrome endpoint. Test processes and temporary profile data were removed.
- `rustfmt` is unavailable in the local Rust toolchain.

## Working Tree

The existing uncommitted installer and test artifacts remain intentionally untouched: `AGENTS.md`, `Cargo.lock`, `setup/`, `scripts/capture_ui.js`, `scripts/full_live_test.js`, and `screenshots/`. The browser fix and this handoff/plan are also uncommitted.

## Next Action

Review the final diff and decide whether to commit the browser fix, plan, handoff, and the pre-existing installer/test changes separately. Do not commit until explicitly requested.
