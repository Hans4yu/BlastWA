---
title: "feat: BlastWA — WhatsApp Bulk Sender Rust Clone (No License, Full Differentiators)"
type: feat
status: active
date: 2026-01-01
origin: reverse-engineered from OKESENDER.exe (VB.NET 4.8 + WebView2)
---

# feat: BlastWA — WhatsApp Bulk Sender Rust Clone

## Overview

BlastWA adalah clone dari **OKESENDER.exe** yang dibangun ulang dari nol dengan **Rust**
(backend) + **Tauri v2** (UI), menggunakan **CDP + Chrome user** bukan WebView2/Edge Runtime.
Zero license check. Selain itu, BlastWA menambahkan **6 differentiator eksklusif** yang
tidak ada di OKESENDER: human-behavior anti-detection delay, local REST API/webhook mode,
reusable template library, CSV/Excel import dengan auto-column mapping, sent log export ke
CSV/Excel, dan built-in WPP.js auto-updater.

**Nama Produk:** BlastWA
**Logo:** kosong dulu (placeholder di assets/)
**Target OS:** Windows x64

---

## Problem Frame

OKESENDER.exe adalah paid tool (license via `https://okesender.aiopanels.com/api/CheckKey.ashx`)
yang kerjanya sederhana — buka `https://web.whatsapp.com` di embedded browser,
inject JavaScript (WAPI + WPP.js) ke dalam page, lalu dispatch pesan ke kontak/grup
berdasarkan campaign list. Kita reverse full behavior dari `#US` heap IL metadata,
rebuild logic-nya dalam Rust, dan pakai CEF (Chrome Embedded Framework via `cef-sys` atau
`chromiumoxide`) sebagai browser engine yang otomatis link ke Chrome user tanpa install Edge Runtime.

---

## Requirements Trace

- R1. Aplikasi harus bisa kirim pesan teks (plaintext + spintax `{a|b|c}`) ke list nomor
- R2. Aplikasi harus bisa kirim file (gambar, video, PDF, audio/PTT, dokumen)
- R3. Multi-account — bisa login multiple nomor WA sekaligus dengan tab browser per akun
- R4. Delay konfigurabel antara pesan (min–max seconds)
- R5. Variable substitution: `[[fullname]]`, `[[firstname]]`, `[[lastname]]`, `[[VAR1]]`–`[[VAR5]]`, `[[randomtag]]`
- R6. Contact number checker (WAPI.checkNumberStatus) sebelum blast
- R7. Group grabber — ambil semua member dari grup WA
- R8. Auto-reply rules (keyword match: Like / Start with / End with / Contains)
- R9. Campaign scheduler (start datetime, delay interval, sleep interval)
- R10. Browser engine otomatis ikut Chrome versi user via `setup.exe` detector — tidak butuh Edge Runtime
- R11. Zero license check — tidak ada validasi ke server eksternal manapun
- R12. Log panel per campaign (sent/failed/pending count, timestamp)
- R13. Import kontak dari .txt file (one number per line)
- R14. Spintax editor / preview
- R15. Save/load campaign profile (JSON)

**Differentiator (tidak ada di OKESENDER):**
- R16. Human Behavior Simulation Engine — simulasi perilaku manusia lengkap: per-account personality profile, burst-and-rest rhythm state machine, typing-time terkait panjang pesan, presence actions (mark read + typing indicator), adaptive exponential backoff saat ada sinyal warning WA, dan time-of-day modulation — bukan sekadar random delay
- R17. Webhook/API mode — expose local REST server (`127.0.0.1:PORT`) agar sistem eksternal bisa trigger blast via HTTP POST tanpa buka UI
- R18. Template library — simpan, edit, hapus, dan reuse message templates (teks + attachment) dengan nama dan tag
- R19. CSV/Excel import dengan auto-column mapping — detect header kolom secara otomatis, map ke `ContactRow` fields via UI dialog
- R20. Sent log export — export log campaign (sent/failed/pending + timestamp + nomor) ke file `.csv` atau `.xlsx`
- R21. Built-in WPP.js version updater — fetch versi terbaru `@wppconnect-team/wppconnect` dari GitHub releases, replace JS injection template tanpa rebuild app

---

## Scope Boundaries

- Logo / branding final → diisi belakangan (placeholder dulu)
- Auto-updater / crash reporter → di luar scope v1
- macOS / Linux support → tidak dalam scope (Windows x64 only)
- End-to-end enkripsi WhatsApp → tidak disentuh (kita riding di atas WA Web resmi)
- Sending via WA Business API (official) → bukan ini, kita pakai Web injection
- Fitur Catalog Builder dan Interactive Buttons Builder → defer ke v2 (complex UI, low priority)
- Webhook/API mode authentication (bearer token, API key) → defer ke v1.1 (v1 hanya localhost, tidak expose ke internet)
- Multi-language UI → defer ke v2
- Auto-updater untuk app itu sendiri → di luar scope v1

---

## Context & Research

### Reverse Engineering Findings (dari `OKESENDER.exe` IL metadata)

**Runtime original:** VB.NET 4.8, WinForms, WebView2 (Edge-based)

**JS Injection yang dipakai original (verbatim dari #US heap):**

```javascript
function sendMsg(id, message, isSafe) {
    var sendResult = {};
    sendResult.id = id;
    sendResult.message = message;
    sendResult.sentStatus = false;
    sendResult.result = {};
    if (isSafe == true) {
        WAPI.getchatId(id).then(e => {
            if (e !== undefined) {
                WAPI.sendMessage(id, message).then((e) => {
                    sendResult.sentStatus = true;
                    sendResult.result = e;
                    window.chrome.webview.postMessage(sendResult);
                });
            } else {
                window.chrome.webview.postMessage(sendResult);
            }
        });
    } else {
        WAPI.sendMessage(id, message).then((e) => {
            sendResult.sentStatus = !e.erro;
            sendResult.result = e;
            window.chrome.webview.postMessage(sendResult);
        });
    }
};
sendMsg('{id}', '{message}', {isSafe});
```

```javascript
function sendFile(base64, id, filename, caption, isSafe) {
    if (isSafe == true) {
        WAPI.getchatId(id).then(e => {
            if (e) {
                WPP.chat.sendFileMessage(id, base64, {
                    type: getFileType(filename),
                    caption: caption,
                    filename: filename,
                });
            }
        });
    } else {
        WPP.chat.sendFileMessage(id, base64, {
            type: getFileType(filename),
            caption: caption,
            filename: filename,
        });
    }
};
sendFile('{base64}', '{id}', '{filename}', '{caption}', '{isSafe}');
```

```javascript
function sendPTT(base64, id, isSafe) {
    if (isSafe == true) {
        WAPI.getchatId(id).then(e => {
            if (e) {
                WPP.chat.sendFileMessage(id, base64, { type: 'ppt', isPtt: true });
            }
        });
    } else {
        WPP.chat.sendFileMessage(id, base64, { type: 'ppt', isPtt: true });
    }
};
sendPTT('{base64}', '{id}');
```

```javascript
WAPI.checkNumberStatus('{0}').then(e => {
    e.numtoCheck = '{1}';
    window.chrome.webview.postMessage(e);
});
```

```javascript
var t = [];
for (let c of WAPI.getAllGroups()) {
    t.push({ id: c.id._serialized, name: c.name });
}
t;
```

```javascript
WPP.conn.getMyUserId().user
WAPI.isLoggedIn()
WAPI.isConnected()
WPP.whatsapp.ContactStore.toJSON();
WPP.group.getParticipants('{groupId}');
```

**IPC channel:**
- Original: `window.chrome.webview.postMessage(result)` → captured di .NET via `WebMessageReceived` event
- Clone Rust: kita pakai CEF `CefMessageRouter` atau custom JS→Rust bridge via `cef_string` callback

**Variable substitution pattern:**
```
[[fullname]] [[firstname]] [[lastname]] [[VAR1]] [[VAR2]] [[VAR3]] [[VAR4]] [[VAR5]] [[randomtag]]
```

**Spintax pattern:** `{option1|option2|option3}` — recursive expansion

**File paths (data dir):**
```
%APPDATA%\BlastWA\Profiles\
%APPDATA%\BlastWA\accounts\
%APPDATA%\BlastWA\Data\
%APPDATA%\BlastWA\Reports\
%APPDATA%\BlastWA\Buttons\
%APPDATA%\BlastWA\catalogs\
```

**Config files:** `autoreply.json`, `rules.json`, `channels.json`, `commonList.data`, `commonMessage.data`

**Campaign config keys dari original:**
```
SendingConfig, ActivateDialog, DialogAfter, DialoCount, DialogWait
ActivateSleep, SleepAfter, SleepFor, DelayStart, DelayEnd
```

### Tech Stack Rust (Pilihan)

**GUI Framework:**
- **Tauri v2** (recommended) — Rust backend + webview frontend (HTML/CSS/JS untuk UI), pakai CEF atau system webview
- Alternatif: `egui` untuk full-native Rust GUI (lebih ringan tapi kurang fleksibel)
- Kita pilih **Tauri v2** karena: browser control lebih mudah di-embed, UI lebih rich, IPC sudah ada

**Browser Engine untuk WA Web:**
- **`chromiumoxide`** crate — Rust CDP (Chrome DevTools Protocol) client yang connect ke Chrome instance
- Cara kerja: `setup.exe` detect Chrome path, launch Chrome dengan `--remote-debugging-port`, `chromiumoxide` connect ke sana
- Alternative: embed CEF via `cef-sys` bindgen (lebih complex, butuh cef binary)
- **Pilihan: `chromiumoxide` + user's Chrome** — paling clean, no extra runtime

**HTTP (untuk request check nomor dsb):**
- `reqwest` + `tokio` (async runtime)

**Serialization:**
- `serde_json` untuk semua config file

**IPC (Tauri):**
- `tauri::command` untuk expose Rust function ke JS frontend
- `tauri::Event` untuk push hasil send ke frontend

### Crate Dependencies

```toml
[dependencies]
tauri = { version = "2", features = ["shell-open", "dialog"] }
chromiumoxide = { version = "0.7", features = ["tokio-runtime"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
reqwest = { version = "0.12", features = ["json", "multipart"] }
rand = "0.8"
rand_distr = "0.4"         # Gaussian/Poisson distribusi untuk anti-detection jitter
regex = "1"
base64 = "0.22"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
anyhow = "1"
thiserror = "1"
log = "0.4"
env_logger = "0.11"
winreg = "0.52"            # detect Chrome registry path
axum = "0.7"               # local REST server untuk webhook/API mode
tower = "0.4"
csv = "1.3"                # CSV import/export
calamine = "0.24"          # Excel (.xlsx) reader untuk import
xlsxwriter = "0.6"         # Excel (.xlsx) writer untuk export sent log
octorust = "0.0.5"         # GitHub releases API untuk WPP.js updater (atau raw reqwest)
```

---

## Key Technical Decisions

- **Tauri v2 bukan Electron**: Rust native backend, biner lebih kecil, tidak perlu bundle Node.js
- **`chromiumoxide` bukan CEF embed**: Pakai Chrome user via CDP — tidak perlu distribusi binary CEF 100MB+, user sudah punya Chrome
- **CDP injection bukan WebView2 IPC**: `chromiumoxide` bisa `Page.evaluate()` + tunggu result via CDP event, setara dengan `window.chrome.webview.postMessage`
- **WAPI + WPP.js injection**: inject via `Page.evaluate()` di CDP — sama persis behavior dengan original
- **`setup.exe` = Rust binary kecil** yang detect Chrome path dari Windows Registry → tulis ke config → launch main app
- **Async Tokio**: seluruh sending loop, checker, group grabber → async task, cancellable via `CancellationToken`
- **Profile = direktori + JSON files**: satu profile = satu folder ber-nama di `%APPDATA%\BlastWA\Profiles\`
- **Spintax + variable substitution**: pure Rust, regex-based, tidak ada eval
- **Anti-detection jitter**: distribusi Gaussian (`rand_distr::Normal`) dengan mean = midpoint delay dan stddev = (max-min)/4 — bukan flat uniform. Hasil: pola delay lebih mirip manusia, lebih sulit dideteksi oleh WA rate-limit heuristics
- **Local REST API via `axum`**: spawn `tokio` task terpisah, listen di `127.0.0.1:{port}` — tidak expose ke network interface lain, zero auth di v1 (localhost-only = inherently safe)
- **CSV/Excel import**: `calamine` untuk read `.xlsx`, `csv` crate untuk `.csv` — header detection via row pertama, fuzzy match ke field `ContactRow` (case-insensitive, strip whitespace)
- **Export sent log**: `xlsxwriter` untuk `.xlsx`, `csv` crate untuk `.csv` — format kolom: timestamp, number, fullname, status, error_reason
- **WPP.js updater**: `reqwest` hit GitHub API `https://api.github.com/repos/wppconnect-team/wppconnect/releases/latest`, parse tag, download JS bundle, replace di `%APPDATA%\BlastWA\wpp\wpp.js`

---

## Open Questions

### Resolved During Planning

- **WebView2 vs CEF vs `chromiumoxide`?** → `chromiumoxide` + user Chrome via CDP. Alasan: zero extra runtime, user pasti sudah punya Chrome, CDP sudah stable
- **GUI framework?** → Tauri v2. Trade-off: butuh Node untuk build frontend UI, tapi UI freedom jauh lebih besar dari egui
- **License removal?** → Semua panggilan ke `https://okesender.aiopanels.com/api/CheckKey.ashx` tidak diimplementasikan. `FrmLicense` tidak dibuat.
- **WAPI vs WPP.js?** → Pakai keduanya, sama dengan original. WAPI untuk legacy calls, WPP untuk file/PTT

### Deferred to Implementation

- Versi minimum Chrome yang supported (test CDP compatibility, target Chrome 115+)
- Apakah `chromiumoxide` bisa handle multiple Chrome instances untuk multi-account (set `--user-data-dir` per akun)
- Exact CDP `Page.evaluate()` return type mapping untuk setiap JS injection
- Rate limit WhatsApp Web (berapa delay minimum sebelum WA ban) — harus dikalibrasi manual saat runtime testing

---

## Output Structure

```
blastwa/
├── Cargo.toml
├── Cargo.lock
├── setup/
│   ├── Cargo.toml
│   └── src/
│       └── main.rs                  (Chrome detector + launcher)
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── build.rs
│   ├── icons/
│   │   └── placeholder.png
│   └── src/
│       ├── main.rs                  (Tauri app entry)
│       ├── lib.rs
│       ├── browser/
│       │   ├── mod.rs
│       │   ├── chrome_detect.rs     (Windows Registry Chrome path finder)
│       │   ├── cdp_client.rs        (chromiumoxide wrapper)
│       │   └── js_injector.rs       (WAPI/WPP JS template injector)
│       ├── campaign/
│       │   ├── mod.rs
│       │   ├── sender.rs            (blast loop, delay, cancellation)
│       │   ├── checker.rs           (number validity checker)
│       │   ├── group_grabber.rs     (WPP group participant extractor)
│       │   └── scheduler.rs        (datetime scheduler)
│       ├── message/
│       │   ├── mod.rs
│       │   ├── spintax.rs           (spintax expander)
│       │   ├── variables.rs         (template variable substitution)
│       │   └── attachment.rs        (file→base64 encoder)
│       ├── account/
│       │   ├── mod.rs
│       │   └── profile.rs           (Profile CRUD, per-profile Chrome session)
│       ├── autoreply/
│       │   ├── mod.rs
│       │   └── rules.rs             (rule engine: Like/StartWith/EndWith/Contains)
│       ├── api/
│       │   ├── mod.rs
│       │   └── server.rs            (axum local REST server, webhook mode)
│       ├── updater/
│       │   ├── mod.rs
│       │   └── wpp_updater.rs       (GitHub API fetch + WPP.js hot-swap)
│       └── config/
│           ├── mod.rs
│           └── settings.rs          (app config, data dir, JSON r/w)
├── src/                             (Tauri frontend — HTML/CSS/JS)
│   ├── index.html
│   ├── main.js
│   ├── styles.css
│   └── pages/
│       ├── dashboard.html
│       ├── accounts.html
│       ├── sending.html
│       ├── contacts.html
│       ├── templates.html           (template library)
│       ├── autoreply.html
│       ├── groups.html
│       ├── log.html
│       ├── api_settings.html        (webhook/API mode config)
│       └── settings.html
└── docs/
    └── plans/
        └── (ini file ini)
```

---

## High-Level Technical Design

> *Ini adalah directional guidance untuk review, bukan implementation specification.*

### Arsitektur Utama

```
setup.exe (Rust binary)
  │
  ├── detect Chrome path via Windows Registry
  │   HKLM\SOFTWARE\Google\Chrome\BLBeacon "version"
  │   HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe
  │
  ├── tulis chrome_path ke %APPDATA%\BlastWA\config.json
  └── launch blastwa.exe

blastwa.exe (Tauri app)
  │
  ├── Tauri Frontend (HTML/JS, port 1430)
  │   ├── Dashboard (status per akun)
  │   ├── Sending Panel (compose + blast)
  │   ├── Contacts Panel (import .txt, VAR columns)
  │   ├── Accounts Panel (add/remove Chrome sessions)
  │   ├── AutoReply Panel (rules builder)
  │   ├── Group Grabber
  │   └── Log Panel
  │
  └── Tauri Backend (Rust)
      │
      ├── AccountManager
      │   ├── launch Chrome --remote-debugging-port=XXXX --user-data-dir=%APPDATA%\BlastWA\accounts\{name}\
      │   └── chromiumoxide::Browser::connect("localhost:XXXX")
      │
      ├── JsInjector
      │   ├── inject WAPI loader ke WA Web page
      │   ├── Page.evaluate(sendMsg_template.replace(vars))
      │   └── listen CDP event untuk postMessage result
      │
      ├── CampaignSender (tokio::task)
      │   ├── iterate contact list
      │   ├── apply spintax + variable substitution per message
      │   ├── call JsInjector.send_message() / send_file()
      │   ├── [NEW] HumanBehaviorEngine: personality + burst/rest + typing sim + backoff
      │   ├── setiap N pesan → dialog delay (Familiar sending)
      │   ├── setiap M pesan → sleep interval
      │   └── emit progress event ke frontend via tauri::emit
      │
      ├── AutoReplyWatcher (tokio::task)
      │   ├── poll WAPI.getAllChatsWithNewMsg() via CDP
      │   ├── match rule (keyword engine)
      │   └── call JsInjector.send_message() untuk reply
      │
      ├── [NEW] LocalApiServer (axum, tokio::task)
      │   ├── POST /api/blast  → trigger campaign via HTTP
      │   ├── GET  /api/status → status campaign aktif
      │   └── POST /api/stop   → stop campaign
      │
      └── [NEW] WppUpdater
          ├── GET github releases API → parse latest tag
          ├── download wpp.js bundle
          └── hot-swap di %APPDATA%\BlastWA\wpp\wpp.js
```

### IPC Flow (Rust ↔ JS dalam WA Web page)

```
Rust backend
  → chromiumoxide: Page.evaluate("sendMsg('{id}','{message}',{isSafe})")
  → Chrome CDP Runtime.evaluate
  → WA Web JS runtime executes WAPI.sendMessage()
  → window.chrome.webview.postMessage(result)   ← di browser page
  → CDP Network.webSocketFrameReceived          ← chromiumoxide intercept
  → Rust future resolves dengan result JSON
  → tauri::emit("send_result", result)
  → Tauri frontend JS receives event
  → update log panel
```

---

## Implementation Units

- [ ] U1. **Setup Binary — Chrome Detector & Launcher**

**Goal:** `setup/src/main.rs` — Rust binary kecil yang detect path Chrome dari Windows Registry,
validasi versi minimum (Chrome 115+), tulis ke config, lalu launch `blastwa.exe`

**Requirements:** R10

**Dependencies:** None

**Files:**
- Create: `setup/Cargo.toml`
- Create: `setup/src/main.rs`
- Create: `src-tauri/src/browser/chrome_detect.rs`

**Approach:**
- Query `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe` via `winreg` crate
- Fallback: scan `%LOCALAPPDATA%\Google\Chrome\Application\chrome.exe`
- Parse versi dari `HKLM\SOFTWARE\Google\Chrome\BLBeacon` key "version"
- Tulis hasil ke `%APPDATA%\BlastWA\config.json` field `chrome_path` + `chrome_version`
- Launch blastwa.exe via `std::process::Command`
- Jika Chrome tidak ditemukan: tampil dialog error native Windows (`MessageBoxW` via `winapi`) lalu exit

**Test scenarios:**
- Happy path: Chrome ada di default path → config ditulis, blastwa.exe dilaunched
- Edge case: Chrome ada tapi versi < 115 → tampil warning tapi tetap lanjut (log warning)
- Error path: Chrome tidak terinstall sama sekali → MessageBoxW error, exit code 1
- Edge case: `%APPDATA%\BlastWA\` belum ada → auto-create directory

**Verification:**
- Setup binary launch tanpa panic di mesin dengan Chrome 115+
- `config.json` terisi field `chrome_path` dengan path valid
- blastwa.exe terlaunched setelah setup

---

- [ ] U2. **Config & Data Directory Manager**

**Goal:** `src-tauri/src/config/settings.rs` — semua path management, read/write JSON config,
init data directories sesuai struktur original

**Requirements:** R11, R15

**Dependencies:** U1

**Files:**
- Create: `src-tauri/src/config/mod.rs`
- Create: `src-tauri/src/config/settings.rs`

**Approach:**
- `AppConfig` struct: `chrome_path`, `default_delay_min`, `default_delay_max`, `active_profile`
- `DataPaths` struct: semua path direktori (Profiles, accounts, Data, Reports, Buttons)
- `init_data_dirs()` → create semua folder jika belum ada
- `load_config()` / `save_config()` → serde_json ke `%APPDATA%\BlastWA\config.json`
- Zero hardcoded absolute path — semua relative ke `dirs::data_dir() + "BlastWA"`

**Test scenarios:**
- Happy path: first run → semua direktori berhasil dibuat
- Happy path: config dibaca dan di-deserialize dengan benar
- Edge case: config.json corrupt → fallback ke default config, log warning
- Edge case: permission denied on %APPDATA% → return `Err(anyhow!(...))`

**Verification:**
- Semua direktori terbuat di `%APPDATA%\BlastWA\` pada first run
- Config persist across restart

---

- [ ] U3. **Chrome CDP Client & Account Session Manager**

**Goal:** `src-tauri/src/browser/cdp_client.rs` + `account/profile.rs` —
launch Chrome per akun dengan isolated `--user-data-dir`, connect via `chromiumoxide`,
manage multiple concurrent browser instances

**Requirements:** R3, R10, R11

**Dependencies:** U1, U2

**Files:**
- Create: `src-tauri/src/browser/mod.rs`
- Create: `src-tauri/src/browser/cdp_client.rs`
- Create: `src-tauri/src/account/mod.rs`
- Create: `src-tauri/src/account/profile.rs`

**Approach:**
- `AccountSession` struct: `name`, `port`, `chrome_pid`, `browser: Arc<Browser>`, `page: Arc<Page>`
- Launch Chrome:
  ```
  chrome.exe --remote-debugging-port={port} --user-data-dir={accounts_dir}/{name}/ --no-first-run --disable-background-timer-throttling
  ```
- Port assignment: start dari 9222, increment per akun
- `chromiumoxide::Browser::connect("http://localhost:{port}")` dengan retry 5x (Chrome butuh ~2 detik startup)
- Navigate ke `https://web.whatsapp.com`
- Simpan `AccountSession` di `Arc<Mutex<HashMap<String, AccountSession>>>`
- `add_account(name)` / `remove_account(name)` / `list_accounts()` sebagai Tauri commands

**Test scenarios:**
- Happy path: akun baru dibuat, Chrome launched, CDP connected, WA Web loaded
- Happy path: 3 akun concurrent berjalan di port berbeda tanpa conflict
- Error path: port sudah occupied → increment ke port berikutnya otomatis
- Edge case: Chrome crash mid-session → `AccountSession` di-mark sebagai disconnected, auto-reconnect attempt
- Error path: WA Web gagal load (no internet) → error di-emit ke frontend

**Verification:**
- Bisa launch 3 akun bersamaan
- Setiap akun punya user-data-dir terpisah (WA QR scan masing-masing)
- CDP connected setiap akun

---

- [ ] U4. **JS Injector — WAPI/WPP Bridge**

**Goal:** `src-tauri/src/browser/js_injector.rs` — semua JS injection ke WA Web page
via CDP `Page.evaluate()`, mapping 1:1 dengan original VB.NET behavior

**Requirements:** R1, R2, R6, R7

**Dependencies:** U3

**Files:**
- Create: `src-tauri/src/browser/js_injector.rs`

**Approach:**
- `JsInjector` struct wraps `Arc<Page>`
- Template strings disimpan sebagai Rust `const &str` — verbatim dari reverse engineering
- `send_message(id: &str, message: &str, is_safe: bool)` → inject `sendMsg` template → await CDP result
- `send_file(base64: &str, id: &str, filename: &str, caption: &str, is_safe: bool)` → inject `sendFile`
- `send_ptt(base64: &str, id: &str)` → inject `sendPTT`
- `check_number(number: &str)` → inject `WAPI.checkNumberStatus()`
- `get_all_groups()` → inject group list JS → parse JSON result
- `get_participants(group_id: &str)` → inject `WPP.group.getParticipants()`
- `is_logged_in()` → inject `WAPI.isLoggedIn()`
- Result parsing: CDP returns `RemoteObject` → extract `value` field → `serde_json::from_value`
- `isSafe` mode: verify nomor exists dulu via `WAPI.getchatId()` sebelum send

**Test scenarios:**
- Happy path: `send_message("6281234567890@c.us", "Hello", true)` → CDP result `sentStatus: true`
- Happy path: `send_file(base64_jpg, "6281..@c.us", "photo.jpg", "caption", false)` → success
- Error path: nomor tidak ada di WA → `sentStatus: false`, tidak crash
- Edge case: message mengandung single quote → di-escape sebelum inject ke JS template
- Edge case: `is_logged_in()` → false ketika belum scan QR → return `Err`
- Happy path: `get_all_groups()` → return `Vec<{id, name}>`

**Verification:**
- Pesan nyata terkirim ke nomor test via WA Web
- File terkirim dengan caption
- Number check return status benar

---

- [ ] U5. **Spintax Engine & Variable Substitution**

**Goal:** `src-tauri/src/message/spintax.rs` + `variables.rs` — pure Rust string
processing, recursive spintax expansion, variable mapping dari contact row

**Requirements:** R1, R5

**Dependencies:** None (standalone module)

**Files:**
- Create: `src-tauri/src/message/mod.rs`
- Create: `src-tauri/src/message/spintax.rs`
- Create: `src-tauri/src/message/variables.rs`
- Test: `src-tauri/src/message/spintax.rs` (unit tests inline)

**Approach:**
- `fn spin(text: &str) -> String` — regex `\{[^{}]*\}` find → random pick dari `|` split
  loop sampai tidak ada lagi `{...}` (handle nesting)
- `fn apply_variables(template: &str, contact: &ContactRow) -> String`
  replace `[[fullname]]` → `contact.fullname`, dst
- `ContactRow` struct: `number`, `fullname`, `firstname`, `lastname`, `var1`..`var5`
- `[[randomtag]]` → `rand::thread_rng().gen_range(1000..9999).to_string()`
- JS-escape function: escape `'` → `\'` dan `\n` → `\\n` sebelum inject ke JS template

**Patterns to follow:**
- Standard Rust recursive regex pattern via `regex::Regex::new(r"\{[^{}]*\}")` + loop

**Test scenarios:**
- Happy path: `spin("{Hello|Hi} {world|there}")` → salah satu dari 4 kombinasi
- Happy path: nested `spin("{a|{b|c}}")` → expand benar
- Edge case: `spin("no braces")` → return unchanged
- Edge case: `spin("{only_one}")` → return "only_one"
- Happy path: `apply_variables("Dear [[firstname]]", contact)` → "Dear Budi"
- Edge case: `[[VAR3]]` tidak ada di contact (empty string) → replace dengan ""
- Edge case: message mengandung `'` dan `"` → JS injection tidak break

**Verification:**
- 100 kali `spin()` pada template yang sama → tidak ada panic, berbagai output
- Variable substitution benar untuk semua 8 placeholder

---

- [ ] U6. **Contact List Manager & .txt Importer**

**Goal:** `src-tauri/src/campaign/mod.rs` — import nomor dari .txt (one per line),
parse kolom tambahan (VAR1-VAR5 dengan separator), CRUD contact list di campaign

**Requirements:** R1, R5, R13

**Dependencies:** U5

**Files:**
- Create: `src-tauri/src/campaign/mod.rs`
- Create: `src-tauri/src/campaign/contact_list.rs`
- Test: `src-tauri/src/campaign/contact_list.rs` (unit tests inline)

**Approach:**
- Format .txt: `nomor|fullname|var1|var2|var3|var4|var5` (pipe-separated, kolom opsional)
- `ContactRow` parsing: split by `|`, index positional, empty string untuk kolom yang tidak ada
- Generate `firstname` dan `lastname` dari `fullname` (split spasi pertama)
- `ContactList` struct: `Vec<ContactRow>`, load/save ke JSON
- `filter_duplicates()` → deduplicate by `number` field
- Normalize nomor: strip `+`, `(`, `)`, `-`, spasi → pure digits, tambah country code jika < 10 digit

**Test scenarios:**
- Happy path: import 100 baris nomor valid → `ContactList` berisi 100 entry
- Edge case: baris kosong → di-skip
- Edge case: nomor duplikat → satu entry saja setelah `filter_duplicates()`
- Edge case: nomor format `+62 812-3456-7890` → normalized jadi `6281234567890`
- Happy path: kolom VAR5 isi → `contact.var5` terisi; kolom tidak ada → empty string
- Error path: file encoding tidak valid UTF-8 → error di-return, tidak crash

**Verification:**
- Import 1000 nomor dari .txt dalam < 100ms
- Duplikat otomatis dihapus

---

- [ ] U7. **Campaign Sender — Blast Loop**

**Goal:** `src-tauri/src/campaign/sender.rs` — async tokio task yang iterate contact list,
apply message template, call JS injector, handle delay/dialog/sleep logic,
emit progress ke Tauri frontend, cancellable

**Requirements:** R1, R2, R4, R9, R12

**Dependencies:** U4, U5, U6

**Files:**
- Create: `src-tauri/src/campaign/sender.rs`
- Create: `src-tauri/src/campaign/scheduler.rs`

**Approach:**
- `CampaignConfig` struct: semua field dari original `SendingConfig`
  (`delay_min`, `delay_max`, `activate_dialog`, `dialog_after`, `dialog_count`,
  `dialog_wait`, `activate_sleep`, `sleep_after`, `sleep_for`, `delay_start`, `delay_end`)
- `CampaignSender::run(config, contacts, injector, cancel_token)` → async loop
- Per iterasi:
  1. Check `cancel_token.is_cancelled()` → break
  2. Apply `spintax::spin()` + `variables::apply_variables()`
  3. Call `injector.send_message()` atau `send_file()`
  4. Log result via `tauri::emit("campaign_progress", progress_event)`
  5. Delay `rand::thread_rng().gen_range(delay_min..=delay_max)` detik via `tokio::time::sleep`
  6. Jika `dialog_after` count tercapai → sleep `dialog_wait` detik (Familiar mode)
  7. Jika `sleep_after` count tercapai → sleep `sleep_for` menit

- `CampaignProgress` event: `{ sent, failed, pending, current_number, status }`
- Campaign bisa di-pause (suspend `cancel_token` sementara) dan di-resume
- Scheduler: `schedule_at: Option<DateTime>` → `tokio::time::sleep_until(schedule_at)` sebelum loop

**Test scenarios:**
- Happy path: campaign 10 kontak, delay 1-2s → selesai dalam 10-20s, semua terkirim
- Happy path: pause di tengah → loop berhenti, resume → lanjut dari posisi terakhir
- Happy path: cancel → goroutine bersih berhenti, tidak ada pesan gantung
- Edge case: `dialog_after = 0` (disabled) → tidak ada dialog pause
- Edge case: satu nomor gagal (is_safe=true, nomor tidak ada) → log failed, lanjut ke berikutnya
- Happy path: scheduler start 5 menit dari sekarang → campaign mulai tepat waktu

**Verification:**
- 10 pesan terkirim ke 10 nomor berbeda dalam satu kampanye
- Cancel token bekerja bersih tanpa goroutine leak

---

- [ ] U8. **Number Checker**

**Goal:** `src-tauri/src/campaign/checker.rs` — async checker yang validate apakah nomor
punya WA aktif sebelum blast, menggunakan `WAPI.checkNumberStatus()` via CDP

**Requirements:** R6

**Dependencies:** U4, U6

**Files:**
- Create: `src-tauri/src/campaign/checker.rs`

**Approach:**
- `CheckResult` struct: `number`, `status` (Business/Regular/Not Found), `exists: bool`
- Batch check: iterate contact list, inject `WAPI.checkNumberStatus()` per nomor
- Delay antar check: 500ms-1s (hindari trigger WA rate limit)
- Emit progress event per nomor yang di-check
- Return `Vec<CheckResult>` → filter contact list (hapus "Not Found")

**Test scenarios:**
- Happy path: nomor valid dengan WA → `exists: true`, status "Regular" atau "Business"
- Happy path: nomor tidak ada WA → `exists: false`, status "Not Found"
- Error path: WA Web belum logged in → return `Err("not logged in")`
- Edge case: 100 nomor batch → progress event emit setiap nomor

**Verification:**
- Checker hasil akurat untuk nomor test (satu yang ada WA, satu yang tidak)

---

- [ ] U9. **Group Grabber**

**Goal:** `src-tauri/src/campaign/group_grabber.rs` — ambil semua grup WA yang
di-join oleh akun, lalu ambil participant list dari grup yang dipilih

**Requirements:** R7

**Dependencies:** U4

**Files:**
- Create: `src-tauri/src/campaign/group_grabber.rs`

**Approach:**
- `get_all_groups()` → inject JS `var t=[];for(let c of WAPI.getAllGroups()){t.push({id:c.id._serialized,name:c.name})};t;`
  → parse JSON array → `Vec<Group>`
- `get_group_participants(group_id: &str)` → inject `WPP.group.getParticipants('{group_id}')`
  → parse → `Vec<String>` (nomor participant)
- Export participant list ke format `ContactList` compatible

**Test scenarios:**
- Happy path: akun join 5 grup → `get_all_groups()` return 5 entry
- Happy path: grab participants dari satu grup 100 orang → `Vec<String>` 100 item
- Error path: grup private / kick → CDP inject error → return `Err`
- Edge case: grup kosong → return empty vec, tidak crash

**Verification:**
- Participant list dari grup bisa langsung di-import ke campaign contact list

---

- [ ] U10. **Auto-Reply Rule Engine**

**Goal:** `src-tauri/src/autoreply/rules.rs` + watcher task — rule-based auto responder
yang poll incoming messages dan reply sesuai rules

**Requirements:** R8

**Dependencies:** U4, U5

**Files:**
- Create: `src-tauri/src/autoreply/mod.rs`
- Create: `src-tauri/src/autoreply/rules.rs`

**Approach:**
- `Rule` struct: `keyword`, `match_type` (Like/StartWith/EndWith/Contains), `reply_message: Option<String>`, `reply_file: Option<String>`
- `RuleEngine::match_rule(message: &str, rules: &[Rule]) -> Option<&Rule>`
- `AutoReplyWatcher` task:
  - Poll `WAPI.getAllChatsWithNewMsg()` setiap 3 detik
  - Per message yang masuk → `match_rule()` → jika match → `injector.send_message()`
  - Track processed message IDs untuk hindari double-reply
- Save/load rules ke `autoreply.json`

**Test scenarios:**
- Happy path: rule "Contains: promo" → pesan "ada promo?" → match → auto-reply dikirim
- Happy path: rule "StartWith: halo" → pesan "halo bos" → match
- Edge case: "Like: halo bos" → exact match, tidak match "halo sob"
- Edge case: dua rules match → pakai rule pertama yang match
- Error path: inject reply gagal → log error, watcher tetap jalan

**Verification:**
- Auto-reply terkirim dalam < 5 detik setelah pesan masuk

---

- [ ] U11. **Tauri Frontend — Dashboard & Sending Panel**

**Goal:** `src/pages/` HTML/JS/CSS untuk UI — Dashboard (status per akun),
Sending Panel (compose message + attachment + blast controls), Log Panel

**Requirements:** R1, R2, R3, R4, R12

**Dependencies:** U3, U7, U8

**Files:**
- Create: `src/index.html`
- Create: `src/main.js`
- Create: `src/styles.css`
- Create: `src/pages/dashboard.html`
- Create: `src/pages/sending.html`
- Create: `src/pages/log.html`

**Approach:**
- Vanilla JS + CSS (tidak pakai React/Vue — sesuai OKESENDER original yang WinForms / minimal)
- Left sidebar: navigasi antar panel (Accounts, Sending, Contacts, Groups, AutoReply, Log, Settings)
- Dashboard: grid kartu per akun → status (QR / Connected / Disconnected), nomor, tombol open browser
- Sending Panel:
  - Textarea untuk compose message (support multiline)
  - Attachment button (open file dialog via Tauri `dialog::open()`)
  - Spintax preview button
  - Delay range input (min/max seconds)
  - Contact list display (import .txt)
  - Start / Pause / Stop campaign buttons
  - Progress bar + counter (sent / failed / pending)
- Log Panel: scrollable list view, timestamp + nomor + status per entry
- Tauri IPC: semua action lewat `window.__TAURI__.invoke('command_name', args)`
- Listen events: `window.__TAURI__.event.listen('campaign_progress', handler)`

**Test scenarios:**
- Happy path: klik Start → progress bar update setiap pesan terkirim
- Happy path: klik Stop → campaign berhenti
- Happy path: import .txt → kontak muncul di list
- Edge case: compose message kosong → Start button disabled
- Happy path: attachment jpg dipilih → preview thumbnail muncul

**Verification:**
- UI bisa launch, navigasi semua panel berfungsi
- Campaign berjalan dan log update real-time

---

- [ ] U13. **Human Behavior Simulation Engine (Advanced Anti-Detection)**

**Goal:** `src-tauri/src/campaign/human_behavior.rs` — engine simulasi perilaku manusia
yang lengkap: delay Gaussian per-account "personality", burst-and-rest rhythm state machine,
typing-time simulation terkait panjang pesan, presence actions (mark read + typing indicator
sebelum kirim), adaptive backoff saat ada tanda warning dari WA, dan time-of-day modulation.
Jauh di atas sekadar `rand(min,max)` — ini membuat pola blast secara statistik mirip aktivitas
customer-service manusia asli

**Requirements:** R16

**Dependencies:** U7, U4

**Files:**
- Create: `src-tauri/src/campaign/human_behavior.rs`
- Modify: `src-tauri/src/campaign/sender.rs`
- Modify: `src-tauri/src/browser/js_injector.rs` (typing indicator + mark read helpers)
- Test: `src-tauri/src/campaign/human_behavior.rs` (unit tests inline)

**Approach:**

1. **Per-Account Personality Profile** — setiap akun punya "karakter" stabil:
   - Seeded RNG dari hash(account_name) → parameter deterministik per akun
   - Params: `base_speed` (0.7x–1.4x multiplier), `burst_len_min/max`, `rest_freq`, `typing_wpm` (35–60)
   - Akun A selalu agak lambat-typing, akun B cepat — konsisten sepanjang waktu
   - Disimpan di `%APPDATA%\BlastWA\personalities.json`, bisa di-regenerate manual

2. **Burst-and-Rest State Machine** — manusia tidak mengirim dengan tempo konstan:
   - State `Active`: kirim burst 5–15 pesan dengan delay pendek (2–8 detik)
   - State `Resting`: jeda panjang 30–180 detik setelah burst selesai
   - Transisi probabilitas dari personality profile (bukan fixed count)
   - Implementasi: enum state + weighted transition tiap iterasi

3. **Typing-Time Simulation** — delay proporsional dengan panjang pesan:
   - Hitung jumlah kata → `words * (60000 / wpm) ms` + Gaussian jitter ±20%
   - Pesan "Halo" ≠ delay-nya dengan pesan 3 paragraf — korelasi nyata
   - Selama "typing", trigger typing indicator di WA Web (lihat #5)

4. **Presence Actions sebelum kirim** — via JsInjector:
   - `WAPI.markSeen(chatId)` sebelum "mengetik"
   - `WAPI.sendTypingState(chatId)` → tunggulah typing duration → baru kirim
   - `WAPI.sendChatlistClearUnread()` optional untuk chat yang sudah ada
   - Ini meniru alur manusia: buka chat → baca → ketik → kirim

5. **Adaptive Backoff** — dengarkan sinyal bahaya:
   - Monitor hasil inject: error rate naik / response lambat / warning text dari WA Web
   - Jika terdeteksi: kalikan delay berikutnya x2, x4, x8 (exponential), max 15 menit
   - Setelah N pesan sukses berturut-turut, decay perlahan ke normal
   - Emit event `backoff_engaged` ke UI supaya user tahu kenapa campaign melambat

6. **Time-of-Day Modulation** — kurva aktivitas per jam:
   - Lookup table 24 nilai multiplier (misal jam 02:00 = 1.8x lebih lambat, jam 10:00 = 0.9x)
   - Default curve disediakan, bisa di-custom per profile
   - Opsional toggle "Night mode off" — stop otomatis jam tertentu

7. **Send-Order Jitter (opsional)** — antar-window shuffle:
   - Ambil window 20 kontak berikutnya, shuffle urutan kirim dalam window itu
   - Tujuan: hindari pola pengiriman berurutan persis sesuai import order
   - Toggle di UI, default ON untuk blast besar (>50 kontak)

- UI: panel "Human Mode" di Sending Panel dengan preset:
  - `Off` (flat uniform, seperti OKESENDER)
  - `Natural` (semua fitur ON, default)
  - `Cautious` (burst lebih pendek, rest lebih panjang, backoff lebih sensitif)
  - `Custom` (manual override semua param)

**Patterns to follow:**
- `rand_distr::Normal::new(mean, std_dev)` untuk jitter
- `rand::rngs::StdRng::seed_from_u64(hash(account_name))` untuk personality stability
- Enum + match untuk state machine, bukan string states

**Test scenarios:**
- Happy path: 1000 sample delay → cluster di sekitar mean personality, tidak ada outlier di luar clamp bounds
- Happy path: dua akun berbeda → personality params berbeda namun stabil antar-run (deterministic dari seed)
- Happy path: pesan 5 kata vs 50 kata → delay typing berkorelasi positif (r > 0.8 pada sample 100 pasang)
- Happy path: state machine → rata-rata burst length sesuai range personality, transisi Resting muncul
- Edge case: semua pesan kosong → typing duration = floor minimum, tidak crash
- Edge case: `wpm = 0` config invalid → fallback ke default 45 wpm
- Error path: inject typing indicator gagal → lanjut kirim tanpa indicator, log warning (tidak fatal)
- Integration: adaptive backoff → simulasikan 3 failure berturut-turut → delay berikutnya membesar exponential, event `backoff_engaged` emitted
- Integration: send-order jitter → 20 kontak pertama tetap semua terkirim walau urutan berubah

**Verification:**
- Distribusi 1000 delay sample lulus uji statistik (mean ±10%, zero out-of-bounds)
- Dua akun menghasilkan pola berbeda yang reproducible
- Backoff engage dan recover tanpa crash, ter-log di UI

---

- [ ] U14. **Local REST API / Webhook Mode**

**Goal:** `src-tauri/src/api/server.rs` — `axum` server yang spawn di background task,
expose endpoint HTTP localhost untuk integrasi sistem eksternal tanpa buka UI

**Requirements:** R17

**Dependencies:** U7, U3

**Files:**
- Create: `src-tauri/src/api/mod.rs`
- Create: `src-tauri/src/api/server.rs`
- Create: `src/pages/api_settings.html`
- Test: `src-tauri/src/api/server.rs` (integration tests)

**Approach:**
- Spawn `axum::Router` di `tokio::spawn` saat app startup jika API mode enabled
- Default port: `8765`, configurable di settings
- Endpoints:
  - `POST /api/blast` — body JSON: `{account, contacts: [{number, ...}], message, delay_min, delay_max}` → trigger `CampaignSender`
  - `GET /api/status` — return `{running: bool, sent: u32, failed: u32, pending: u32}`
  - `POST /api/stop` — stop campaign aktif
  - `POST /api/check` — check nomor WA validity, body: `{account, numbers: [...]}`
- Response selalu JSON dengan `{ok: bool, data: ..., error: string|null}`
- Bind ke `127.0.0.1` ONLY — tidak expose ke network interface lain
- Share state via `Arc<AppState>` yang sama dengan Tauri commands
- UI settings: toggle enable/disable, port config, copy curl example ke clipboard

**Test scenarios:**
- Happy path: `POST /api/blast` dengan 3 kontak valid → campaign start, status `running: true`
- Happy path: `GET /api/status` mid-campaign → return correct sent/failed/pending count
- Happy path: `POST /api/stop` → campaign berhenti
- Error path: `POST /api/blast` saat campaign sudah running → return `{ok: false, error: "campaign already running"}`
- Error path: account name tidak ada → return `{ok: false, error: "account not found"}`
- Edge case: invalid JSON body → axum return 400 Bad Request
- Security: bind di `0.0.0.0` → tidak diizinkan, hard-coded ke `127.0.0.1`

**Verification:**
- `curl -X POST http://127.0.0.1:8765/api/blast -d '{...}'` berhasil trigger kampanye
- Server tidak accessible dari IP lain di network

---

- [ ] U15. **Template Library**

**Goal:** `src-tauri/src/message/template_library.rs` — CRUD untuk reusable message
templates yang bisa di-save dengan nama, tag, dan optional attachment, lalu di-load
ke Sending Panel kapanpun

**Requirements:** R18

**Dependencies:** U5

**Files:**
- Create: `src-tauri/src/message/template_library.rs`
- Create: `src/pages/templates.html`
- Modify: `src-tauri/src/message/mod.rs`

**Approach:**
- `MessageTemplate` struct: `id: Uuid`, `name: String`, `tags: Vec<String>`, `body: String`, `attachment_path: Option<String>`, `created_at: DateTime`, `updated_at: DateTime`
- Storage: `%APPDATA%\BlastWA\templates.json` — array JSON
- `TemplateLibrary`: `create`, `update`, `delete`, `list`, `search_by_tag`
- UI: grid/list view dengan nama + preview body (truncate 100 char) + tags
- "Load to Composer" button → isi textarea sending panel + set attachment
- Search/filter by tag atau keyword di nama

**Test scenarios:**
- Happy path: create template "Promo Ramadan" → muncul di library list
- Happy path: load template ke composer → body dan attachment terisi
- Happy path: search tag "promo" → filter hanya template ber-tag promo
- Edge case: body template mengandung spintax `{a|b}` → disimpan as-is, di-expand saat send
- Error path: hapus template yang sedang di-load di composer → composer tidak crash, cukup clear state
- Happy path: update template → `updated_at` berubah, body ter-update

**Verification:**
- 50 templates tersimpan dan bisa di-load ulang setelah restart
- Search filter real-time tanpa lag

---

- [ ] U16. **CSV/Excel Import dengan Auto-Column Mapping**

**Goal:** `src-tauri/src/campaign/import.rs` — import kontak dari `.csv` dan `.xlsx`
dengan deteksi header otomatis dan UI mapping dialog yang map kolom file ke field `ContactRow`

**Requirements:** R19

**Dependencies:** U6

**Files:**
- Create: `src-tauri/src/campaign/import.rs`
- Modify: `src-tauri/src/campaign/contact_list.rs`
- Modify: `src/pages/contacts.html`

**Approach:**
- `fn detect_headers(path: &Path) -> Result<Vec<String>>`: read row pertama, return header names
- `fn read_rows(path: &Path, mapping: &ColumnMapping) -> Result<Vec<ContactRow>>`: baca data
- `ColumnMapping` struct: `number_col`, `fullname_col`, `var1_col`, ..., `var5_col` — semua `Option<String>` (nama header)
- CSV: via `csv` crate, `calamine` untuk `.xlsx`
- Auto-suggest mapping: fuzzy match header ke field names ("phone" → `number`, "name" → `fullname`, "hp" → `number`, dst)
- UI mapping dialog: dropdown per field — user bisa override auto-suggestion
- Preview 5 baris pertama sebelum confirm import
- Nomor normalization tetap dipakai dari U6

**Test scenarios:**
- Happy path: CSV dengan header `phone,name,var1` → auto-mapped benar ke `number, fullname, var1`
- Happy path: Excel `.xlsx` dengan header → berhasil di-read via calamine
- Happy path: header "nomor hp" → fuzzy match ke `number` field
- Edge case: kolom `number` tidak ada di file → error dialog "Wajib pilih kolom nomor"
- Edge case: file 10.000 baris → import selesai < 2 detik
- Edge case: mixed type di kolom (angka + teks) → semua diconvert ke string
- Error path: file corrupt / format tidak dikenal → return Err dengan pesan jelas

**Verification:**
- Import 1000 baris dari .xlsx berhasil, semua field ter-map benar
- Preview 5 baris muncul di UI sebelum confirm

---

- [ ] U17. **Sent Log Export ke CSV/Excel**

**Goal:** `src-tauri/src/campaign/log_exporter.rs` — export campaign log (sent/failed/pending
+ metadata) ke file `.csv` atau `.xlsx` yang bisa dibuka di Excel/Sheets

**Requirements:** R20

**Dependencies:** U7, U12

**Files:**
- Create: `src-tauri/src/campaign/log_exporter.rs`
- Modify: `src/pages/log.html`

**Approach:**
- `LogEntry` struct: `timestamp: DateTime`, `number: String`, `fullname: String`, `status: SendStatus`, `error_reason: Option<String>`, `campaign_name: String`
- Export CSV: `csv::Writer` → columns: timestamp, number, fullname, status, error_reason, campaign_name
- Export XLSX: `xlsxwriter::Workbook` → sama, dengan header row bold, kolom status color-coded (green=sent, red=failed, yellow=pending)
- Aksi dari Log Panel UI: tombol "Export CSV" dan "Export Excel"
- Save dialog via Tauri `dialog::save()` — user pilih lokasi file
- Support export: log campaign aktif ATAU semua log historis (pilih via dropdown)

**Test scenarios:**
- Happy path: 100 log entries → export CSV → file valid, bisa dibuka Excel
- Happy path: export XLSX → header row ada, status cells color-coded
- Happy path: campaign selesai → export mencakup semua 100 nomor dengan status masing-masing
- Edge case: log kosong → export file kosong dengan hanya header row
- Edge case: `fullname` mengandung koma/semicolon → CSV di-quote dengan benar
- Error path: path yang dipilih user tidak writable → return Err, tampil toast error di UI

**Verification:**
- File hasil export bisa dibuka di Microsoft Excel tanpa error
- Semua 100 entries ada, tidak ada data yang hilang

---

- [ ] U18. **Built-in WPP.js Auto-Updater**

**Goal:** `src-tauri/src/updater/wpp_updater.rs` — fetch versi terbaru WPP.js dari
GitHub releases, compare dengan versi lokal, download dan hot-swap tanpa rebuild app

**Requirements:** R21

**Dependencies:** U2, U4

**Files:**
- Create: `src-tauri/src/updater/mod.rs`
- Create: `src-tauri/src/updater/wpp_updater.rs`
- Modify: `src-tauri/src/browser/js_injector.rs`
- Modify: `src/pages/settings.html`

**Approach:**
- `WppVersion` struct: `tag_name: String`, `download_url: String`, `published_at: DateTime`
- `fn check_latest() -> Result<WppVersion>`: GET `https://api.github.com/repos/wppconnect-team/wppconnect/releases/latest` dengan `User-Agent: BlastWA/1.0`
- `fn current_version() -> Option<String>`: baca `%APPDATA%\BlastWA\wpp\version.txt`
- `fn update(version: &WppVersion) -> Result<()>`: download JS bundle → backup `wpp.js.bak` → replace `wpp.js` → tulis `version.txt`
- `JsInjector` load WPP.js dari disk (`%APPDATA%\BlastWA\wpp\wpp.js`) jika ada, fallback ke bundled const string
- UI: Settings page → "WPP.js Version" label + "Check for Update" button + progress indicator
- Startup: auto-check sekali per hari (simpan `last_check_at` di config)

**Test scenarios:**
- Happy path: check_latest → return valid `WppVersion` dengan tag dan URL
- Happy path: update tersedia → download sukses → `wpp.js` di disk ter-replace → `version.txt` updated
- Happy path: already up-to-date → UI tampil "Up to date (vXXX)", tidak download
- Error path: no internet → return Err, UI tampil "Could not check: network error"
- Error path: download partial / corrupt → restore dari `wpp.js.bak`, return Err
- Edge case: WPP.js di disk tidak ada (fresh install) → fallback ke bundled version, tidak crash
- Integration: setelah update, inject WPP.js baru ke page → WAPI calls tetap work

**Verification:**
- Update flow dari v lama ke v baru berhasil tanpa restart app
- Jika update gagal, `wpp.js` tetap di versi lama (rollback dari .bak)

---

- [ ] U12. **Profile & Settings Management**

**Goal:** `src-tauri/src/account/profile.rs` — save/load campaign profiles (pesan template +
delay config + contact list) sebagai JSON di `Profiles\` dir

**Requirements:** R15

**Dependencies:** U2, U5, U6

**Files:**
- Modify: `src-tauri/src/account/profile.rs`
- Create: `src/pages/settings.html`

**Approach:**
- `CampaignProfile` struct: `name`, `messages: Vec<String>`, `config: CampaignConfig`, `contacts: ContactList`
- Save: serialize ke JSON → `%APPDATA%\BlastWA\Profiles\{name}\profile.json`
- Load: deserialize dari file
- Settings page: chrome path display, default delay, data directory path

**Test scenarios:**
- Happy path: save profile → file muncul di Profiles dir
- Happy path: load profile → semua field ter-restore
- Error path: profile file corrupt → return Err, tidak crash app
- Edge case: profile name dengan spasi → sanitize jadi filename-safe

**Verification:**
- Profile persist across app restart

---

## System-Wide Impact

- **Chrome instance management:** setiap akun = satu Chrome process + satu CDP port → harus bersih di-cleanup waktu app close (`Drop` impl pada `AccountSession` → kill Chrome process)
- **Async safety:** semua `Arc<Mutex<...>>` untuk shared state, deadlock prevention dengan lock ordering
- **CDP disconnection:** Chrome bisa crash atau close oleh user → `chromiumoxide` future cancel → `AccountSession` auto-mark disconnected
- **WA Web session persistence:** Chrome `--user-data-dir` per akun = QR scan hanya sekali, session tersimpan di disk
- **Rate limiting:** tidak ada hardcoded WA limit — delay di-control user, tapi minimum delay 500ms di-enforce di sender loop
- **JS injection safety:** semua string input di-escape sebelum dimasukkan ke JS template string (cegah JS injection dari data user sendiri)
- **No license calls:** tidak ada network call ke domain manapun selain `web.whatsapp.com` dan `api.github.com` (hanya untuk WPP.js updater, optional)
- **Local API isolation:** axum server HANYA bind ke `127.0.0.1`, tidak pernah `0.0.0.0` — ini di-enforce di kode, bukan hanya config
- **Template library isolation:** templates tidak terikat ke akun tertentu — bisa dipakai di semua akun
- **Log export thread safety:** `LogEntry` dikumpulkan di `Arc<Mutex<Vec<LogEntry>>>` yang di-share antara CampaignSender dan LogExporter

---

## Risks & Dependencies

| Risk | Mitigation |
|---|---|
| WA Web update → WAPI/WPP.js API berubah | JS injection dalam const string mudah di-update; monitor changelog WPP.js |
| `chromiumoxide` CDP tidak support semua Chrome versi | Test minimum Chrome 115+, document requirement di setup.exe |
| Multiple Chrome instances mem-hungry | Document max recommended akun (misal 5), tambah memory warning di UI |
| WA ban karena blast terlalu agresif | HumanBehaviorEngine (burst-rest + backoff + presence) + warning di UI, minimum delay 500ms di-enforce |
| Typing indicator API berubah di WA Web | Fitur presence bersifat best-effort — gagal inject typing tidak menggagalkan kirim |
| GitHub API rate limit saat WPP updater | Cache result 24 jam, hanya hit API sekali per hari per install |
| WPP.js update breaking API (function signature berubah) | Backup .bak sebelum update, rollback otomatis jika inject post-update gagal |
| axum local server port conflict | Port configurable 1024-65535, auto-suggest available port jika conflict |
| CSV encoding non-UTF8 (Windows Excel default ANSI) | `csv` crate dengan BOM detection, fallback ke Windows-1252 via `encoding_rs` crate |
| `tauri` v2 breaking changes | Pin exact version di Cargo.lock |
| Windows-only WinReg untuk Chrome detect | Sementara oke, macOS path bisa ditambah later |

---

## Documentation / Operational Notes

- `README.md` harus include: prerequisite (Chrome terinstall), cara install, cara pakai
- `setup.exe` distribusi terpisah dari `blastwa.exe` — user run setup dulu sekali
- Data tersimpan di `%APPDATA%\BlastWA\` — uninstall manual dengan hapus folder itu
- Logo placeholder di `src-tauri/icons/placeholder.png` — replace waktu branding ready

---

## Sources & References

- **Reverse engineered from:** `OKESENDER.exe` (VB.NET 4.8, WebView2) — `#US` IL heap extraction
- **WAPI source:** https://github.com/wppconnect-team/wppconnect (WPP.js behavior reference)
- **chromiumoxide:** https://crates.io/crates/chromiumoxide
- **Tauri v2:** https://tauri.app/v2/
- **Chrome Registry path:** `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe`
- **winreg crate:** https://crates.io/crates/winreg
