// groups cache contract check.
// fidelity note: each simulated navigation builds a FRESH window scope and
// executes each <script> once, then calls init_groups(), mirroring route()'s
// script re-execution. the localStorage stub is SHARED across navigations,
// because the real app keeps one document (and its storage) alive across tab
// moves. NOT covered here: cross-navigation lexical-declaration collisions
// (top-level let/const sharing the live document's global scope) - those are
// caught by the shared-context double-navigation pass in
// verify_router_lifecycle.js instead.
//
// contract under test (blastwa.groups.cache):
//   1. successful fetch populates cache; next tab visit restores it without
//      calling list_groups again, showing a cache-age note naming the account
//   2. an explicit Refresh bypasses the cache and hits whatsapp
//   3. a failed refresh must NOT blank the page when good cached rows exist
//   4. an EMPTY answer must NOT overwrite an entry holding non-empty rows,
//      even across an account switch on the single cache slot; a later
//      successful fetch for the same account replaces normally
//   5. cache is account-bound: another connected account forces a refetch
//   6. a shapeless/legacy entry without an account field is never served
const fs = require('fs');
const vm = require('vm');
const path = require('path');

const html = fs.readFileSync(path.join(__dirname, '..', 'src', 'pages', 'groups.html'), 'utf8');
const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map((m) => m[1]);

let failures = 0;
function assert(cond, name) {
  if (cond) {
    console.log(`PASS ${name}`);
  } else {
    console.log(`FAIL ${name}`);
    failures++;
  }
}

// shared fake storage: survives "navigations" like real localStorage does
function makeLocalStorage() {
  const store = new Map();
  return {
    getItem: (k) => (store.has(k) ? store.get(k) : null),
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
  };
}

// invoke stub with per-test behavior switches + call recording
function makeInvoke(state) {
  return async (cmd) => {
    state.calls.push(cmd);
    if (cmd === 'list_accounts') {
      return [{ name: state.connectedAccount, connected: true, browser_running: true }];
    }
    if (cmd === 'list_groups') {
      if (state.groupsError) throw new Error(state.groupsError);
      return state.groupsResult.map((g) => ({ ...g }));
    }
    return { mock: true };
  };
}

function makeEl() {
  const listeners = {};
  const el = {
    textContent: '', innerHTML: '', value: '', checked: false, disabled: false,
    dataset: {}, children: [], scrollTop: 0, scrollHeight: 0,
    style: {},
    classList: { add() {}, remove() {}, toggle() {} },
    addEventListener(type, fn) { (listeners[type] ||= []).push(fn); },
    dispatch(type) { for (const fn of listeners[type] || []) fn({ preventDefault() {} }); },
    querySelectorAll() { return []; },
    querySelector() { return null; },
    appendChild() {}, remove() {}, insertAdjacentHTML() {}, click() {},
  };
  el.parentElement = { scrollTop: 0, scrollHeight: 0 };
  return el;
}

// one simulated tab visit: fresh window scope, shared localStorage
async function navigate(localStorageStub, state) {
  const els = new Map();
  const id = (x) => {
    if (!els.has(x)) els.set(x, makeEl());
    return els.get(x);
  };
  const win = {
    document: {
      getElementById: id,
      createElement: () => makeEl(),
      querySelectorAll: () => [],
      querySelector: () => null,
      addEventListener() {},
      body: makeEl(),
    },
    location: { hash: '#/groups' },
    console,
    alert(msg) { state.alerts.push(String(msg)); },
    confirm() { return true; },
    setTimeout, clearTimeout,
    setInterval: () => 0, clearInterval() {},
    fetch: async () => { throw new Error('no fetch in test'); },
    localStorage: localStorageStub,
    blastwa: {
      invoke: makeInvoke(state),
      listen: async () => () => {},
      addCleanup: () => {},
      isTauri: true,
      esc: (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({
        '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
      })[c]),
      stampName: (b, e) => `${b}-test.${e}`,
    },
  };
  win.window = win;
  const ctx = vm.createContext(win);
  for (const code of scripts) {
    vm.runInContext(code, ctx, { filename: 'groups.html[script]' });
  }
  await win.init_groups();
  return { win, els };
}

(async () => {

  // ---- scenario 1: fetch success -> away -> back restores from cache ----
  {
    const state = { calls: [], alerts: [], connectedAccount: 'main', groupsResult: [
      { id: '123@g.us', name: 'Brody Squad' },
      { id: '456@g.us', name: 'Blast Testers' },
    ] };
    const ls = makeLocalStorage();

    const nav1 = await navigate(ls, state);
    assert(state.calls.includes('list_groups'), 's1 first visit fetches groups');
    assert(nav1.els.get('groups-body').innerHTML.includes('Brody Squad'),
      's1 table renders fetched group');

    const callsBefore = state.calls.filter((c) => c === 'list_groups').length;

    const nav2 = await navigate(ls, state);
    const listCalls = state.calls.filter((c) => c === 'list_groups').length - callsBefore;
    assert(listCalls === 0, 's1 second visit does NOT re-invoke list_groups');
    assert(nav2.els.get('groups-body').innerHTML.includes('Brody Squad'),
      's1 second visit renders from cache instantly');
    const note2 = nav2.els.get('groups-cache-note');
    const noteTxt = (note2 && note2.textContent) || '';
    assert(/cached/i.test(noteTxt) && noteTxt.includes('main'),
      's1 restore shows cache-age note naming the account');
  }

  // ---- scenario 2: explicit Refresh bypasses cache ----
  {
    const state = { calls: [], alerts: [], connectedAccount: 'main', groupsResult: [
      { id: '123@g.us', name: 'Brody Squad' },
    ] };
    const ls = makeLocalStorage();
    await navigate(ls, state); // populate cache
    state.groupsResult.push({ id: '789@g.us', name: 'Fresh Grab' });

    const nav2 = await navigate(ls, state);
    nav2.els.get('btn-refresh-groups').dispatch('click');
    await new Promise((r) => setTimeout(r, 0));
    // give the async click handler a tick to finish
    await new Promise((r) => setImmediate(r));
    const listCalls = state.calls.filter((c) => c === 'list_groups').length;
    assert(listCalls >= 2, 's2 explicit Refresh re-invokes list_groups');
    assert(nav2.els.get('groups-body').innerHTML.includes('Fresh Grab'),
      's2 refreshed row visible after explicit Refresh');
  }

  // ---- scenario 3: failed refresh keeps stale rows on screen ----
  {
    const state = { calls: [], alerts: [], connectedAccount: 'main', groupsResult: [
      { id: '123@g.us', name: 'Brody Squad' },
    ] };
    const ls = makeLocalStorage();
    await navigate(ls, state); // populate cache

    const nav2 = await navigate(ls, state);
    state.groupsError = 'cdp down';
    nav2.els.get('btn-refresh-groups').dispatch('click');
    await new Promise((r) => setImmediate(r));
    assert(nav2.els.get('groups-body').innerHTML.includes('Brody Squad'),
      's3 stale cached rows still shown after failed refresh');
    const note = nav2.els.get('groups-cache-note');
    assert(note && /cached/i.test(note.textContent || ''),
      's3 visible note says data comes from cache');
    assert(!/Failed to load groups/.test(nav2.els.get('groups-empty').textContent || ''),
      's3 no dead-end failure message while stale data is on screen');
  }

  // ---- scenario 4: empty answer must not destroy another account's rows ----
  {
    const state = { calls: [], alerts: [], connectedAccount: 'main', groupsResult: [
      { id: '123@g.us', name: 'Brody Squad' },
    ] };
    const ls = makeLocalStorage();
    await navigate(ls, state);

    // switch accounts; whatsapp answers [] for 'second'. the single-slot
    // write chokepoint in saveCache must refuse the empty overwrite so
    // main's good rows survive in storage.
    state.connectedAccount = 'second';
    state.groupsResult = [];
    const navEmpty = await navigate(ls, state);
    const raw = JSON.parse(ls.getItem('blastwa.groups.cache') || 'null');
    assert(!!raw && raw.account === 'main' && Array.isArray(raw.groups) && raw.groups.length > 0,
      's4 refused empty write keeps main-account rows in storage');
    assert(/No groups found for second/.test(navEmpty.els.get('groups-empty').textContent || ''),
      's4 honest empty state shown for second while rows stay stored');

    // a later SUCCESSFUL fetch for second replaces normally
    state.groupsResult = [{ id: '999@g.us', name: 'Second Squad' }];
    const navOk = await navigate(ls, state);
    assert(navOk.els.get('groups-body').innerHTML.includes('Second Squad'),
      's4 later successful fetch renders normally');
    const rawAfter = JSON.parse(ls.getItem('blastwa.groups.cache') || 'null');
    assert(!!rawAfter && rawAfter.account === 'second' && rawAfter.groups.length === 1,
      's4 successful non-empty fetch overwrites the slot normally');
  }

  // ---- scenario 5: cache is account-bound ----
  {
    const state = { calls: [], alerts: [], connectedAccount: 'main', groupsResult: [
      { id: '123@g.us', name: 'Main Group' },
    ] };
    const ls = makeLocalStorage();
    await navigate(ls, state);

    state.connectedAccount = 'second';
    state.groupsResult = [{ id: '999@g.us', name: 'Second Group' }];
    const callsBefore = state.calls.filter((c) => c === 'list_groups').length;
    const nav2 = await navigate(ls, state);
    const listCalls = state.calls.filter((c) => c === 'list_groups').length - callsBefore;
    assert(listCalls === 1, 's5 account switch forces exactly one refetch');
    assert(nav2.els.get('groups-body').innerHTML.includes('Second Group')
      && !nav2.els.get('groups-body').innerHTML.includes('Main Group'),
      's5 other account rows are NOT served from cache');
  }

  // ---- scenario 6: shapeless/legacy entry without account field ----
  {
    const state = { calls: [], alerts: [], connectedAccount: 'main', groupsResult: [
      { id: '123@g.us', name: 'Fresh From Wa' },
    ] };
    const ls = makeLocalStorage();
    ls.setItem('blastwa.groups.cache',
      JSON.stringify({ groups: [{ id: 'x@g.us', name: 'Ghost Rows' }] }));
    const nav = await navigate(ls, state);
    assert(state.calls.includes('list_groups'),
      's6 pre-seeded entry WITHOUT account field does not satisfy restore');
    assert(nav.els.get('groups-body').innerHTML.includes('Fresh From Wa')
      && !nav.els.get('groups-body').innerHTML.includes('Ghost Rows'),
      's6 rendered rows come from the live fetch, not the shapeless cache');
  }

  console.log('');
  console.log(failures ? `${failures} FAILURES` : 'ALL GROUPS CACHE CHECKS PASSED');
  process.exit(failures ? 1 : 0);
})().catch((e) => {
  console.error('HARNESS ERROR:', e);
  process.exit(1);
});
