// source-level router lifecycle verification.
// simulates exactly what route() does: inject page html, re-execute each
// <script> in a fresh window scope, then assert the handlers that inline
// onclick attributes reference are actually defined afterwards.
const fs = require('fs');
const vm = require('vm');
const path = require('path');

const PAGES = ['dashboard', 'sending', 'contacts', 'groups', 'autoreply', 'templates', 'log', 'settings'];

// handlers referenced by inline onclick / oninput in each page's static html
const REQUIRED = {
  dashboard: ['init_dashboard', 'addAccount', 'openAccountBrowser', 'removeAccount'],
  sending: ['init_sending'],
  contacts: ['init_contacts', 'handleImport', 'clearAll'],
  groups: ['init_groups'],
  autoreply: ['init_autoreply', 'addRuleRow', 'syncEmpty'],
  templates: ['init_templates', 'openNewTpl', 'closeEditor', 'saveTpl'],
  log: ['init_log', 'exportLog'],
  settings: ['init_settings', 'saveSettings'],
};

function makeWindow() {
  const el = () => ({
    addEventListener() {}, removeEventListener() {},
    querySelectorAll: () => [], querySelector: () => null,
    innerHTML: '', textContent: '', value: '', checked: false,
    style: {}, dataset: {}, classList: { add() {}, remove() {}, toggle() {} },
    appendChild() {}, remove() {}, insertAdjacentHTML() {}, click() {},
    focus() {}, select() {}, disabled: false,
    children: [],
  });
  const win = {
    document: {
      getElementById: () => el(),
      createElement: () => el(),
      querySelectorAll: () => [],
      querySelector: () => null,
      addEventListener() {}, body: el(),
    },
    location: { hash: '' },
    console: { log() {}, warn() {}, error() {} },
    alert() {}, prompt() { return 'test'; }, confirm() { return true; },
    setTimeout, clearTimeout, setInterval: () => 0, clearInterval() {},
    // contacts.html rows-visible slider measures row height on next frame
    requestAnimationFrame: (cb) => setTimeout(() => cb(16), 0),
    fetch: async () => { throw new Error('no fetch in test'); },
    __TAURI__: undefined,
    blastwa: {
      invoke: async () => ({ mock: true }),
      listen: async () => () => {},
      addCleanup: () => {},
      isTauri: false,
      esc: (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({
        '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
      })[c]),
    },
  };
  win.window = win;
  return win;
}

let failures = 0;

(async () => {

// simulate TWO navigations per page to prove no init duplication issues
// and that handlers are (re)defined each time
for (const page of PAGES) {
  const html = fs.readFileSync(path.join(__dirname, 'pages', page + '.html'), 'utf8');
  const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map(m => m[1]);

  for (const nav of [1, 2]) {
  // eslint-disable-next-line no-await-in-loop
    const win = makeWindow();
    const ctx = vm.createContext(win);
    try {
      // execute each script in order, same as route() does
      for (const code of scripts) {
        vm.runInContext(code, ctx, { filename: `${page}.html[script]` });
      }
      // then the router calls the init hook exactly once
      if (typeof win[`init_${page}`] === 'function') {
        await win[`init_${page}`]();
      }
      // assert required globals exist after full lifecycle
      for (const name of REQUIRED[page]) {
        if (typeof win[name] !== 'function') {
          console.log(`FAIL ${page} nav${nav}: ${name} is ${typeof win[name]}`);
          failures++;
        }
      }
      if (typeof win[`init_${page}`] !== 'function') {
        console.log(`FAIL ${page} nav${nav}: init hook missing`);
        failures++;
      }
    } catch (e) {
      console.log(`FAIL ${page} nav${nav}: script threw: ${e.message}`);
      failures++;
    }
  }
}

// shared-context pass: real tab switching happens in ONE document, so all
// page scripts share the same global lexical environment for the app's whole
// lifetime. execute every page's script in a single window, twice, in
// toolbar order; any 'already declared' collision or cross-page clobbering
// fails here.
{
  // makeWindow() already installs blastwa.esc; no reassignment needed here.
  const win = makeWindow();
  const ctx = vm.createContext(win);
  try {
    for (let round = 1; round <= 2; round++) {
      for (const page of PAGES) {
        const html = fs.readFileSync(path.join(__dirname, 'pages', page + '.html'), 'utf8');
        const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((m) => m[1]);
        for (const code of scripts) {
          vm.runInContext(code, ctx, { filename: `${page}.html[shared r${round}]` });
        }
        if (typeof win[`init_${page}`] === 'function') {
          // eslint-disable-next-line no-await-in-loop
          await win[`init_${page}`]();
        } else {
          console.log(`FAIL shared nav r${round} ${page}: init hook missing`);
          failures++;
        }
      }
    }
  } catch (e) {
    console.log(`FAIL shared-context navigation sequence: ${e.message}`);
    failures++;
  }
}

// verify main.js itself parses as a module and contains the lifecycle guards
const mainjs = fs.readFileSync(path.join(__dirname, 'main.js'), 'utf8');
const checks = [
  ['script re-execution after injection', /createElement\(['"]script['"]\)/.test(mainjs)],
  ['scripts removed after execution (no double-run)', /old\.remove\(\)/.test(mainjs)],
  ['init called exactly once per load', /if \(typeof initFn === ['"]function['"]\)/.test(mainjs)],
  ['init failure does not wipe page', /init_\$\{page\} failed/.test(mainjs)],
  ['nav epoch guard exists', /navEpoch\+\+/.test(mainjs)],
  ['page cleanups run on navigation', /runPageCleanups\(\)/.test(mainjs)],
  ['stale listener registration dropped', /user navigated away before registration/.test(mainjs)],
  ['shared esc() exposed on window.blastwa', /stampName, esc \}/.test(mainjs)],
];
for (const [name, ok] of checks) {
  if (!ok) { console.log(`FAIL main.js: ${name}`); failures++; }
}

// esc hygiene: pages must alias the shared helper, never re-declare it at
// top level. a top-level const/let/function collides on the SECOND navigation
// because classic-script lexical globals persist for the document lifetime.
// shallow-indent bound (0-4 chars incl. tabs): catches top-level declarations
// written at any common style, while leaving legitimately nested consts
// (e.g. sending.html's function-local `const esc`, indented deeper) alone;
// actual collisions at ANY indent still fail the shared-context pass above.
for (const page of PAGES) {
  const pageHtml = fs.readFileSync(path.join(__dirname, 'pages', page + '.html'), 'utf8');
  if (/^[ \t]{0,4}function esc\(/m.test(pageHtml)) {
    console.log(`FAIL ${page}: top-level 'function esc' redeclared (use window.blastwa.esc)`);
    failures++;
  }
  if (/^[ \t]{0,4}(const|let)[ \t]+esc\b/m.test(pageHtml)) {
    console.log(`FAIL ${page}: top-level 'const/let esc' throws on second navigation`);
    failures++;
  }
}

console.log('');
console.log(failures ? `${failures} FAILURES` : 'ALL ROUTER LIFECYCLE CHECKS PASSED');
process.exit(failures ? 1 : 0);
})();
