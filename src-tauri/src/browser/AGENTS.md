# BROWSER AUTOMATION KNOWLEDGE BASE

## OVERVIEW
The `browser` module manages Chrome DevTools Protocol (CDP) WebSocket communication, local Chrome binary auto-detection, and WPPConnect JavaScript library runtime injection.

## STRUCTURE
```
src-tauri/src/browser/
├── cdp_client.rs     # Low-level WebSocket client for CDP commands and events
├── chrome_detect.rs  # OS registry and standard path detector for Chrome/Brave/Edge
├── js_injector.rs    # WPPConnect payload injection & JS evaluation bridge
└── mod.rs            # Chrome session lifecycle and process supervisor
```

## WHERE TO LOOK
| Task | File | Notes |
|---|---|---|
| Launching Browser | `mod.rs` / `chrome_detect.rs` | Finds executable and launches with remote debugging port |
| CDP Command Execution | `cdp_client.rs` | Handles `Runtime.evaluate`, `Page.navigate`, and DOM events |
| WA Automation Scripts | `js_injector.rs` | Evaluates WPPConnect wrapper methods inside WhatsApp Web |

## CONVENTIONS
- Always verify CDP connection health before issuing automation commands.
- Handle WhatsApp QR code generation, session restoration, and disconnect events gracefully.
- Isolate user data directories per account profile (`Data/profiles/<name>`).

## ANTI-PATTERNS
- **DO NOT** inject untrusted user input directly into evaluation strings.
- **NEVER** open multiple conflicting CDP sessions on the same remote debugging port.
