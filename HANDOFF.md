# BlastWA Handoff

**Updated:** 2026-09-06
**Branch:** main
**Base commit:** b148415 (Priorities 2-7 committed) + P1 command modularization complete in working tree

## Current Objective

Complete the command-modularization refactor (Priority 2 of the 2026-09-05 handoff), restore the launch-result combiner regression, and leave the tree ready for headful manual smoke testing.

## Milestone Status

### Committed (b148415, 2026-09-06 00:09 +07)
Priorities 2-7 as described in the 2026-09-05 handoff: security hardening (REST token auth + CSP), persistence integrity (atomic writes, schema envelopes, cross-process `.lock` files, corrupt-file backups), multi-profile process registry, structured `AppError` IPC errors, and health diagnostics.

### Completed in this pass (P1: Command Modularization — uncommitted until now)
- Extracted all remaining non-account IPC commands out of `src-tauri/src/main.rs` into dedicated modules under `src-tauri/src/commands/`:
  - `campaigns.rs` — start_campaign (multi-channel, scheduling, list/catalog payloads), pause/resume/stop, get_status
  - `contacts.rs` — get/clear/import contacts, checker, keep-only, generated ranges, exports, WA phonebook + catalog pulls
  - `groups.rs` — list_groups, grab_participants, CSV/XLSX exports
  - `autoreply.rs` — load/save rules
  - `templates.rs` — template CRUD + spintax preview
  - `logs.rs` — get_logs, export_log, campaign history
  - `config.rs` — get/save config, get_health_diagnostics
  - `updater.rs` — WPP bundle version check/update
  - `profiles.rs` — open_profile_window, list_profiles, desktop shortcut creation
- `main.rs` slimmed from 1664 to 662 lines: AppCtx, account section (impls + launch/probe), `remove_desktop_profile_shortcut`, CLI profile parse, `main()`, unit tests. `invoke_handler` now registers `commands::<module>::<fn>` paths.
- `AppError` deduplicated: single definition in `src-tauri/src/error.rs` (exported via `lib.rs`); `commands/accounts.rs` imports it.
- **Regression fixed:** `combine_launch_and_discovery` was deleted by abdb7a5 while its test module survived — a compile break invisible to `cargo test --workspace` because the bin target is gated behind `required-features = ["gui"]`. Function restored in `main.rs` and wired into `launch_session`'s failure path; the 3 bin tests compile and pass again.

### Known remaining delegation
Account commands (`commands/accounts.rs`) still delegate to `*_impl` functions in `main.rs`. This is intentional for now: the account section owns `launch_session`/`probe_account_state` which are entangled with AppCtx caches (`auth_cache`, `wpp_bootstrapped`). Extracting them needs a shared context trait/struct and is deferred.

## Verification (this pass)

```
cargo check -p blastwa --features gui --all-targets   # 0 errors, 0 warnings
cargo test --workspace                                # 70 lib + 3 integration passed
cargo test -p blastwa --features gui --bin blastwa    # 3 passed (combiner tests)
node scripts/verify_router_lifecycle.js               # PASSED
node scripts/check_checker_cache.js                   # PASSED
node scripts/check_groups_cache.js                    # PASSED
node scripts/check_sending_page.js                    # PASSED
```

## Remaining Tasks

### P0: Headful manual smoke test (blocked on live phone, not automation)
`cargo tauri dev -- --features gui`, then: Add Account (double-click must not duplicate), Open Browser, scan QR, verify badge transitions to Online with number, Remove Selected / Remove All (directories + `.lnk` cleanup), Settings → Health Diagnostics refresh.

### P1 follow-up (optional): extract account section
Move `launch_session`, `probe_account_state`, and the `*_impl` account functions into `commands/accounts.rs` or an `account` service layer, removing the `super::super::*_impl` back-references.

### P2: Optional
- `rustup component add clippy` + resolve warnings
- `rustup component add rustfmt` + `cargo fmt --all`
- Expose/copy `api_token` in `src/pages/settings.html`
- Update `scripts/full_live_test.js` to read `TEST_PHONE` / `TEST_TARGET` env vars

## Known Pitfalls (unchanged, still load-bearing)

- **Always** run the desktop app with `cargo tauri dev -- --features gui` (plain `cargo tauri dev` fails on missing feature).
- **Always** run `cargo test -p blastwa --features gui --bin blastwa` alongside `cargo test --workspace`; the latter never compiles bin tests, which is how the combiner regression hid.
- Windows locks `accounts/<name>` while Chrome is open — removal/rename surfaces a clear error instead.
- `scripts/verify_router_lifecycle.js:148` asserts the literal `/stampName, esc \}/` pattern in `src/main.js` — do not reformat the `window.blastwa = {...}` assignment.
- `setup` crate: never set `test = true` (Windows elevation error 740).

## Exact Next Move

1. Commit this tree: `git add src-tauri/ src/ && git commit -m "refactor(commands): complete command modularization (priority 2)"`
2. Run the headful smoke test above; record results in this file.
