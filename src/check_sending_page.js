// one-off syntax + wiring check for the U4 toolbar additions
const fs = require('fs');
const html = fs.readFileSync(__dirname + '/pages/sending.html', 'utf8');

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
console.log('ALL SENDING PAGE CHECKS PASSED');
