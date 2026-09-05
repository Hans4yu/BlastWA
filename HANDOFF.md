# BlastWA Handoff

**Updated:** 2026-09-06 (second pass)
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

### Committed (4f3bca3, 2026-09-06): P2 tooling & cleanup
- **Settings UI:** Local API panel shows the active `api_token` with a Copy button, and renders curl samples with the real port and `X-BlastWA-Token` header.
- **Live test script:** `scripts/full_live_test.js` now reads `TEST_TARGET`, `TEST_PHONE`, `TEST_ASSETS_DIR` env vars (sensible defaults preserved) instead of hardcoding phone numbers and a user-specific path.
- **Clippy:** component installed; all 10 workspace warnings fixed (unnecessary casts in setup, slice over `&mut Vec` in `jitter_order`, `unwrap_or_default` in settings, char-array split in contact_list, `SessionList` type alias in server.rs, `is_multiple_of` in human_behavior, `#[allow(clippy::too_many_arguments)]` on `run_campaign`). `cargo clippy --workspace --all-targets` is warning-free.

## Verification (2026-09-06, second pass)

```
cargo check -p blastwa --features gui --all-targets   # 0 errors, 0 warnings
cargo clippy --workspace --all-targets                # 0 warnings
cargo test --workspace                                # 70 lib + 3 integration passed
cargo test -p blastwa --features gui                  # 70 lib + 3 integration + 3 bin passed
cargo build --package blastwa-setup --release         # passed
node scripts/verify_router_lifecycle.js               # PASSED
node scripts/check_checker_cache.js                   # PASSED
node scripts/check_groups_cache.js                    # PASSED
node scripts/check_sending_page.js                    # PASSED
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

Run the headful smoke test above and record the results in this file. There is no pending refactor work: the modularization milestone is complete and the tree is clean at 4f3bca3.
