---
title: "feat: Multi-Profile Launcher, Live Campaign Progress, Formatting Toolbar"
type: feat
status: active
date: 2026-08-25
---

# Multi-Profile Launcher, Live Campaign Progress, Formatting Toolbar

## Overview

Four user-facing gaps in BlastWA close here:

1. **Multi-profile launcher (OKESENDER parity)** — run account A's campaign and account B's campaign at the same time by opening a second BlastWA window bound to a different profile. Each profile is a fully isolated instance: own accounts, contacts, templates, settings, logs, API port. Chosen architecture: profile launcher (option A), matching OKESENDER's launcher principle.
2. **Live campaign progress** — the Send Campaign page's Sent/Failed/Pending counters never move because the backend never emits `campaign_progress`; the sender already has an `on_progress` callback hook that the GUI layer never wired to a Tauri event.
3. **Message formatting toolbar** — Bold / Italic / Strikethrough / Monospace / Emoji buttons above the message body, matching OKESENDER's compose toolbar.
4. **Settings semantics clarification** — Campaign Settings (per-run: account, delays, preset, safe mode) vs the Settings page (app-level: chrome path, API, WPP) get explicit labeling so the split is obvious.

---

## Problem Frame

The app currently behaves as a single global instance: one contact list, one running campaign at a time (`AppState.running` is a single `AtomicBool`), one settings file. The user runs multiple WhatsApp accounts and wants OKESENDER-style parallelism: launch a second app window bound to a different profile and blast from both simultaneously.

Independently, the campaign progress UI is dead (frontend listens for `campaign_progress`, backend never emits it), the compose box lacks the formatting toolbar OKESENDER has, and the split between "Campaign Settings" and the "Settings" page is unlabeled and confusing.

---

## Requirements Trace

- R1. `blastwa.exe --profile <name>` runs a fully isolated instance: own data dir, accounts, contacts, templates, settings, logs, API port.
- R2. Default run (no flag) behaves exactly as today (data in `%APPDATA%\BlastWA`).
- R3. From inside any window, the user can open a new window bound to a new or existing profile (File menu).
- R4. API port conflicts between profiles are avoided automatically (auto-pick free port when the configured one is taken).
- R5. While a campaign runs, Sent/Failed/Pending counters and the progress bar update live (per send, not on completion).
- R6. Message body toolbar inserts WhatsApp formatting (`*bold*`, `_italic_`, `~strike~`, `` ```mono``` ``) around the current selection and inserts emoji at the cursor.
- R7. Campaign Settings and the Settings page are visually/verbally distinguished (per-run vs app-level).
- R8. Campaign logs persist to disk per profile: after an app restart, past campaigns are viewable with date and sent/failed counts (OKESENDER "Sent Campaigns" parity).
- R9. Export filenames auto-include a timestamp (`blastwa-groups-2026-08-25-1442.csv`) so exports never overwrite each other and never need manual renaming.
- R10. A campaign can carry multiple message variants rotated across contacts (contact i gets variant i mod N), each variant still running through spintax + variables + Human Mode.
- R11. The number checker's valid results can be imported into the contact list in one click (filter-then-blast flow).
- R12. Pause actually suspends the send loop and Resume continues it, with a visible PAUSED state (audit + fix of the existing buttons).
- R13. A campaign can be scheduled for a future time, with a visible countdown and the ability to cancel before it fires.
- R14. Blind mode: skip delivery-status verification for speed (no status in logs).
- R15. Audio attachments can be sent as voice notes (PTT); GIFs render animated.
- R16. The account's WhatsApp phonebook can be imported into the contact list with selection.
- R17. Messages with interactive buttons or a list menu can be composed and sent (WPP capability permitting).
- R18. Catalog-style messages (title, description, items) can be built, saved, and sent.
- R19. One campaign can fan out across multiple connected accounts without duplicates.
- R20. Candidate numbers can be generated from a prefix range and fed into the checker.
- R21. OKESENDER "familiar accounts" semantics are researched and a go/no-go decision recorded.

---

## Scope Boundaries

- No per-account campaigns inside one window (option B) — parallelism comes from multiple profile windows.
- No cross-profile contact/template sharing.
- No changes to CDP architecture, Chrome isolation, or account persistence format.
- No license checks (unchanged principle).
- Excel per-sheet group export already shipped — untouched.

---

## Context & Research

### Relevant Code and Patterns

- `src-tauri/src/config/settings.rs` — `AppConfig::app_dir()` is the single data-root resolver; every storage path derives from it. Profile-awareness lands here.
- `src-tauri/src/main.rs` — `main()` / tauri Builder; CLI arg parsing goes here; window title set in `src-tauri/tauri.conf.json`.
- `src-tauri/src/api/server.rs` — `AppState`, `start_server(api_port)`; port auto-pick on bind failure.
- `src-tauri/src/campaign/sender.rs` — `on_progress: impl Fn(ProgressEvent)` hook exists; `ProgressEvent { sent, failed, pending, current_number, status }`.
- `src-tauri/src/main.rs` `start_campaign` — spawns the pipeline; the emit closure wires in here.
- `src/pages/sending.html` — `listen('campaign_progress')` already correct on the frontend; compose toolbar goes here.
- `src/pages/settings.html` — app-level settings live here.
- `src/main.js` — menubar (File menu) definition for the "New Window (Profile)" action.

### Institutional Learnings

- Tauri v2: `window.__TAURI__` requires `withGlobalTauri` (already enabled); dialog plugin already installed with `dialog:default` capability — the save dialog for exports is a working pattern to copy.
- Frontend assets are embedded at Rust build time — frontend edits require `cargo build --features gui`, not just a refresh.
- innerHTML-injected page scripts need the router's re-execution lifecycle (already in place; new pages/controls just work).
- Concurrent multi-MB CDP evaluates reset the websocket — chunked WPP injection exists; do not reintroduce single-shot large evaluates.

---

## Key Technical Decisions

- **Profile = isolated data root**: `--profile <name>` resolves the app dir to `%APPDATA%\BlastWA\profiles\<name>`; everything under the existing `app_dir()` automatically isolates (accounts.json, Profiles/, accounts/, templates/, Data/, Reports/, wpp/). No storage format changes.
- **Profile passed via env + CLI**: main() parses `--profile <name>` and sets the resolved dir before any config load; also accept `BLASTWA_PROFILE` env for launcher convenience.
- **API port per profile**: each profile's `config.json` keeps its own `api_port`; on bind failure the server walks up to the next free port and persists the choice.
- **Progress emission via existing hook**: pass an `on_progress` closure into the pipeline call from `start_campaign` that does `app_handle.emit("campaign_progress", payload)` — no new event system.
- **Toolbar is plain text insertion**: WhatsApp formatting is literal characters (`*`, `_`, `~`, backticks) around `textarea.selectionStart/End`; emoji picker is a fixed grid of common emojis inserted at the cursor. No rich-text editor.

---

## Open Questions

### Resolved During Planning

- Architecture fork (launcher vs in-window multi-campaign): user chose the profile launcher (option A).
- Where profile windows come from: File menu action spawning a detached `blastwa.exe --profile <name>` process (no separate launcher binary needed).

### Deferred to Implementation

- Exact sanitize rules for profile directory names (must be filesystem-safe on Windows).
- Whether the profile picker in the File menu should list existing profiles from disk scan of `profiles/` (likely yes — cheap readdir).

---

## Implementation Units

- [x] U1. **Profile-aware data root**

**Goal:** `--profile <name>` isolates all app data under `profiles/<name>/`.

**Requirements:** R1, R2

**Dependencies:** None

**Files:**
- Modify: `src-tauri/src/config/settings.rs`
- Modify: `src-tauri/src/main.rs`
- Test: `src-tauri/src/config/settings.rs` (inline tests)

**Approach:**
- `AppConfig` gains a profile field (or a process-global set once in main before any config load).
- `app_dir()` returns `%APPDATA%\BlastWA` when no profile, `%APPDATA%\BlastWA\profiles\<name>` otherwise.
- Window title gains a `Profile: <name>` suffix when a profile is active (default profile stays untitled).

**Patterns to follow:**
- Existing `AppConfig::app_dir()` call sites — none of them should need changes.

**Test scenarios:**
- Happy path: no flag → app_dir is the classic root (existing behavior, existing tests stay green).
- Happy path: profile "work" → app_dir ends with `profiles/work`.
- Edge case: profile name with path separators/illegal chars → sanitized to a safe single directory segment.

**Verification:**
- Launching with `--profile work` creates `profiles/work` on first save and never touches the root data dir.

---

- [x] U2. **Profile launcher + per-profile API port**

**Goal:** Open additional BlastWA windows bound to profiles from the File menu; API ports never collide between profiles.

**Requirements:** R3, R4

**Dependencies:** U1

**Files:**
- Modify: `src/main.js` (File menu item)
- Modify: `src-tauri/src/main.rs` (spawn command + api port fallback)
- Modify: `src-tauri/src/api/server.rs` (port walk + persist)
- Modify: `src/pages/settings.html` (show effective profile + api port, read-only)

**Approach:**
- Tauri command `open_profile_window(profile: String)`: validates/sanitizes the name, spawns the current exe detached with `--profile <name>` (`std::process::Command` with creation flags for detached on Windows).
- File menu gains "New Window (Profile...)" → JS prompt (or dialog) for the name → invoke.
- `start_server`: try configured port; on bind error, walk +1 until free; persist the effective port back to the profile's config.

**Patterns to follow:**
- Existing menubar wiring in `src/main.js`; existing dialog plugin usage for prompts if a native picker is preferred.

**Test scenarios:**
- Happy path: File → New Window (Profile "b") → second window opens with `Profile: b` in the title and its own empty accounts table.
- Edge case: opening a profile that already exists reuses its data (accounts persisted under that profile reappear).
- Error path: default API port taken by profile A → profile B's server lands on 8766+ and the Settings page shows it.
- Integration: campaign running in window A does not block a campaign in window B (separate processes).

**Verification:**
- Two windows, two profiles, two simultaneous campaigns, no port errors in either log.

---

- [x] U3. **Live campaign progress emission**

**Goal:** Sent/Failed/Pending and the progress bar update in real time during a blast.

**Requirements:** R5

**Dependencies:** None

**Files:**
- Modify: `src-tauri/src/main.rs` (`start_campaign` wiring)
- Modify: `src-tauri/src/campaign/pipeline.rs` (thread the callback through if not already exposed)
- Modify: `src-tauri/src/campaign/sender.rs` (only if the hook signature needs an emit-friendly shape)

**Approach:**
- `start_campaign` builds an `on_progress` closure capturing `AppHandle` and emitting `campaign_progress` with the `ProgressEvent` fields (camelCase keys to match the frontend listener: `sent`, `failed`, `pending`, `current_number`, `status`).
- Throttle not required (one event per send is naturally rate-limited by human-mode delays).
- `get_status` keeps working unchanged; the counters in `AppState` stay the source of truth for the REST API.

**Patterns to follow:**
- Frontend listener in `src/pages/sending.html` (`listen('campaign_progress', ...)`) — emit payload shape must match what it reads.

**Test scenarios:**
- Happy path: campaign of 3 contacts → UI counters tick 1/0/2 → 2/0/1 → 3/0/0 and the bar fills to 100%.
- Edge case: failed send increments Failed and the bar still progresses.
- Error path: pressing Stop mid-run → final event arrives, counters freeze, no further events after stop.
- Integration: progress events from window A never appear in window B (separate processes/webviews).

**Verification:**
- Manual blast to 2-3 numbers shows live counter movement; log shows one emitted event per send attempt.

---

- [x] U4. **Message formatting toolbar + emoji picker**

**Goal:** OKESENDER-style Bold/Italic/Strikethrough/Monospace/Emoji buttons above the message body.

**Requirements:** R6

**Dependencies:** None

**Files:**
- Modify: `src/pages/sending.html`

**Approach:**
- A slim button row between the label and the textarea: **B**, *I*, ~S~, `` ` `` mono, 🙂 emoji.
- Each formatter wraps the current textarea selection with the WhatsApp marker (`*sel*`, `_sel_`, `~sel~`, `` ```sel``` ``); empty selection inserts the marker pair and places the cursor inside.
- Emoji button toggles a small inline grid (common emojis: 😀😂🥹😍🤩😎👍🙏🔥💯🎉✅❤️🤣😭😅🙌💪👌🤝🚀💡) inserting at the cursor.
- Draft persistence (existing localStorage mechanism) automatically covers toolbar output — no extra work.

**Patterns to follow:**
- Existing button styles (`btn btn-sm`); existing draft `saveDraft()` wiring in the same file.

**Test scenarios:**
- Happy path: select "promo", click B → `*promo*` in the textarea.
- Edge case: no selection → clicking B inserts `**` with the cursor between the stars.
- Edge case: toolbar edits trigger the draft save (switch tabs and back → formatting preserved).
- Happy path: emoji click inserts at cursor without losing focus of the textarea.

**Verification:**
- Compose a message with bold + emoji, send to a test number, verify WhatsApp renders the formatting.

---

- [x] U5. **Settings semantics clarification**

**Goal:** Make the Campaign Settings vs Settings split self-explanatory.

**Requirements:** R7

**Dependencies:** None

**Files:**
- Modify: `src/pages/sending.html` (panel subtitle)
- Modify: `src/pages/settings.html` (section subtitle)

**Approach:**
- Campaign Settings panel gains a one-line subtitle: "Per-campaign settings — applied to this send only."
- Settings page gains a subtitle on the app section: "App-wide settings — Chrome, API, updates. Campaign options live on the Send Campaign page."
- No layout or theme changes.

**Patterns to follow:**
- Existing `page-subtitle` / panel header conventions.

**Test scenarios:**
- Test expectation: none — copy-only change, verified visually.

**Verification:**
- Both pages read unambiguously; no functional change.

---

- [ ] U6. **Persistent campaign log / sent campaigns history**

**Goal:** Campaign logs survive app restarts — past campaigns listed with date, message preview, and sent/failed counts (OKESENDER "Sent Campaigns" parity).

**Requirements:** R8

**Dependencies:** U1 (profiles isolate their history automatically)

**Files:**
- Modify: `src-tauri/src/campaign/log_exporter.rs` (append-to-disk writer)
- Modify: `src-tauri/src/main.rs` (persist log entries at emit time + `list_sent_campaigns` command)
- Modify: `src/pages/log.html` (history list with dates + per-campaign sent/failed summary)

**Approach:**
- Each campaign start appends a JSON-lines record to `app_dir()/Data/campaigns.jsonl`: `{ started_at, account, message_preview, total, sent, failed }`; counters updated on completion (rewrite last line or sidecar state).
- Log page gains a "Past campaigns" section reading that file, newest first; existing live log stays as-is.
- Export button on each history row reuses the CSV exporter.

**Patterns to follow:**
- Existing `log_exporter.rs` CSV writing; existing `Data/` directory convention.

**Test scenarios:**
- Happy path: run a campaign, close the app, reopen → history row with correct date and counts.
- Edge case: app killed mid-campaign → record shows partial counts with status "interrupted".
- Edge case: no campaigns yet → empty-state message, no file errors.

**Verification:**
- Restart the app after a real blast; history shows the campaign without re-running anything.

---

- [ ] U7. **Auto-unique export filenames**

**Goal:** Every export defaults to a timestamped filename — no manual renaming, no silent overwrites.

**Requirements:** R9

**Dependencies:** None

**Files:**
- Modify: `src/pages/groups.html` (CSV + XLSX defaultPath)
- Modify: `src/pages/log.html` (log export defaultPath)

**Approach:**
- Shared helper `stampName(base, ext)` → `blastwa-groups-2026-08-25-1442.csv` (local time, minute precision); used as the save dialog `defaultPath` everywhere exports happen.

**Patterns to follow:**
- Existing save-dialog calls in groups.html.

**Test scenarios:**
- Happy path: two exports a minute apart produce two distinct default filenames.
- Edge case: user keeps the suggested name in a folder with an existing file → native save dialog's own overwrite confirmation handles it.

**Verification:**
- Export twice; second dialog suggests a different name without any manual editing.

---

- [ ] U8. **Multiple message rotation**

**Goal:** Compose N message variants; contact i receives variant i mod N (then spintax + variables + Human Mode apply per send).

**Requirements:** R10

**Dependencies:** None (composes on top of U3's progress events naturally)

**Files:**
- Modify: `src/pages/sending.html` (variant tabs / "Add message" in the Message panel)
- Modify: `src-tauri/src/main.rs` (`start_campaign` accepts `messages: Vec<String>`)
- Modify: `src-tauri/src/campaign/pipeline.rs` + `src-tauri/src/campaign/sender.rs` (rotation at send time)
- Test: `src-tauri/src/campaign/sender.rs` (inline tests)

**Approach:**
- UI: first variant is the existing textarea; "+ Add message" adds variant textareas with remove buttons; draft persistence keys per variant index.
- IPC: `messages: Option<Vec<String>>` — empty/missing falls back to the single `message` field (backward compatible with the REST API).
- Sender: variant index = contact index mod variants.len(), resolved before spintax so each variant spins independently.

**Patterns to follow:**
- Existing draft persistence and spintax/variables pipeline.

**Test scenarios:**
- Happy path: 2 variants, 5 contacts → contacts 1/3/5 get variant A, 2/4 get variant B (verify via log).
- Edge case: one variant only → identical behavior to today.
- Edge case: a variant left empty → treated as absent; rotation skips it (min 1 variant enforced).
- Happy path: spintax resolves independently inside each variant.

**Verification:**
- Blast to test contacts with 2 variants; received messages alternate and spin correctly.

---

- [ ] U9. **Checker → import valid numbers**

**Goal:** Filter-then-blast flow: check numbers, then import only the valid ones into the contact list in one click.

**Requirements:** R11

**Dependencies:** None

**Files:**
- Modify: `src/pages/contacts.html` (Check Numbers panel: input/list, results table, "Import valid" button)
- Modify: `src-tauri/src/main.rs` (expose checker results + `import_valid_contacts` command if not reachable)
- Modify: `src-tauri/src/campaign/checker.rs` (return structured valid/invalid results if it does not already)

**Approach:**
- Panel on the Contacts page: paste numbers or use the current list → run checker against the connected account → results table (number, valid/invalid, error) → "Import valid to list" appends them as contacts.
- Reuses the existing checker engine and the LID/number normalization already in the codebase.

**Patterns to follow:**
- Existing contacts table rendering + import flow; existing checker command wiring.

**Test scenarios:**
- Happy path: 5 numbers (3 real) → checker marks 3 valid → import adds exactly 3 contacts.
- Edge case: checker run with no connected account → clear error, no partial import.
- Edge case: duplicates between results and existing contacts → dedupe respects the Remove-duplicates checkbox.

**Verification:**
- Check a mixed list, import valid-only, see the contact count match the valid count.

---

- [ ] U10. **Pause/Resume audit and fix**

**Goal:** Pause suspends the send loop mid-campaign; Resume continues it; UI shows a PAUSED state.

**Requirements:** R12

**Dependencies:** U3 (progress events carry the paused state)

**Files:**
- Modify: `src-tauri/src/campaign/sender.rs` (pause flag checked between sends)
- Modify: `src-tauri/src/app_state.rs` or `src-tauri/src/api/server.rs` (paused flag on state)
- Modify: `src-tauri/src/main.rs` (`pause_campaign` / `resume_campaign` semantics)
- Modify: `src/pages/sending.html` (Pause toggles to Resume, PAUSED badge in the progress panel)

**Approach:**
- Audit first: confirm what `pause_campaign` does today; if it is a disguised stop, add a `paused: Arc<AtomicBool>` checked at the top of each send iteration (sleep-wait while paused, stop flag still honored).
- Progress events include `paused: true` so the badge renders; Resume emits the next regular event.

**Patterns to follow:**
- Existing `stop_flag` CancellationToken pattern.

**Test scenarios:**
- Happy path: pause mid-campaign → no sends occur while paused (log gap), resume → sends continue from the next pending contact.
- Edge case: pause then Stop → campaign stops cleanly from the paused state.
- Edge case: pause with 0 pending → no-op, state stays consistent.

**Verification:**
- Manual blast with a pause in the middle; received-message timestamps show the gap and clean continuation.

---

- [ ] U11. **Schedule send**

**Goal:** Queue a campaign to start at a chosen time, with a live countdown and cancel-before-fire.

**Requirements:** R13

**Dependencies:** U3 (progress panel hosts the countdown), U10 (stop semantics reused for cancel)

**Files:**
- Modify: `src/pages/sending.html` (Schedule control + countdown display)
- Modify: `src-tauri/src/main.rs` (`start_campaign` gains `schedule_at: Option<String>`; scheduled state + cancel)
- Modify: `src-tauri/src/campaign/pipeline.rs` (delayed dispatch through the existing blast channel)

**Approach:**
- UI: "Send Now" stays; a datetime-local input + "Schedule" button arms the campaign; the progress panel shows a countdown and a Cancel button; the compose draft stays editable until fire.
- Backend: schedule spawns a tokio task sleeping until the target time (woken early by cancel), then pushes the normal BlastRequest; app restart drops the schedule (documented v1 limitation, consistent with in-memory session state).

**Patterns to follow:**
- Existing `stop_flag` cancellation; existing progress event shape (add a `scheduled`/`countdown` status value).

**Test scenarios:**
- Happy path: schedule 2 minutes ahead → countdown renders → campaign fires on time and progresses normally.
- Edge case: cancel before fire → no blast request is pushed, UI returns to idle.
- Edge case: schedule time in the past → rejected with a clear error at arm time.
- Error path: account disconnected by fire time → the campaign fails with the standard login error path.

**Verification:**
- Schedule a 1-minute-out campaign to a test number; observe countdown, auto-fire, and normal completion.

---

## Phase 3 - Full OKESENDER Parity (U12-U19)

User-approved inclusion of the remaining OKESENDER surface. These land after U1-U11;
each is independently shippable. WPP API support must be verified against the live
injected bundle before each unit's JS work (the `getAll` vs `getAllGroups` incident).

- [ ] U12. **Blind mode toggle**

**Goal:** Skip per-message status verification for maximum send speed.

**Requirements:** R14

**Dependencies:** None

**Files:**
- Modify: `src/pages/sending.html` (mode select: Safe / Blind)
- Modify: `src-tauri/src/campaign/sender.rs` (blind path skips the chat-exists check and does not wait for send confirmation)

**Approach:** one mode selector next to Safe mode; blind mode logs sends without delivery status. OKESENDER semantics: "will send to all contacts, valid or invalid; blind mode will not show the status of the message."

**Test scenarios:**
- Happy path: blind blast completes faster than safe blast on the same list.
- Edge case: blind mode still respects pause/stop and human-mode delays.

**Verification:** timing comparison on a small list; no status column in blind logs.

---

- [ ] U13. **Voice note (PTT) + GIF attachments**

**Goal:** Attachment picker accepts audio-as-voice-note and GIFs, sent via WPP `sendFileMessage` with the correct message type.

**Requirements:** R15

**Dependencies:** U4 (attachment picker exists)

**Files:**
- Modify: `src/pages/sending.html` (accept filters + send-as-voice toggle for audio)
- Modify: `src-tauri/src/campaign/sender.rs` and attachment plumbing (ptt flag, gif detection)

**Approach:** audio files gain a "send as voice note" checkbox (PTT flag); `.gif` files route to the gif message type. Caption rules follow WhatsApp's own constraints.

**Test scenarios:**
- Happy path: mp3 sent as PTT renders as a playable voice note.
- Happy path: gif renders animated in the chat.
- Error path: unsupported file triggers a clear alert before any send.

**Verification:** manual send of both types to a test number.

---

- [ ] U14. **Import contacts from WhatsApp phonebook**

**Goal:** Pull the account's saved contacts into the blast list.

**Requirements:** R16

**Dependencies:** None

**Files:**
- Modify: `src-tauri/src/browser/js_injector.rs` (contact store read via WPP/Store)
- Modify: `src-tauri/src/main.rs` (`list_wa_contacts` + `import_wa_contacts` commands)
- Modify: `src/pages/contacts.html` ("Import from WhatsApp" button + selection table)

**Approach:** read the contact store (name + number), present a selectable table, import the checked ones through the normal contact pipeline (dedupe respected).

**Test scenarios:**
- Happy path: import 3 checked contacts, they appear in the list with names.
- Edge case: contacts without numbers are skipped.

**Verification:** import on the live account; count matches selection.

---

- [ ] U15. **Interactive buttons + list messages**

**Goal:** Send messages with interactive reply buttons or a list menu (OKESENDER `FrmInteactiveButtonsBuilder`, `WAPI.sendButtons`, `sendListMenu`).

**Requirements:** R17

**Dependencies:** U8 (message composition surface)

**Files:**
- Modify: `src/pages/sending.html` (interactive builder inside the compose panel)
- Modify: `src-tauri/src/browser/js_injector.rs` (`send_buttons`, `send_list_menu` via WPP equivalents)
- Modify: `src-tauri/src/campaign/sender.rs` (message type dispatch: text | buttons | list)

**Approach:** builder UI produces a JSON action payload stored with the campaign; sender picks the WPP call per type. **First step of the unit: capability probe** - verify the injected WPP bundle exposes the needed calls; if absent, surface a clear "not supported by current WPP build" error instead of shipping broken sends.

**Test scenarios:**
- Happy path: button message renders with clickable buttons in the receiving chat.
- Error path: WPP lacks the API, compose shows the capability warning at build time, send refuses cleanly.

**Verification:** manual send to a test number; buttons clickable and reply received.

---

- [ ] U16. **Catalog message builder**

**Goal:** Build and send catalog-style messages (OKESENDER `FrmCatalogBuilder`: title, description, items with thumbnails).

**Requirements:** R18

**Dependencies:** U15 (same message-type dispatch infrastructure)

**Files:**
- Modify: `src/pages/sending.html` (catalog editor panel or modal)
- Modify: `src-tauri/src/campaign/sender.rs` (catalog message type)
- Modify: `src-tauri/src/message/template_library.rs` (persist catalogs as templates)

**Approach:** catalogs saved as reusable JSON templates (`catalog_*.json` naming mirrors OKESENDER); send path uses the WPP catalog/product message API if available - same capability-probe rule as U15.

**Test scenarios:**
- Happy path: 2-item catalog renders as a product list message.
- Edge case: catalog with no items refuses to send.

**Verification:** manual send; items visible in the receiving chat.

---

- [ ] U17. **Multi-channel send**

**Goal:** One campaign distributed across several connected accounts (round-robin or split), multiplying daily volume.

**Requirements:** R19

**Dependencies:** U1-U2 (profiles are separate processes, so this operates on accounts within ONE window)

**Files:**
- Modify: `src/pages/sending.html` (account multi-select in Campaign Settings)
- Modify: `src-tauri/src/campaign/pipeline.rs` (campaign fan-out across accounts)
- Modify: `src-tauri/src/campaign/sender.rs` (per-account worker consuming a shared contact queue)

**Approach:** selected accounts each get a worker pulling from one shared queue; per-account progress events tagged with the account name; stop/pause affect all workers. Accounts must be CONNECTED to join the pool.

**Test scenarios:**
- Happy path: 20 contacts across 2 accounts, each account sends ~10, no duplicates.
- Edge case: one account disconnects mid-run, its share re-routes or fails visibly, the other continues.
- Edge case: all selected accounts must be connected at start, else reject.

**Verification:** two connected accounts, one campaign, both chats show sends, counters sum correctly.

---

- [ ] U18. **Number generator**

**Goal:** Generate candidate numbers in a prefix range for checking before blasting (OKESENDER `ButtonNumberGenerator`).

**Requirements:** R20

**Dependencies:** U9 (checker flow consumes the output)

**Files:**
- Modify: `src/pages/contacts.html` (generator panel: prefix + range + cap)

**Approach:** generate numeric suffixes under a prefix (e.g. 62812 + 0000-0099), cap the count, feed straight into the checker panel - never directly into the send list.

**Test scenarios:**
- Happy path: prefix 62812, range 0-99, produces 100 candidates in the checker input.
- Edge case: range cap enforced (no runaway generation).

**Verification:** generate, check, import valid - end to end.

---

- [ ] U19. **Familiar accounts (research-first)**

**Goal:** Understand and replicate OKESENDER's "familiar accounts" (`BtnAddFamiliarAccount`, `FrmAdvanced`).

**Requirements:** R21

**Dependencies:** None

**Files:**
- Create: `docs/plans/` research note (deep RE of FrmAdvanced logic and its WAPI calls)
- Modify: TBD by findings

**Approach:** OKESENDER gates this behind licensing and its semantics are not obvious from strings alone. This unit starts with a dedicated RE pass (decompile FrmAdvanced, trace its WAPI calls); implementation scope is decided from the findings. If it turns out to be license-server noise, the unit closes with that conclusion.

**Test scenarios:**
- Test expectation: none - research unit; outcome is a go/no-go follow-up.

**Verification:** a written RE conclusion and go/no-go decision.

---

## Backlog (remaining, intentionally deferred)

- Check for Update flow (BlastWA has its own WPP updater already; app self-update deferred)
- Received-message export (requires inbound message observation - candidate for the Engager research)

---

## System-Wide Impact

- **Interaction graph:** progress emission adds a per-send Tauri event; the existing frontend listener is the only consumer. REST `/api/status` unaffected (reads the same counters).
- **Error propagation:** profile resolution failures (bad name) fail fast at startup with a clear message; API port walk logs each retry.
- **State lifecycle risks:** two windows writing the SAME profile dir is user-error — mitigated by the launcher always passing a fresh/existing profile name and the title showing the active profile; no file locking added (documented limitation).
- **API surface parity:** REST API exists per profile instance; external consumers must target the profile's effective port (visible in that window's Settings page).
- **Unchanged invariants:** account persistence format, Chrome isolation flags, CDP architecture, WPP injection pipeline, group export — all untouched.

---

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Two windows opened on the same profile → concurrent writes to accounts.json/contacts | Window title shows the active profile; launcher flow steers to distinct profiles; accepted limitation for v1 |
| API port collision across profiles | Bind-failure port walk + persist effective port (U2) |
| Progress events flooding the webview on huge lists | One event per send; human-mode delays naturally throttle |
| Profile name misuse in filesystem | Sanitize to a safe segment at the U1 boundary; single validation point |

---

## Sources & References

- Reverse-engineered OKESENDER strings: `FrmGroupGrabber`, export dialogs, formatting toolbar buttons (`D:/Tes/OKESENDER.exe` UTF-16 extraction)
- Related code: `src-tauri/src/campaign/sender.rs` (`on_progress` hook), `src/pages/sending.html` (`campaign_progress` listener)
