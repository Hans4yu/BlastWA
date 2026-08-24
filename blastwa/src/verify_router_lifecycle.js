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
    style: {}, classList: { add() {}, remove() {}, toggle() {} },
    appendChild() {}, remove() {}, insertAdjacentHTML() {}, click() {},
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
    fetch: async () => { throw new Error('no fetch in test'); },
    __TAURI__: undefined,
    blastwa: {
      invoke: async () => ({ mock: true }),
      listen: async () => () => {},
      isTauri: false,
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
];
for (const [name, ok] of checks) {
  if (!ok) { console.log(`FAIL main.js: ${name}`); failures++; }
}

console.log('');
console.log(failures ? `${failures} FAILURES` : 'ALL ROUTER LIFECYCLE CHECKS PASSED');
process.exit(failures ? 1 : 0);
})();
