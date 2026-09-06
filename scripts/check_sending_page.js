// one-off syntax + wiring check for the U4 toolbar additions
const fs = require('fs');
const path = require('path');
const html = fs.readFileSync(path.join(__dirname, '..', 'src', 'pages', 'sending.html'), 'utf8');

const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
if (!scripts.length) {
  console.error('FAIL: no inline script found');
  process.exit(1);
}
for (const code of scripts) {
  try { new Function(code); } catch (e) {
    console.error('FAIL: script syntax error:', e.message);
    process.exit(1);
  }
}

// every element id referenced through $('...') must exist in the markup
const ids = new Set([...html.matchAll(/id="([^"]+)"/g)].map((m) => m[1]));
const refs = new Set([...scripts.join('\n').matchAll(/\$\('([^']+)'\)/g)].map((m) => m[1]));
const missing = [...refs].filter((r) => !ids.has(r));
if (missing.length) {
  console.error('FAIL: referenced ids missing from markup:', missing.join(', '));
  process.exit(1);
}

// toolbar contract: wrap buttons carry data-wrap, emoji grid exists
for (const need of ['fmt-toolbar', 'btn-emoji', 'emoji-grid']) {
  if (!ids.has(need)) {
    console.error('FAIL: missing required id:', need);
    process.exit(1);
  }
}
const wraps = [...html.matchAll(/data-wrap="([^"]*)"/g)].map((m) => m[1]);
if (JSON.stringify(wraps) !== JSON.stringify(['*', '_', '~', '```'])) {
  console.error('FAIL: unexpected wrap tokens:', wraps);
  process.exit(1);
}

// attachment picker contract: the visible "Choose File" control must be a
// real button (a bare visible <input type=file> would taunt the user with
// tauri's permanent "No file chosen" label), the native input stays hidden
// as the browser-dev fallback, and a Remove control exists
for (const need of ['btn-choose-attachment', 'btn-clear-attachment', 'attachment', 'attachment-name']) {
  if (!ids.has(need)) {
    console.error('FAIL: missing attachment picker id:', need);
    process.exit(1);
  }
}
const fileInputTag = html.match(/<input[^>]*type="file"[^>]*>/);
if (!fileInputTag || !/display\s*:\s*none/.test(fileInputTag[0])) {
  console.error('FAIL: native file input must stay hidden (display:none):', fileInputTag && fileInputTag[0]);
  process.exit(1);
}
console.log('ALL SENDING PAGE CHECKS PASSED');
