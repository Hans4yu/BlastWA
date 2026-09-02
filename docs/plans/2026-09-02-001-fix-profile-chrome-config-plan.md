---
title: Fix Profile-Scoped Chrome Configuration for Installed Launches
type: fix
status: completed
date: 2026-09-02
---

# Fix Profile-Scoped Chrome Configuration for Installed Launches

## Overview

Make an installed BlastWA profile inherit the installer-detected Chrome runtime when that profile has no explicit Chrome configuration. Preserve profile isolation for all other settings and retain the underlying Chrome startup error when CDP discovery fails.

---

## Problem Frame

The installed app's Settings page shows `chrome_path` as `(run blastwa-init.exe)` and an empty version for profile `Hp_ABN__Ibu_`. Clicking Open Browser returns `chrome cdp endpoint not found after launch`.

Confirmed on this checkout: the classic runtime config contains a valid Chrome path, while `BlastWA/profiles/Hp_ABN__Ibu_/config.json` is absent. `main()` loads the profile-scoped config before `open_browser`; the empty path causes Chrome spawn to fail, but `launch_session` drops that failure and reports only the later discovery miss.

---

## Requirements Trace

- R1. A profile created or launched after setup must be able to use the Chrome runtime detected by setup.exe.
- R2. Explicit per-profile Chrome settings must remain authoritative.
- R3. Open Browser failures must identify the underlying Chrome spawn problem instead of masking it as only a CDP discovery failure.
- R4. Existing profile isolation, Chrome session isolation, and frontend lifecycle behavior must remain unchanged.

---

## Scope Boundaries

- Do not redesign the Chrome singleton or CDP port allocation unless installed-style verification proves an independent failure.
- Do not merge the entire classic config into profiles; delays, API settings, and profile metadata remain profile-local.
- Do not rewrite the installer UI or alter the existing uncommitted setup wizard beyond the stale setup executable label if needed.
- Do not modify unrelated existing changes in `AGENTS.md`, `Cargo.lock`, `scripts/`, or `screenshots/`.

---

## Context & Research

### Relevant Code and Patterns

- `src-tauri/src/config/settings.rs`: `AppConfig::app_dir()` adds `profiles/<sanitized-name>` when a launcher profile is active; `load_or_default()` currently falls straight back to defaults.
- `src-tauri/src/main.rs`: `main()` activates the profile before config load; `launch_session()` passes `cfg.chrome_path` to the CDP session manager and currently converts launch errors to `None`.
- `src-tauri/src/browser/cdp_client.rs`: `SessionManager::launch()` uses `Command::new(chrome_path)`, isolated `--user-data-dir`, and a requested CDP port.
- `setup/src/main.rs`: `perform_installation()` writes the detected Chrome fields to the classic `APPDATA\\BlastWA\\config.json`.
- `src/pages/settings.html`: the placeholder still names the removed `blastwa-init.exe` bootstrapper.

### External References

- Chromium user data directory behavior: `https://chromium.googlesource.com/chromium/src/+/main/docs/user_data_dir.md`
- Chrome DevTools Protocol startup guidance: `https://chromedevtools.github.io/devtools-protocol/`
- Rust Windows process spawning behavior: `https://doc.rust-lang.org/std/process/struct.Command.html`

External research supports keeping the isolated user-data directory and checking the actual executable path before diagnosing singleton or fixed-port behavior.

---

## Key Technical Decisions

- **Use narrow runtime inheritance:** fill only missing `chrome_path` and `chrome_version` from the classic config. This fixes profiles created after installation without leaking mutable campaign/API settings.
- **Preserve explicit profile values:** a non-empty profile Chrome path or version is never overwritten by the classic config, even if the explicit path is invalid.
- **Retain launch context:** keep the `SessionManager::launch()` error and include it when endpoint discovery also fails, so an empty or invalid executable path is actionable.
- **Do not add a second Chrome detector in the GUI:** setup already writes the classic configuration and the runtime should consume that shared baseline.

---

## Open Questions

### Resolved During Planning

- **Should setup's classic Chrome config be available to profiles?** Yes. Profiles launched after setup cannot otherwise receive the only detected Chrome path, and the user-visible failure confirms that gap.
- **Is the CDP singleton behavior the primary failure?** No. The active profile has no config file and therefore no executable path; test singleton behavior only after this is corrected.

### Deferred to Implementation

- **Exact helper name and placement:** choose the smallest pure merge helper that can be tested without mutating the process-global `OnceLock` profile state.
- **Installed binary packaging completeness:** the current installation has `blastwa.exe`, but the installer’s candidate-copy behavior should be verified separately and not expanded into this fix unless the installed-style test cannot launch the binary.

---

## Implementation Units

- [x] U1. **Add profile Chrome fallback resolution**

**Goal:** Make profile-scoped `AppConfig::load()` use the classic config's Chrome fields only when the profile fields are absent or empty.

**Requirements:** R1, R2, R4

**Dependencies:** None

**Files:**
- Modify: `src-tauri/src/config/settings.rs`
- Test: `src-tauri/src/config/settings.rs`

**Approach:** Keep `AppConfig::app_dir()` and profile storage unchanged. Read the classic config as a narrow fallback source, merge only `chrome_path` and `chrome_version`, and retain defaults when neither source provides a value.

**Execution note:** Add characterization tests before changing the load path.

**Patterns to follow:** Existing pure `resolve_app_dir()` tests and `AppConfig` serde defaults.

**Test scenarios:**
- Happy path — missing profile config plus populated classic config -> loaded profile receives the classic Chrome path and version.
- Edge case — profile has an explicit non-empty Chrome path/version -> those values remain unchanged.
- Edge case — profile Chrome path is empty but version is explicit -> only the missing path is inherited.
- Error path — classic config is missing or malformed -> profile load remains usable with default empty Chrome fields rather than panicking.
- Isolation — profile-specific delay, API, and human-mode values remain unchanged when Chrome fields are inherited.

**Verification:** A profile process launched with only the installer-created classic config exposes the detected Chrome path through `get_config`.

---

- [x] U2. **Preserve Chrome launch errors**

**Goal:** Make Open Browser report the actual process-spawn failure when CDP discovery cannot find an endpoint.

**Requirements:** R3, R4

**Dependencies:** U1

**Files:**
- Modify: `src-tauri/src/main.rs`
- Test: `src-tauri/src/main.rs`

**Approach:** Retain the error returned by `SessionManager::launch()` while continuing the existing endpoint discovery path. Include the retained context in the final error only when discovery fails; do not change successful session registration.

**Test scenarios:**
- Happy path — successful session launch -> Open Browser still returns `{ ok: true, port }` and attaches the pipeline.
- Error path — invalid/empty Chrome executable plus no endpoint -> returned error includes both launch context and the endpoint-discovery context.
- Regression — stale live-session cleanup and existing account validation remain unchanged.

**Verification:** A failed installed launch points to the missing/invalid Chrome executable instead of only saying that a CDP endpoint was not found.

---

- [x] U3. **Correct the installed settings guidance**

**Goal:** Remove the obsolete bootstrapper name from the user-facing Chrome settings placeholder.

**Requirements:** R1, R3

**Dependencies:** U1

**Files:**
- Modify: `src/pages/settings.html`
- Test: `scripts/verify_router_lifecycle.js` (existing frontend lifecycle check)

**Approach:** Change the placeholder/label to refer to setup.exe or the detected runtime configuration, without introducing a browser-side Chrome detector.

**Test scenarios:**
- Happy path — settings page renders the new setup guidance and still displays a non-empty runtime path when supplied by IPC.
- Regression — router lifecycle test continues to pass after the page text change.

**Verification:** Users are directed toward the current installer/runtime path and no longer toward `blastwa-init.exe`.

---

## System-Wide Impact

- **Interaction graph:** `main()` -> profile config load -> `open_browser` -> `launch_session` -> `SessionManager::launch` -> CDP discovery -> pipeline attach.
- **Error propagation:** a process-spawn error is preserved through the async task and combined with the final discovery error.
- **State lifecycle risks:** fallback must not write the classic values into the profile file or overwrite explicit profile fields.
- **API surface parity:** no REST or frontend IPC command shape changes are required.
- **Integration coverage:** installed-style verification must inspect root config, launch a named profile, read Settings, and click Open Browser.
- **Unchanged invariants:** profile directory isolation, loopback-only CDP/API behavior, and existing port exclusion remain intact.

---

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Classic and runtime data-directory resolvers diverge on Windows | Verify both target the same per-user root before changing installer code; current machine evidence shows the classic config at the expected root. |
| A profile contains a stale explicit Chrome path | Preserve explicit values and report the path-specific spawn error; do not silently replace user configuration. |
| Chrome is already locked or a fixed port is occupied | Validate the path fallback first, then use the existing isolated user-data directory and CDP discovery behavior for secondary diagnosis. |
| Existing dirty installer changes are accidentally overwritten | Limit edits to the listed source/page files and inspect the final diff before staging. |

---

## Optimization Review

The requested ce-optimize review does not justify a multi-branch experiment loop: this is a deterministic correctness bug, and the current worktree is intentionally dirty with installer artifacts. The useful hard metrics are:

- Profile config fallback test coverage: missing classic/profile combinations must pass 100%.
- Explicit override preservation: 100% of override cases retain profile values.
- Diagnostic error coverage: invalid executable tests must include the underlying spawn context.
- Installed-style outcome: Settings must show a real Chrome path before Open Browser is attempted; successful CDP discovery is the end-to-end gate.

No performance optimization should be accepted if it weakens profile isolation or error detail. A full ce-optimize run can be performed later on a clean, isolated branch if launch latency or CDP discovery time becomes the measurable goal.

---

## Documentation / Operational Notes

- Rebuild both the application and setup executable for installed-style verification; do not test an old `blastwa.exe` beside a new installer.
- Verify the same Windows user performs installation and launch because configuration is per-user.
- Preserve the existing live Chrome ports: WebView2 test automation uses 9223 and WhatsApp Chrome sessions use 9222+.

---

## Sources & References

- Related code: `src-tauri/src/config/settings.rs`, `src-tauri/src/main.rs`, `src-tauri/src/browser/cdp_client.rs`, `setup/src/main.rs`, `src/pages/settings.html`
- External docs: `https://chromium.googlesource.com/chromium/src/+/main/docs/user_data_dir.md`, `https://chromedevtools.github.io/devtools-protocol/`, `https://doc.rust-lang.org/std/process/struct.Command.html`
