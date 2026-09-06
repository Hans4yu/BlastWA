// BlastWA router + Tauri IPC helpers. vanilla js, no build step.
// shell: menubar + toolbar + content + statusbar (oke sender-style chrome).
//
// page script lifecycle: pages are injected via innerHTML, which per HTML5
// spec does NOT execute <script> tags. route() therefore re-creates each
// script element (executing them in document order), then calls the page's
// init_<page>() exactly once per load. per-page listeners registered through
// blastwa.listen() are tracked and cleaned up on the next navigation so
// repeated navigation cannot accumulate duplicates.

const isTauri = !!window.__TAURI__;
// tauri's webview user agent contains "Tauri" even when the global bridge
// is missing, which lets us tell a broken desktop build apart from a
// deliberate browser-dev session.
const uaIsTauri = /tauri/i.test(navigator.userAgent || '');

if (!isTauri && uaIsTauri) {
  // desktop build without the bridge = broken install. never mock silently.
  console.error(
    '[BlastWA] FATAL: running inside the Tauri desktop shell but window.__TAURI__ is missing. ' +
    'IPC is unavailable. Check that app.withGlobalTauri is enabled in tauri.conf.json.'
  );
} else if (!isTauri) {
  console.warn(
    '[BlastWA EXPLICIT MOCK MODE] no Tauri runtime detected (plain browser dev). ' +
    'All invoke() calls return labeled mocks and never reach Rust.'
  );
}

export const invoke = isTauri
  ? window.__TAURI__.core.invoke
  : uaIsTauri
    ? // desktop build, bridge missing: fail loudly instead of mock success
      async (cmd) => {
        throw new Error(
          `Tauri IPC bridge unavailable in desktop build (window.__TAURI__ missing). ` +
          `invoke('${cmd}') cannot reach Rust. Verify app.withGlobalTauri in tauri.conf.json.`
        );
      }
    : // plain browser dev: explicitly labeled mock, announced at startup
      async (cmd, args = {}) => {
        console.warn(`[BlastWA MOCK] invoke('${cmd}') mocked (browser dev mode, not real IPC)`);
        return { mock: true, mockLabeled: true, cmd, args };
      };

const IPC_COMMANDS = Object.freeze({
  accounts: 'list_accounts',
  campaignStatus: 'get_status',
  addAccount: 'add_account',
  removeAccount: 'remove_account',
  removeAllAccounts: 'remove_all_accounts',
  renameAccount: 'rename_account',
  openBrowser: 'open_browser',
});

async function invokeCommand(command, args = {}) {
  return invoke(IPC_COMMANDS[command] || command, args);
}

function normalizeAccounts(value) {
  const list = Array.isArray(value) ? value : value?.accounts;
  return Array.isArray(list) ? list.filter((account) => account && typeof account === 'object') : [];
}

function normalizeCampaignStatus(value) {
  if (!value || typeof value !== 'object' || value.mock) {
    return { running: false, sent: 0, failed: 0, mock: !!value?.mock };
  }
  return {
    running: value.running === true,
    sent: Number.isFinite(value.sent) ? value.sent : 0,
    failed: Number.isFinite(value.failed) ? value.failed : 0,
  };
}

async function getAccounts() {
  return normalizeAccounts(await invokeCommand('accounts'));
}

async function getCampaignStatus() {
  return normalizeCampaignStatus(await invokeCommand('campaignStatus'));
}

// ----- page-scoped listener lifecycle -----

let navEpoch = 0;          // increments on every navigation
let pageCleanups = [];     // unlisten fns for the CURRENT page only
let injectedPageScripts = [];

function runPageCleanups() {
  for (const fn of pageCleanups) {
    try { fn(); } catch (e) { console.warn('page cleanup failed', e); }
  }
  pageCleanups = [];
  for (const script of injectedPageScripts) script.remove();
  injectedPageScripts = [];
}

// register any page-scoped teardown (e.g. clearing an interval) that should
// run when the current page is navigated away from
export function addCleanup(fn) {
  if (typeof fn === 'function') pageCleanups.push(fn);
}

export async function listen(event, handler) {
  if (isTauri && window.__TAURI__.event) {
    const epoch = navEpoch;
    const unlisten = await window.__TAURI__.event.listen(event, handler);
    if (epoch === navEpoch) {
      // still on the page that asked for it: clean up on next navigation
      pageCleanups.push(() => { try { unlisten(); } catch (e) {} });
    } else {
      // user navigated away before registration finished: drop immediately
      try { unlisten(); } catch (e) {}
    }
    return unlisten;
  }
  console.log(`[mock listen] ${event}`, 'handler registered');
  return () => {};
}

const PAGES = [
  'dashboard', 'sending', 'contacts', 'groups',
  'autoreply', 'templates', 'log', 'settings',
];

const contentEl = document.getElementById('content');
const navEl = document.getElementById('nav');

function setActive(page) {
  navEl.querySelectorAll('.tool-item').forEach((btn) => {
    btn.classList.toggle('active', btn.dataset.page === page);
  });
}

async function route() {
  const hash = location.hash.replace('#/', '') || 'dashboard';
  const page = PAGES.includes(hash) ? hash : 'dashboard';
  navEpoch++;
  const routeEpoch = navEpoch;
  setActive(page);

  // new navigation: invalidate in-flight listener registrations and
  // release the previous page's event listeners
  runPageCleanups();

  try {
    const res = await fetch(`pages/${page}.html`);
    const html = await res.text();
    if (routeEpoch !== navEpoch) return;
    contentEl.innerHTML = html;
    contentEl.scrollTop = 0;

    // innerHTML marks injected <script> tags as already-started, so they
    // never run. re-create each one as a fresh script element; appending
    // to the DOM executes it synchronously, preserving document order.
    for (const old of [...contentEl.querySelectorAll('script')]) {
      const s = document.createElement('script');
      if (old.src) {
        s.src = old.src;
      } else {
        s.textContent = old.textContent;
      }
      document.body.appendChild(s);
      injectedPageScripts.push(s);
      old.remove(); // executed copy lives in body; keep content clean
    }

    // init hook: exactly once per page load. an init failure must not
    // wipe the already-rendered page.
    const initFn = window[`init_${page}`];
    if (typeof initFn === 'function') {
      try {
        const epoch = navEpoch;
        await initFn();
        if (epoch !== navEpoch) return;
      } catch (e) {
        console.error(`init_${page} failed:`, e);
      }
    }
  } catch (e) {
    contentEl.innerHTML = `<div class="empty-state">Failed to load page: ${e.message}</div>`;
  }
}

// toolbar navigation
navEl.addEventListener('click', (ev) => {
  const btn = ev.target.closest('.tool-item');
  if (!btn) return;
  location.hash = `#/${btn.dataset.page}`;
});

// ----- menubar behavior -----
const menubar = document.getElementById('menubar');

menubar.addEventListener('click', (ev) => {
  const item = ev.target.closest('.menu-item');
  const btn = ev.target.closest('.menu-dropdown button');

  if (btn) {
    // close menu, then run action
    menubar.querySelectorAll('.menu-item.open').forEach((m) => m.classList.remove('open'));
    const action = btn.dataset.action;
    if (action === 'nav') {
      location.hash = `#/${btn.dataset.page}`;
    } else if (action === 'exit') {
      if (isTauri && window.__TAURI__.window) {
        window.__TAURI__.window.getCurrentWindow().close();
      }
    } else if (action === 'about') {
      alert('BlastWA v0.2.0\nWhatsApp bulk sender.\nRust + Tauri v2 + Chrome CDP.\nNo license, ever.');
    } else if (action === 'new-profile') {
      openProfileModal();
    }
    return;
  }

  if (item) {
    const wasOpen = item.classList.contains('open');
    menubar.querySelectorAll('.menu-item.open').forEach((m) => m.classList.remove('open'));
    if (!wasOpen) item.classList.add('open');
  }
});

// click elsewhere closes menus; hover moves between menus when one is open
document.addEventListener('click', (ev) => {
  if (!ev.target.closest('.menubar')) {
    menubar.querySelectorAll('.menu-item.open').forEach((m) => m.classList.remove('open'));
  }
});
menubar.addEventListener('mouseover', (ev) => {
  const item = ev.target.closest('.menu-item');
  if (!item) return;
  if (menubar.querySelector('.menu-item.open')) {
    menubar.querySelectorAll('.menu-item.open').forEach((m) => m.classList.remove('open'));
    item.classList.add('open');
  }
});

// ----- profile launcher modal (U2) -----
// lives outside the page router: one instance in index.html, wired once at
// module scope, never re-created on navigation

const profileModal = document.getElementById('profile-modal');

function showProfileError(msg) {
  const el = document.getElementById('profile-error');
  if (!msg) {
    el.classList.add('hidden');
    el.textContent = '';
  } else {
    el.textContent = msg;
    el.classList.remove('hidden');
  }
}

async function refreshProfileChips() {
  const wrap = document.getElementById('profile-chips');
  const listWrap = document.getElementById('profile-list-wrap');
  let names = [];
  try {
    names = await invoke('list_profiles');
    if (!Array.isArray(names)) names = [];
  } catch (e) {
    names = [];
  }
  if (!names.length) {
    listWrap.style.display = 'none';
    return;
  }
  listWrap.style.display = '';
  wrap.innerHTML = '';
  for (const name of names) {
    const chip = document.createElement('button');
    chip.type = 'button';
    chip.className = 'profile-chip';
    chip.textContent = name;
    chip.addEventListener('click', () => {
      document.getElementById('profile-name').value = name;
      showProfileError('');
    });
    wrap.appendChild(chip);
  }
}

function openProfileModal() {
  document.getElementById('profile-name').value = '';
  showProfileError('');
  profileModal.classList.remove('hidden');
  refreshProfileChips();
  document.getElementById('profile-name').focus();
}

function closeProfileModal() {
  profileModal.classList.add('hidden');
}

async function submitProfile() {
  const raw = document.getElementById('profile-name').value.trim();
  const createShortcut = document.getElementById('profile-create-shortcut')?.checked ?? false;
  if (!raw) {
    showProfileError('Profile name is required.');
    return;
  }
  try {
    await invoke('open_profile_window', { profile: raw, createShortcut });
    closeProfileModal();
  } catch (e) {
    showProfileError(e.message || String(e));
  }
}

document.getElementById('profile-cancel').addEventListener('click', closeProfileModal);
document.getElementById('profile-open').addEventListener('click', submitProfile);
document.getElementById('profile-name').addEventListener('keydown', (ev) => {
  if (ev.key === 'Enter') submitProfile();
});
profileModal.addEventListener('click', (ev) => {
  // click on the dim backdrop closes; clicks inside the panel do not
  if (ev.target === profileModal) closeProfileModal();
});

// ----- status bar updater (module scope: created exactly once) -----
async function refreshStatus() {
  const $ = (id) => document.getElementById(id);
  try {
    const list = await getAccounts();
    const connected = list.find((a) => a.connected);
    const waiting = list.find((a) => a.browser_running && !a.wa_authenticated);
    if (connected) {
      $('sb-dot').classList.remove('off');
      $('sb-conn').textContent = 'Online';
      $('sb-account').textContent = `Account: ${connected.number || connected.name}`;
    } else if (waiting) {
      $('sb-dot').classList.add('off');
      $('sb-conn').textContent = 'Waiting for scan';
      $('sb-account').textContent = `Account: ${waiting.name}`;
    } else {
      $('sb-dot').classList.add('off');
      $('sb-conn').textContent = 'Not connected';
      $('sb-account').textContent = 'Account: -';
    }
    const st = await getCampaignStatus();
    const campaignRunning = st.running;
    $('sb-login').textContent = campaignRunning
      ? 'Sending...'
      : connected
        ? 'Logged In'
        : waiting
          ? 'Waiting for scan'
          : 'Logged Out';
  } catch (e) {
    // keep last known state on transient errors
  }
}
setInterval(refreshStatus, 3000);

window.addEventListener('hashchange', route);
window.addEventListener('DOMContentLoaded', () => {
  route();
  refreshStatus();
});

// timestamped default filename for save dialogs (U7):
// stampName('blastwa-groups', 'csv') -> 'blastwa-groups-2026-08-25-1442.csv'
function stampName(base, ext) {
  const d = new Date();
  const p = (n) => String(n).padStart(2, '0');
  const stamp = `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}-` +
    `${p(d.getHours())}${p(d.getMinutes())}`;
  return `${base}-${stamp}.${ext}`;
}

// escape a value for safe interpolation into innerHTML templates.
// single shared spelling; pages alias it via window.blastwa.esc.
function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  })[c]);
}

// shared helpers usable by pages
const legacyHelpers = { stampName, esc };

// in-page replacement for window.confirm. the dialog plugin shims confirm
// into an async invoke of `plugin:dialog|confirm`, a command that no longer
// exists on its rust side (only open/save/message are registered), so every
// shimmed confirm rejects and guarded actions silently no-op. this overlay
// uses the same modal styling as the add-account dialog and resolves a
// boolean, so `await uiConfirm(...)` behaves like confirm() should.
window.uiConfirm = function (message, opts = {}) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'modal-overlay';
    overlay.innerHTML =
      '<div class="modal-panel">' +
        '<div class="panel-header"><span>' + (opts.title || 'Confirm') + '</span></div>' +
        '<div class="modal-body"><p id="ui-confirm-text" style="margin:0"></p></div>' +
        '<div class="modal-footer">' +
          '<button class="btn btn-secondary" id="ui-confirm-cancel">Cancel</button>' +
          '<button class="btn" id="ui-confirm-ok">OK</button>' +
        '</div>' +
      '</div>';
    overlay.querySelector('#ui-confirm-text').textContent = message;
    document.body.appendChild(overlay);

    const done = (value) => {
      document.removeEventListener('keydown', onKey);
      overlay.remove();
      resolve(value);
    };
    const onKey = (ev) => {
      if (ev.key === 'Escape') done(false);
      if (ev.key === 'Enter') done(true);
    };
    overlay.querySelector('#ui-confirm-ok').onclick = () => done(true);
    overlay.querySelector('#ui-confirm-cancel').onclick = () => done(false);
    overlay.addEventListener('click', (ev) => { if (ev.target === overlay) done(false); });
    document.addEventListener('keydown', onKey);
    overlay.querySelector('#ui-confirm-ok').focus();
  });
};

window.blastwa = { ...legacyHelpers,
  invoke,
  invokeCommand,
  getAccounts,
  getCampaignStatus,
  normalizeAccounts,
  normalizeCampaignStatus,
  listen,
  isTauri,
  addCleanup,
};
