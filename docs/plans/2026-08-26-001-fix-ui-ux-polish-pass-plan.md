---
title: "fix: BlastWA UI/UX polish pass — alignment, tokens, native-dialog replacement, live status"
type: fix
status: active
date: 2026-08-26
---

# BlastWA UI/UX Polish Pass — Senior Designer Critique & Fix Plan

**Target repo:** blastwa (repo root is `D:/Tes`, all paths below are repo-relative against it)

## Overview

A senior-designer polish pass over the BlastWA desktop UI: fix every misalignment the user screenshotted (footer stat bars, table action buttons, account chips, checkboxes), normalize the type/height system, replace the ugly native `prompt()` Add Account dialog with an in-app modal, and make account status update live instead of requiring a tab switch. Research surfaced three silent functional bugs that this pass also fixes: an undefined `var(--primary)` token (5 references in 4 dead style rules), a currently-red router lifecycle harness whose failures don't set a non-zero exit code, and a dashboard poll that destroys in-flight inline renames.

---

## Problem Frame

The user (non-coder) screenshotted ten UI areas and asked for a designer-grade critique of "margin position, width, length, font width". The screenshots predate the last source edits (frontend assets are embedded at Rust compile time — `frontendDist: "../src"`, no dev server), so some fixes already exist in source but were never rebuilt or verified. This plan treats current source as baseline: verify what is already fixed, finish what is not, and add the two missing features (modal, live status).

### Designer Critique (per screenshot, with current values)

1. **Counts-bar footers (dashboard/contacts/groups/autoreply/log/templates — the six footers that exist)** — pre-rebuild: `4px 10px` padding, raw 11px `Total: 2` text, no visual grouping. Source now has a pill system (`.counts-bar > span` → `999px` pills, accent numbers) — unverified in the exe. Groups footer additionally crams `Total: 21` + `cached 14h ago · tes` text against three 30px buttons with no separator; 30px buttons dominate an 11.5px text bar.
2. **Rows-shown slider (contacts)** — thumb sits ~2px off vertical center against its label; value `15` has no width reservation (jitters 15↔5); 110px track is short relative to the label; the whole row lacked breathing room from `Total` pill.
3. **Table action buttons (dashboard)** — `Open Browser`/`Remove` are 24px `.btn-sm`; the pencil `.icon-btn` rendered teal-on-teal (dark icon on dark accent bg) and looked taller than neighbors. Source fix exists (white bg, hover→accent+white icon) but unverified.
4. **Account chips (sending)** — dot (8px) vs mixed 12px/11px text baselines made `● tes Offline` look vertically scrambled; `line-height: 16px` fix exists in source, unverified.
5. **Checkboxes (sending)** — native checkboxes, no `accent-color`, default blue clashes with the teal identity; label/checkbox vertical centering relies on browser defaults.
6. **Add Account dialog** — native WebView2 `prompt()` renders the "tauri.localhost says" chrome dialog: off-brand, ugly, and historically unreliable in Tauri webviews. Must become an in-app modal.
7. **Stale status** — sending-page account chips render once on navigation; a QR scan completing on the dashboard is invisible until the user switches tabs.
8. **Typography drift** — 10 hardcoded font sizes (10/10.5/11/11.5/12/12.5/13/14/16/18px) and control heights {22, 24, 26, 28, 30}px with no tokens; `var(--primary)` referenced 5× (in 4 rules) but never defined (chip hover/selected/check + emoji-tab active styles silently dead).

---

## Requirements Trace

- R1. Every page footer reads as aligned stat pills with consistent padding/gap; footer action buttons align with the bar and don't crowd text.
- R2. Rows-shown slider: thumb vertically centered, reserved-width value, comfortable gaps, persists across navigation.
- R3. Dashboard action buttons: uniform 24px height, pencil icon white-bg with accent hover, all vertically centered in the cell.
- R4. Account chips: dot/text/check vertically centered, consistent padding, selected state visibly styled (requires working `--primary`).
- R5. Sending-page checkboxes align with labels and use teal `accent-color`.
- R6. Add Account opens an in-app modal matching the profile-launcher pattern (Enter saves, Escape/backdrop cancels, error slot, autofocus).
- R7. Sending-page chips refresh live (~3s) without tab switching, with change-detection so clicks aren't disturbed, and cleanup on navigation.
- R8. Type/height tokens exist and outliers converge (controls: 30px primary / 24px small; 26px menubar exempt as window chrome); `verify_router_lifecycle.js` green before and after page-script edits — and its exit code is trustworthy (failures set non-zero exit).

Grouping: R1–R5 visual alignment · R6–R7 new features · R8 tooling & tokens.

## Scope Boundaries

- Not replacing the ~40 `alert()` calls with toasts (separate pass; native alert has shown no reliability issues in this app's daily use — the prompt() replacement is driven by its ugly native chrome plus the inline-rename precedent, not by alert breakage).
- Live chips stay status-only on the Sending page: a non-connected chip's recovery action (Open Browser → QR) remains on the Dashboard — accepted scope for this pass.
- Not redesigning the app shell/navigation or switching fonts (Helvetica-first stack is a deliberate earlier decision).
- Not adding new pages or backend features; Rust changes limited to none (all fixes are frontend + harness).
- Not vendoring icon/font libraries (no-network constraint).

---

## Context & Research

### Relevant Code and Patterns

- Modal blueprint: `blastwa/src/index.html:120–134` + `blastwa/src/main.js:200–260` (profile launcher) — overlay outside router, single instance, `.hidden` toggle, backdrop-click guard `ev.target === overlay`, Enter-to-submit, error slot.
- Polling precedents: `blastwa/src/main.js:310` (module-scope 3s statusbar), `blastwa/src/pages/dashboard.html:41–45` (page-scope 3s + `addCleanup`).
- Page scripts re-execute per navigation (router re-creates `<script>` nodes): top-level `const` throws on second visit — use `var`/function-scope (comment at `blastwa/src/pages/dashboard.html:46–50`).
- Pills/slider/dots already in source: `blastwa/src/styles.css:233–257` (.counts-bar), `:260–264` (.row-slider), `:391–421` (.counters with `:has()` dots), `:653–683` (.account-chip).
- Undefined token refs: `blastwa/src/styles.css:627,665–668,682` use `var(--primary)`.

### Institutional Learnings

- Frontend edits require `cargo build --features gui` to appear in the exe (assets embedded at build time); browser mock mode (`index.html` direct open) allows no-rebuild iteration.
- `:has()`-generated status dots silently die if `status-sent|failed|pending` classes move — restyle around them, don't rename.
- Never hold async locks across long awaits (commit `e608649` froze all UI polling once).
- Harnesses are the safety net: `verify_router_lifecycle.js`, `check_sending_page.js` (asserts every `$('id')` exists), `check_groups_cache.js`, `check_checker_cache.js`.

### External References

- Skipped deliberately: local patterns are strong (in-repo modal + polling precedents); no external UI library may be added (no-bundler constraint).

---

## Key Technical Decisions

- **Fix the harness first (U1)**: `verify_router_lifecycle.js` currently exits 3 FAILURES (`requestAnimationFrame is not defined` — the new rows-slider in `contacts.html:456` uses rAF; the harness VM mock lacks it). It exists to catch the page-script pitfall during multi-navigation; restore it before touching any page script.
- **Define `--primary: var(--accent)`** rather than renaming 5 refs: smallest diff, keeps both names meaningful, un-deadens chip selected/hover and emoji-tab active instantly.
- **Footer action buttons become `.btn-sm`** (24px): in a footer bar with no adjacent 30px input, 30px buttons visually dominate 11.5px text; 24px reads as secondary chrome. (Contrast: generator/checker rows keep 30px buttons beside 30px inputs — user explicitly wanted those matched.)
- **Page-scoped modal in `dashboard.html`** (not index-level): dashboard is the only consumer, page scripts re-execute cleanly with `var`, and it avoids growing the module shell. Pattern copied 1:1 from profile modal (overlay/panel/footer classes already exist).
- **Chips poll with snapshot-diff**: re-render only when the JSON of `[{name,connected,browser_running,port,number}]` changes, so an open dropdown/click is never yanked mid-interaction; interval registered with `addCleanup`.

---

## Open Questions

### Resolved During Planning

- Are the user's reported misalignments still live? — Mixed: several fixes already exist in source but were never rebuilt; plan verifies each against fresh screenshots instead of assuming.
- Does `prompt()` work in the exe? — It renders (user screenshot) but is native-styled and historically flaky in Tauri webviews; replacing it is both a UX and reliability fix.

### Deferred to Implementation

- Exact token names for font sizes — decided while editing under the ≥2-consumers rule (single-use literals stay); the final size→token mapping gets recorded as a comment in `styles.css` at U2 close.

---

## Implementation Units

- [x] U1. **Restore the red safety net**

**Goal:** `node blastwa/src/verify_router_lifecycle.js` exits green (currently 3 failures).

**Requirements:** R8

**Dependencies:** None — must land before any page-script edit.

**Files:**
- Modify: `blastwa/src/verify_router_lifecycle.js`

**Approach:**
- Add a `requestAnimationFrame` stub to the harness's VM window mock (`cb => setTimeout(cb, 0)` is sufficient for the slider's one-shot measurement call).
- Make failures trustworthy: set `process.exitCode = 1` when any FAIL line is recorded (today it prints 3 FAILURES yet exits 0 — exit-code gating is broken).

**Patterns to follow:**
- Existing window mock structure in the same file.

**Test scenarios:**
- Happy path: run harness → all navigations (nav1/nav2 + shared-context sequence) pass, exit code 0.
- Error path: temporarily break a page script reference → harness prints FAIL and exits non-zero.

**Verification:**
- Harness exits 0 with no FAIL lines.

- [x] U2. **Token & control-height normalization**

**Goal:** One source of truth for type sizes and control heights; `--primary` defined; checkboxes themed and aligned.

**Requirements:** R4, R5, R8

**Dependencies:** U1 (harness green first).

**Files:**
- Modify: `blastwa/src/styles.css`

**Approach:**
- `:root`: add `--primary: var(--accent);` and font-size tokens for sizes with ≥2 consumers (single-use literals stay as-is — no naming surface without reuse); map the 10 literals onto tokens, keeping 10px only for `.badge`/`.tag`.
- Heights: controls converge to 30px (`.btn`, inputs, `.modal-body input` 28→30) or 24px (`.btn-sm`, `.fmt-btn`, `.rename-input`); `.profile-chip` 22→24; menubar 26 stays (chrome, not a control). Note: `.modal-body`/`.profile-chip` live in the index-level profile modal — U7 must screenshot it.
- Checkboxes: `input[type="checkbox"] { accent-color: var(--accent); width: 14px; height: 14px; vertical-align: -2px; }` + `.checkbox-label { display: flex; align-items: center; gap: 8px; }` (verify the three sending-page variants share one pattern; the inline `style="margin-bottom:0;margin-top:8px"` hacks may normalize into the class). This global rule also restyles settings/autoreply/contacts checkboxes — they join the U7 capture list.

**Patterns to follow:**
- Existing `:root` 4px-grid token block.

**Test scenarios:**
- Happy path: behavioral, not grep — chip selected tint (via `color-mix`) and emoji-tab active style actually render after aliasing; `.counts-bar > span b` and `.counters b` keep accent color.
- Edge case: `.badge`/`.tag` still 10px; single-use literals unchanged.

**Verification:**
- `check_sending_page.js` still passes (all `$('id')` refs intact); visual: checkbox rows aligned, teal checkboxes.

- [x] U3. **Footer system: pills, slider, groups buttons**

**Goal:** All six footers (dashboard/contacts/groups/autoreply/log/templates) read as one system; slider aligned; groups buttons verified quiet and spaced.

**Requirements:** R1, R2

**Dependencies:** U2 (tokens).

**Files:**
- Modify: `blastwa/src/styles.css`

**Approach:**
- Verify existing pill CSS in the rebuilt exe; adjust: `.row-slider input[type="range"]` gets `vertical-align: middle` and `height: 16px` (thumb centering), `.row-slider b` keeps `min-width: 20px`, slider row gap 8→10px.
- Groups footer: the three buttons are already `.btn .btn-sm` in source — verify only; the remaining work is a 12px gap before the button group (smaller diff, no new CSS rule) so the `cached …` text never touches a button.
- Confirm every footer's pills wrap gracefully at min window width (980px) via flex-wrap (already set — verify visually).

**Test scenarios:**
- Happy path: screenshot each of the six footers at 1280×820 and 980×640 — pills aligned, no crowding.
- Edge case: slider at min=5 and max=40 — value label never reflows the bar (reserved width).
- Edge case: navigate contacts → dashboard → contacts — slider value persists (localStorage `ROWS_KEY` untouched by this pass; verify the polish didn't break it).

**Verification:**
- Fresh screenshots match the spec; no horizontal overflow at min width.

- [x] U4. **Dashboard actions row + account chips alignment**

**Goal:** Uniform 24px action buttons with a legible pencil icon; chips vertically centered with visible selected state.

**Requirements:** R3, R4

**Dependencies:** U2 (`--primary` alive makes selected-chip styling work).

**Files:**
- Modify: `blastwa/src/styles.css` (only if verification shows drift)

**Approach:**
- Verify in exe: `.icon-btn` white bg / accent hover with white icon; all three buttons 24px, `vertical-align: middle` in the cell.
- Chips: `line-height: 16px` on name/state, dot `align-self: center`, chip `align-items: center`; selected chip now visibly tinted (`color-mix` on `--primary` resolves once defined).
- If any drift remains, fix values — do not restructure working CSS.

**Test scenarios:**
- Happy path: dashboard screenshot — pencil button same height as Open Browser/Remove, icon white-on-teal only on hover.
- Happy path: sending page — select two chips, ✓ appears, tint visible, dot/text baselines aligned.
- Edge case: long account name — chip wraps state text or ellipsizes without breaking row height.

**Verification:**
- Screenshots confirm; no layout shift on hover/selection.

- [x] U5. **Add Account in-app modal**

**Goal:** Replace the native `prompt()` with a styled modal matching the profile-launcher pattern.

**Requirements:** R6

**Dependencies:** U1.

**Files:**
- Modify: `blastwa/src/pages/dashboard.html`
- Modify: `blastwa/src/styles.css` (only if page-scoped tweaks needed)

**Approach:**
- Markup: `.modal-overlay.hidden` + `.modal-panel` (reuse existing classes) with title "Add Account", label "Account name" + hint copy (allowed charset, max length), text input `maxlength="64"`, `.modal-error` slot, Cancel / Add buttons in `.modal-footer`.
- Behavior: `window.addAccount()` opens the modal instead of `prompt()`; input autofocus + select (keep inside the open-click handler so the harness's element mocks — which lack `focus()`/`select()` — stay green); Enter submits, Escape cancels, backdrop click cancels only when `ev.target === overlay`.
- Listener discipline: the Escape keydown is the page's first document-level listener — register it and clean it up: `window.blastwa.addCleanup(() => document.removeEventListener('keydown', handler))`, or the closures stack per dashboard visit forever.
- Validation: client-side duplicate check before invoke (backend `save_account_name` silently no-ops on duplicates — it will never error); surface the real backend failures (empty name, >64 chars, invalid charset per `validate_account_name`) in the error slot.
- Submit state: disable the Add button while the invoke is pending (Enter-spam must not double-fire `add_account`).
- Focus return: on close (save/cancel/Escape), return focus to the "+ Add Account" trigger.
- On success: close modal, existing `add_account` invoke + warning surface stays as-is, then `renderAccounts()`.
- Page-scope discipline: `var`/function declarations only at top level (scripts re-execute per navigation).

**Patterns to follow:**
- `blastwa/src/index.html:120–134` + `blastwa/src/main.js:200–260` (overlay toggle, error slot, Enter handling).

**Test scenarios:**
- Happy path: open modal → type "Test Acc" → Enter → account appears in table, modal closed, focus back on trigger.
- Error path: submit empty → inline error text, modal stays open.
- Error path: submit name failing `validate_account_name` (>64 chars / invalid charset) → backend error shown in slot, modal stays open.
- Error path: submit duplicate of an existing name → client-side check blocks the invoke with a duplicate message; backend is never relied on (it no-ops silently).
- Edge case: Escape and backdrop click close without side effects; reopening shows empty input (no stale text).
- Edge case: double-Enter quickly → exactly one `add_account` invoke (submit disabled while pending).
- Integration: after successful add, the 3s poll reflects the new row without manual refresh.
- Integration: dashboard → sending → dashboard — Escape handler still works exactly once (no stacked listeners; proven by `verify_router_lifecycle.js` multi-navigation).

**Verification:**
- Live CDP drive: modal opens, saves, cancels; screenshots captured.

- [x] U6. **Live status everywhere, without yanking**

**Goal:** Chips and counters reflect reality without switching tabs — on the Sending page AND on the Dashboard, where today's unconditional 3s poll destroys in-flight inline renames (same yank class this unit fixes).

**Requirements:** R7

**Dependencies:** U1.

**Files:**
- Modify: `blastwa/src/pages/sending.html`
- Modify: `blastwa/src/pages/dashboard.html`

**Approach:**
- Sending: add page-scoped `setInterval` (3s) calling `list_accounts` and re-rendering chips **only when** `JSON.stringify` of the mapped snapshot `[{name,connected,browser_running,port,number}]` differs from the last render (prevents click yanking).
- Dashboard: gate `renderAccounts()` with the same snapshot-diff, plus a hard guard — skip re-render entirely while any row contains `[data-renaming]` (the inline-rename input dies with `innerHTML` replacement otherwise).
- Register cleanup for both: `window.blastwa.addCleanup(() => clearInterval(poll))` — same pattern as `dashboard.html:41–45`.
- Counters already update via `campaign_progress` events — verify; do not add competing polling for them.

**Patterns to follow:**
- `blastwa/src/pages/dashboard.html:41–45` (poll + cleanup), `blastwa/src/main.js:305–337` (statusbar poll resilience: keep last-known state on transient errors).

**Test scenarios:**
- Integration: with `tes` connected, sit on Sending — chip shows Connected within ~3s of state change without navigation.
- Integration: start an inline rename on Dashboard, wait through two poll ticks — input survives, edit intact, table refreshes after the rename completes or cancels.
- Edge case: navigate away mid-poll → interval cleared (no ghost updates; harness `verify_router_lifecycle.js` proves cleanup).
- Error path: backend transient failure during poll → chips keep last-known state, no console error spam.

**Verification:**
- CDP live test: trigger state change (open/close browser), observe chip update without tab switch; rename-survival test on Dashboard.

- [x] U7. **Rebuild, full verification, screenshots**

**Goal:** Everything above proven in the actual exe.

**Requirements:** R1–R8

**Dependencies:** U1–U6.

**Files:**
- Create: `blastwa/debug-shots/cdp.js` (one parameterized capture runner: `node cdp.js <page-hash> <out.png>`; today `debug-shots/` holds only ad-hoc PNGs with no committed generator)
- Modify: `blastwa/debug-shots/` (new verification screenshots)

**Approach:**
- `cargo build --features gui` (assets are embedded — non-negotiable), kill old exe, relaunch with `--remote-debugging-port=9223`.
- Run: `node blastwa/src/verify_router_lifecycle.js`, `node blastwa/src/check_sending_page.js`, `node blastwa/src/check_groups_cache.js`, `node blastwa/src/check_checker_cache.js`, `cargo test --features gui --lib` (57 tests).
- CDP-capture screenshots of all 8 routed pages (dashboard/contacts/groups/sending/templates/autoreply/log/settings) at 1280×820, plus the index-level profile-launcher modal (its `.modal-body input`/`.profile-chip` heights changed in U2); eyeball against the critique list; iterate if any drift. Explicitly confirm the chip selected tint renders (`color-mix()` needs Chromium 111+ — same evergreen bet as `:has()`, but verify, don't assume).

**Test scenarios:**
- Integration: full suite green; screenshots archived per page (8 pages + profile modal).
- Edge case: min window 980×640 — no overflow in any footer.

**Verification:**
- All harnesses + cargo tests green; fresh screenshots showing every R1–R7 item fixed.

---

## System-Wide Impact

- **Interaction graph:** router re-executes page scripts on every navigation — every new top-level binding must be `var`/function; intervals must be `addCleanup`-registered or they leak across pages.
- **Error propagation:** modal error slot replaces `alert()` for Add Account only; other alerts untouched (scope boundary).
- **State lifecycle risks:** chips re-render must be diff-gated or in-flight clicks are lost; `:has()` dot selectors die if `status-*` classes are renamed — they are not.
- **API surface parity:** no IPC changes; `add_account` invoked exactly as before.
- **Unchanged invariants:** generator/checker 30px button rows (user-approved), Helvetica stack, teal palette, all backend behavior.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Page-script `const` re-declaration kills whole page on 2nd visit | U1 harness green gates all page edits; `var` discipline in U5/U6 |
| Chips/table poll yanks UI mid-click or mid-rename | Snapshot-diff gate + `[data-renaming]` guard before re-render (U6) |
| CSS edits invisible in exe → "nothing changed" confusion | U7 rebuild is explicit; browser mock mode for fast iteration |
| `:has()`/`accent-color`/`color-mix()` on old WebView2 (color-mix needs Chromium 111+) | Evergreen runtime on Win10/11; U7 visually confirms the tint renders; degrades to border-only selection, acceptable |
| Token aliasing changes colors unexpectedly | `--primary: var(--accent)` is identity-preserving by definition; U2 verifies behaviorally |
| Document-level Escape listener stacks per dashboard visit | U5 registers removal via `addCleanup`; harness multi-navigation proves it |

---

## Sources & References

- Research: ce-repo-research-analyst + ce-learnings-researcher runs, 2026-08-26 (exact values cited inline).
- User screenshots (10): footer stat bars across pages, action buttons, chips ×2, checkboxes, sending status, native prompt dialog.
- Document review: ce-design-lens / ce-coherence / ce-scope-guardian / ce-feasibility reviewers, 2026-08-26 — all P1 findings applied (footer count 6, 8-page capture list, cdp.js as new file, harness exit code, duplicate-name client-side check, Escape-listener cleanup, rename-vs-poll guard, profile-modal capture).
- Related commits: `e608649` (poll freeze), `a4d292c` (cache overwrite guard).
