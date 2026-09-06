// auto-reply page contracts, driven the same way route() drives pages:
// each simulated navigation gets a FRESH window scope, and the cleanups the
// page registered through addCleanup run before the next navigation starts
// (mirroring runPageCleanups()).
//
// contracts pinned:
//   1. editing a row schedules a debounced save_rules (tab switch no longer
//      wipes rules — the regression the user reported)
//   2. navigating away with a pending debounce still persists (cleanup save)
//   3. incomplete rows (no keyword / no reply) are not persisted
//   4. saved rules restore on load, with PascalCase match types mapped back
//      to the select values, and loading never marks the page dirty
//   5. deleting a row persists the deletion on navigation
//   6. the watcher status strip reflects autoreply_status telemetry
const fs = require('fs');
const vm = require('vm');
const path = require('path');

const html = fs.readFileSync(path.join(__dirname, '..', 'src', 'pages', 'autoreply.html'), 'utf8');
const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((m) => m[1]);

let failures = 0;
function assert(name, cond) {
  console.log(`${cond ? 'PASS' : 'FAIL'} ${name}`);
  if (!cond) failures++;
}

// static: every id referenced through $('...') must exist in the markup
{
  const ids = new Set([...html.matchAll(/id="([^"]+)"/g)].map((m) => m[1]));
  const refs = new Set([...scripts.join('\n').matchAll(/\$\('([^']+)'\)/g)].map((m) => m[1]));
  const missing = [...refs].filter((r) => !ids.has(r));
  assert('no dangling $() references', missing.length === 0);
  if (missing.length) console.error('  missing:', missing.join(', '));
}

// ----- mini DOM: just enough for this page's row building + delegation -----

function matchSel(node, sel) {
  if (sel.startsWith('.')) return node._classes.includes(sel.slice(1));
  const [tag, ...classes] = sel.split('.');
  if (tag && node.tagName !== tag.toUpperCase()) return false;
  return classes.every((c) => node._classes.includes(c));
}

function matchesDesc(root, sel) {
  const out = [];
  const walk = (el) => {
    for (const c of el.children) {
      if (matchSel(c, sel)) out.push(c);
      walk(c);
    }
  };
  walk(root);
  return out;
}

function makeEl(tag, classes = []) {
  const el = {
    tagName: tag.toUpperCase(),
    _classes: [...classes],
    children: [],
    parentElement: null,
    textContent: '',
    innerHTML: '',
    value: '',
    checked: false,
    disabled: false,
    style: {},
    dataset: {},
    classList: {
      add: (...cs) => cs.forEach((c) => { if (!el._classes.includes(c)) el._classes.push(c); }),
      remove: (...cs) => { el._classes = el._classes.filter((c) => !cs.includes(c)); },
      toggle: (c, force) => {
        const on = force === undefined ? !el._classes.includes(c) : force;
        if (on) el.classList.add(c); else el.classList.remove(c);
      },
      contains: (c) => el._classes.includes(c),
    },
    addEventListener(type, fn) { (this._h ||= {})[type] = fn; },
    removeEventListener() {},
    querySelector(sel) { return matchesDesc(el, sel)[0] || null; },
    querySelectorAll(sel) { return matchesDesc(el, sel); },
    appendChild(c) { c.parentElement = el; el.children.push(c); return c; },
    remove() {
      if (!el.parentElement) return;
      const i = el.parentElement.children.indexOf(el);
      if (i >= 0) el.parentElement.children.splice(i, 1);
    },
    focus() {},
    click() { el._h && el._h.click && el._h.click(); },
    closest(sel) {
      let n = el;
      while (n) {
        if (matchSel(n, sel)) return n;
        n = n.parentElement;
      }
      return null;
    },
  };
  // building a rule row: recreate its input children from the class tokens
  // in the template (checkbox r-enabled, select r-match, inputs, danger btn)
  Object.defineProperty(el, 'innerHTML', {
    get() { return el._html || ''; },
    set(v) {
      el._html = String(v);
      el.children = [];
      for (const g of el._html.matchAll(/class="([^"]+)"/g)) {
        const cs = g[1].split(/\s+/);
        if (cs.includes('btn-danger')) {
          el.appendChild(makeEl('button', cs));
        } else if (cs.includes('r-match')) {
          const s = makeEl('select', cs);
          s.value = 'contains';
          el.appendChild(s);
        } else if (cs.includes('r-enabled')) {
          const c = makeEl('input', cs);
          c.checked = true;
          el.appendChild(c);
        } else {
          el.appendChild(makeEl('input', cs));
        }
      }
    },
  });
  Object.defineProperty(el, 'lastElementChild', {
    get() { return el.children[el.children.length - 1] || null; },
  });
  return el;
}

// ----- navigation harness -----

let cleanups = [];
let timers = [];

async function navigate(state) {
  for (const fn of cleanups.splice(0)) {
    try { fn(); } catch (e) { console.error('cleanup failed', e); }
  }
  timers = [];
  const els = new Map();
  const $ = (id) => {
    if (!els.has(id)) {
      const el = makeEl('div');
      el.id = id;
      els.set(id, el);
    }
    return els.get(id);
  };
  const saveCalls = [];
  const win = {
    document: {
      getElementById: $,
      createElement: (t) => makeEl(t),
      querySelectorAll: () => [],
      querySelector: () => null,
      addEventListener() {},
      body: makeEl('body'),
    },
    location: { hash: '#/autoreply' },
    console: { log() {}, warn() {}, error() {} },
    alert(msg) { state.alerts.push(String(msg)); },
    setTimeout: (fn, ms) => { const t = { fn, ms }; timers.push(t); return t; },
    clearTimeout: (t) => { if (t) t.cleared = true; },
    setInterval: () => 0,
    clearInterval() {},
    blastwa: {
      isTauri: false,
      esc: (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({
        '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
      })[c]),
      addCleanup: (fn) => cleanups.push(fn),
      listen: async () => () => {},
      invoke: async (cmd, args) => {
        if (cmd === 'load_rules') return state.savedRules || [];
        if (cmd === 'save_rules') {
          saveCalls.push(args.rules);
          return { ok: true, saved: args.rules.length, skipped: 0 };
        }
        if (cmd === 'autoreply_status') return state.status ||
          { total_rules: 0, armed_rules: 0, watching: [], replies_sent: 0, last_reply_epoch: 0 };
        return {};
      },
    },
  };
  win.window = win;

  const ctx = vm.createContext(win);
  for (const code of scripts) vm.runInContext(code, ctx, { filename: 'autoreply.html' });
  await win.init_autoreply();
  await new Promise((r) => setTimeout(r, 0)); // let the trailing refresh settle

  return {
    $, saveCalls,
    flushTimers: async () => {
      while (timers.some((t) => !t.cleared)) {
        for (const t of timers.splice(0)) if (!t.cleared) t.fn();
        await new Promise((r) => setTimeout(r, 0));
      }
    },
  };
}

function fillRow(row, { keyword, reply }) {
  row.querySelector('.r-keyword').value = keyword;
  row.querySelector('.r-reply').value = reply;
}

(async () => {
  // --- s1: an edit schedules a debounced save with the wire format ---
  const a = await navigate({ alerts: [] });
  a.$('btn-add-rule')._h.click();
  const rowA = a.$('rules-body').children[0];
  fillRow(rowA, { keyword: 'Promo', reply: 'ketik menu' });
  a.$('rules-body')._h.input({ target: rowA.querySelector('.r-keyword') });
  await a.flushTimers();
  assert('s1 edit autosaved exactly one rule',
    a.saveCalls.length === 1 && a.saveCalls[0].length === 1);
  assert('s1 match type sent in PascalCase wire form',
    a.saveCalls[0][0].match_type === 'Contains');
  assert('s1 keyword + reply round-trip',
    a.saveCalls[0][0].keyword === 'Promo' && a.saveCalls[0][0].reply_message === 'ketik menu');

  // --- s2: navigating away with the debounce still pending persists ---
  const b = await navigate({ alerts: [] });
  b.$('btn-add-rule')._h.click();
  fillRow(b.$('rules-body').children[0], { keyword: 'halo', reply: 'halo juga' });
  b.$('rules-body')._h.input({ target: b.$('rules-body').children[0].querySelector('.r-keyword') });
  assert('s2 no save yet before debounce fires', b.saveCalls.length === 0);
  for (const fn of cleanups.splice(0)) fn(); // simulate runPageCleanups()
  await new Promise((r) => setTimeout(r, 10));
  assert('s2 navigation cleanup saved the pending row',
    b.saveCalls.length === 1 && b.saveCalls[0].length === 1 &&
    b.saveCalls[0][0].keyword === 'halo');

  // --- s3: incomplete rows are skipped at save time ---
  const c = await navigate({ alerts: [] });
  c.$('btn-add-rule')._h.click();
  c.$('btn-add-rule')._h.click();
  fillRow(c.$('rules-body').children[0], { keyword: 'siap', reply: 'ok' });
  fillRow(c.$('rules-body').children[1], { keyword: 'no-reply-yet', reply: '' });
  c.$('btn-save-rules')._h.click();
  await new Promise((r) => setTimeout(r, 10));
  assert('s3 only the fully-armed row is persisted',
    c.saveCalls.length === 1 && c.saveCalls[0].length === 1 &&
    c.saveCalls[0][0].keyword === 'siap');

  // --- s4: saved rules restore, match types map back, load stays clean ---
  const d = await navigate({
    alerts: [],
    savedRules: [
      { name: 'menu', match_type: 'StartWith', keyword: 'menu', reply_message: 'balas menu', enabled: true },
      { name: 'off', match_type: 'Like', keyword: 'test', reply_message: 'x', enabled: false },
    ],
  });
  assert('s4 both saved rules restored as rows',
    d.$('rules-body').children.length === 2);
  const rowD = d.$('rules-body').children[0];
  assert('s4 PascalCase StartWith mapped back to select value',
    rowD.querySelector('.r-match').value === 'start_with');
  assert('s4 disabled flag restored',
    d.$('rules-body').children[1].querySelector('.r-enabled').checked === false);
  assert('s4 loading never triggered a save', d.saveCalls.length === 0);

  // --- s5: deleting a row persists the deletion on navigation ---
  const e = await navigate({
    alerts: [],
    savedRules: [
      { name: 'a', match_type: 'Contains', keyword: 'aaa', reply_message: 'r1', enabled: true },
      { name: 'b', match_type: 'Contains', keyword: 'bbb', reply_message: 'r2', enabled: true },
    ],
  });
  const delBtn = e.$('rules-body').children[0].querySelector('button.btn-danger');
  e.$('rules-body')._h.click({ target: delBtn });
  assert('s5 delete removed the row', e.$('rules-body').children.length === 1);
  for (const fn of cleanups.splice(0)) fn();
  await new Promise((r) => setTimeout(r, 10));
  assert('s5 navigation saved the surviving rule only',
    e.saveCalls.length === 1 && e.saveCalls[0].length === 1 &&
    e.saveCalls[0][0].keyword === 'bbb');

  // --- s6: watcher status strip renders telemetry ---
  const f = await navigate({
    alerts: [],
    status: { total_rules: 2, armed_rules: 2, watching: ['main'], replies_sent: 3, last_reply_epoch: 1730000000 },
  });
  assert('s6 live watcher named in the status strip',
    /Auto-reply is on/.test(f.$('watch-status').innerHTML) &&
    /main/.test(f.$('watch-status').innerHTML) &&
    /3 auto-replies sent/.test(f.$('watch-status').innerHTML));

  const g = await navigate({
    alerts: [],
    status: { total_rules: 0, armed_rules: 0, watching: [], replies_sent: 0, last_reply_epoch: 0 },
  });
  assert('s6 idle state points at the Dashboard when rules are armed but no session runs — or the neutral add-a-rule copy',
    /Dashboard|Add a rule/.test(g.$('watch-status').innerHTML));

  console.log('');
  console.log(failures ? `${failures} FAILURES` : 'ALL AUTOREPLY PAGE CHECKS PASSED');
  process.exit(failures ? 1 : 0);
})().catch((e) => {
  console.error('HARNESS ERROR', e);
  process.exit(1);
});
