# FRONTEND UI KNOWLEDGE BASE

## OVERVIEW
Vanilla JavaScript Single Page Application (SPA) providing a fast, lightweight dashboard for BlastWA without modern framework overhead.

## STRUCTURE
```
src/
├── index.html                 # Main window frame, sidebar toolbar, modal containers
├── main.js                    # SPA routing engine, navigation epochs, IPC wrappers
├── styles.css                 # CSS variables, responsive grid, dark/light theme
└── pages/                     # Individual view templates loaded dynamically
    ├── dashboard.html         # Session status & profile picker
    ├── sending.html           # Active blast monitoring & pause/resume controls
    ├── contacts.html          # Contact list grid & CSV/Excel uploader
    ├── groups.html            # Group extractor
    ├── autoreply.html         # Auto-response rules
    ├── templates.html         # Spintax editor & preview
    ├── log.html               # Blast history logs
    └── settings.html          # App preferences
```

## WHERE TO LOOK
| Task | File | Notes |
|---|---|---|
| Adding a New Page | `src/main.js` & `src/pages/<name>.html` | Register in `PAGES` array in `main.js` |
| IPC Calls to Rust | `src/main.js` | Use `window.blastwa.invoke`; wrapper supports Tauri and browser mocks |
| Event Listeners | `src/main.js` | Use `listen(event, handler)` for auto-cleanup |

## CONVENTIONS
- Each page script defines an initialization hook named `init_<page>()` (e.g. `init_dashboard()`).
- HTML escaping must strictly use `window.blastwa.esc()`.
- Navigation state is tracked by `navEpoch`; stale callbacks from previously loaded pages must be discarded.
- Page-scoped Tauri listeners use `window.blastwa.listen()`; register timers and other teardown callbacks with `window.blastwa.addCleanup()`.

## ANTI-PATTERNS
- **DO NOT** declare top-level `const`/`let` in `<script>` tags inside `pages/*.html`; use `var` or properties on `window` to allow multiple visits without `Identifier already declared` syntax errors.
- **DO NOT** attach unmanaged global window event listeners without registering them in `addCleanup()`.

## VERIFICATION
- Regression scripts live in `scripts/`, outside this directory. Run from repository root.
- `node scripts/verify_router_lifecycle.js` checks page handlers, revisits, and router lifecycle using mocked browser state.
- `scripts/check_sending_page.js`, `scripts/check_autoreply_page.js`, `scripts/check_checker_cache.js`, and `scripts/check_groups_cache.js` cover page-specific behavior.
- `node scripts/check_window_isolation.js` checks profile WebView storage isolation.
