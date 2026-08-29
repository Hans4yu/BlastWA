// number-checker cache contract, driven the same way route() drives pages:
// each simulated navigation gets a FRESH window scope but shares one
// localStorage stub, because the real app keeps one document (and therefore
// one storage) alive across tab switches while re-executing page scripts.
//
// top-level lexical-declaration collisions (let/const re-declared on the
// second navigation of the SAME live document) are NOT detectable here --
// that class is covered by verify_router_lifecycle.js's shared-context pass.
//
// contracts pinned:
//   1. a completed check survives navigating away and back (no re-invoke)
//   2. Run Check always re-invokes and replaces the rows
//   3. a failed re-check keeps the previous rows on screen
//   4. Keep Valid Only clears the cache (the list it described is gone)
//   5. export/keep buttons enable only when at least one row answered yes
const fs = require('fs');
const vm = require('vm');
const path = require('path');

const html = fs.readFileSync(path.join(__dirname, '..', 'src', 'pages', 'contacts.html'), 'utf8');
const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((m) => m[1]);

let failures = 0;
function assert(name, cond) {
  console.log(`${cond ? 'PASS' : 'FAIL'} ${name}`);
  if (!cond) failures++;
}

// one storage for the whole run: this is the thing that must survive
const storage = new Map();
const localStorageStub = {
  getItem: (k) => (storage.has(k) ? storage.get(k) : null),
  setItem: (k, v) => storage.set(k, String(v)),
  removeItem: (k) => storage.delete(k),
};

function makeEl(id) {
  const el = {
    id,
    _text: '',
    innerHTML: '',
    value: '',
    checked: true,
    disabled: false,
    style: {},
    options: [],
    children: [],
    classList: { add() {}, remove() {}, toggle() {} },
    dataset: {},
    addEventListener(type, fn) { (this._h ||= {})[type] = fn; },
    removeEventListener() {},
    querySelectorAll(sel) {
      if (sel === 'tr') {
        const n = (this.innerHTML.match(/<tr>/g) || []).length;
        return Array.from({ length: n }, () => makeEl('tr'));
      }
      return [];
    },
    querySelector: () => null,
    insertAdjacentHTML(_pos, frag) { this.innerHTML += frag; },
    appendChild() {}, remove() {}, click() { this._h && this._h.click && this._h.click(); },
    focus() {}, dispatchEvent() {},
  };
  Object.defineProperty(el, 'textContent', {
    get() { return this._text; },
    set(v) { this._text = String(v); },
  });
  Object.defineProperty(el, 'parentElement', {
    get() { return { scrollTop: 0, scrollHeight: 0 }; },
  });
  return el;
}

// one navigation: fresh scope + fresh element map, shared storage
async function navigate(state) {
  const els = new Map();
  const $ = (id) => {
    if (!els.has(id)) els.set(id, makeEl(id));
    return els.get(id);
  };

  const invokes = [];
  const win = {
    document: {
      getElementById: $,
      createElement: (t) => makeEl(t),
      querySelectorAll: () => [],
      querySelector: () => null,
      addEventListener() {},
      body: makeEl('body'),
    },
    localStorage: localStorageStub,
    location: { hash: '#/contacts' },
    console: { log() {}, warn() {}, error() {} },
    alert() {}, confirm: () => state.confirm !== false, prompt: () => '',
    setTimeout, clearTimeout, setInterval: () => 0, clearInterval() {},
    requestAnimationFrame: (cb) => setTimeout(() => cb(16), 0),
    Event: class { constructor(t) { this.type = t; } },
    __TAURI__: undefined,
    blastwa: {
      isTauri: false,
      esc: (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({
        '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
      })[c]),
      addCleanup() {},
      listen: async () => () => {},
      stampName: (b, e) => `${b}.${e}`,
      invoke: async (cmd, args) => {
        invokes.push(cmd);
        if (cmd === 'get_contacts') return state.contacts || [];
        if (cmd === 'list_accounts') return [{ name: 'tes', connected: true }];
        if (cmd === 'check_numbers_cmd') {
          if (state.checkThrows) throw new Error('cdp hiccup');
          return state.checkResult || [];
        }
        if (cmd === 'keep_contacts_only') return { kept: (args.validNumbers || []).length };
        if (cmd === 'export_valid_numbers') return { exported: 1 };
        return {};
      },
    },
  };
  win.window = win;

  const ctx = vm.createContext(win);
  for (const code of scripts) vm.runInContext(code, ctx, { filename: 'contacts.html' });
  await win.init_contacts();

  return { els, invokes, win, $ };
}

const YES = { number: '6282132102060', exists: true, kind: 'Regular' };
const NO = { number: '6289541071050', exists: false, kind: 'Not Found' };

(async () => {
  // --- s1: a completed check survives a tab switch, with no re-check ---
  const a = await navigate({ checkResult: [YES, NO] });
  a.$('btn-run-check')._h.click();
  await new Promise((r) => setTimeout(r, 30));
  assert('s1 check ran and rendered both rows',
    (a.$('check-body').innerHTML.match(/<tr>/g) || []).length === 2);
  assert('s1 export enabled when a valid row exists',
    a.$('btn-export-valid').disabled === false);

  const b = await navigate({ checkResult: [YES, NO] });
  assert('s1 restored rows after navigating back',
    (b.$('check-body').innerHTML.match(/<tr>/g) || []).length === 2);
  assert('s1 restore did NOT re-invoke the checker',
    !b.invokes.includes('check_numbers_cmd'));
  assert('s1 restore note names the cache age',
    /cached/i.test(b.$('check-summary').textContent));
  assert('s1 restore re-enables export from cache',
    b.$('btn-export-valid').disabled === false);
  assert('s1 restored rows hide the empty state',
    b.$('check-empty').style.display === 'none');

  // --- s2: Run Check always re-invokes and replaces rows ---
  const c = await navigate({ checkResult: [YES] });
  c.$('btn-run-check')._h.click();
  await new Promise((r) => setTimeout(r, 30));
  assert('s2 explicit Run Check re-invoked the checker',
    c.invokes.filter((i) => i === 'check_numbers_cmd').length === 1);
  assert('s2 rows replaced by the fresh result',
    (c.$('check-body').innerHTML.match(/<tr>/g) || []).length === 1);

  // --- s3: a failed re-check keeps the previous rows visible ---
  const d = await navigate({ checkThrows: true });
  await new Promise((r) => setTimeout(r, 10));
  d.$('btn-run-check')._h.click();
  await new Promise((r) => setTimeout(r, 40));
  assert('s3 previous rows still on screen after a failed check',
    (d.$('check-body').innerHTML.match(/<tr>/g) || []).length === 1);
  assert('s3 no dead-end empty state while cached rows are shown',
    d.$('check-empty').style.display === 'none');
  assert('s3 cache untouched by the failure',
    JSON.parse(storage.get('blastwa.check.cache')).outcomes.length === 1);

  // --- s4: Keep Valid Only drops the cache it just consumed ---
  const e = await navigate({ checkResult: [YES, NO] });
  e.$('btn-run-check')._h.click();
  await new Promise((r) => setTimeout(r, 30));
  e.$('btn-keep-valid')._h.click();
  await new Promise((r) => setTimeout(r, 40));
  assert('s4 cache cleared after Keep Valid Only',
    storage.get('blastwa.check.cache') === undefined);
  assert('s4 export disabled again once the list is consumed',
    e.$('btn-export-valid').disabled === true);

  const f = await navigate({});
  assert('s4 nothing restored from a cleared cache',
    (f.$("check-body").innerHTML.match(/<tr>/g) || []).length === 0);

  // --- s5: an all-invalid result leaves both action buttons disabled ---
  const g = await navigate({ checkResult: [NO] });
  g.$('btn-run-check')._h.click();
  await new Promise((r) => setTimeout(r, 30));
  assert('s5 keep disabled when nothing is valid',
    g.$('btn-keep-valid').disabled === true);
  assert('s5 export disabled when nothing is valid',
    g.$('btn-export-valid').disabled === true);
  assert('s5 invalid rows are still cached and rendered',
    (g.$('check-body').innerHTML.match(/<tr>/g) || []).length === 1);

  console.log('');
  console.log(failures ? `${failures} FAILURES` : 'ALL CHECKER CACHE CHECKS PASSED');
  process.exit(failures ? 1 : 0);
})().catch((e) => {
  console.error('HARNESS ERROR', e);
  process.exit(1);
});
