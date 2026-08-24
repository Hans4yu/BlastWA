// BlastWA router + Tauri IPC helpers. vanilla js, no build step.
// shell: menubar + toolbar + content + statusbar (oke sender-style chrome).

const isTauri = !!window.__TAURI__;
export const invoke = window.__TAURI__
  ? window.__TAURI__.core.invoke
  : async (cmd, args = {}) => ({ mock: true, cmd, args });

export async function listen(event, handler) {
  if (isTauri && window.__TAURI__.event) {
    await window.__TAURI__.event.listen(event, handler);
  } else {
    console.log(`[mock listen] ${event}`, 'handler registered');
  }
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
  setActive(page);

  try {
    const res = await fetch(`pages/${page}.html`);
    contentEl.innerHTML = await res.text();
    contentEl.scrollTop = 0;

    // per-page init hook if defined by that page's script tag
    const initFn = window[`init_${page}`];
    if (typeof initFn === 'function') initFn();
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

// ----- status bar updater -----
async function refreshStatus() {
  const $ = (id) => document.getElementById(id);
  try {
    const accounts = await invoke('list_accounts');
    const list = Array.isArray(accounts) ? accounts : [];
    const connected = list.find((a) => a.connected);
    if (connected) {
      $('sb-dot').classList.remove('off');
      $('sb-conn').textContent = 'Connected';
      $('sb-account').textContent = `Account: ${connected.number || connected.name}`;
    } else {
      $('sb-dot').classList.add('off');
      $('sb-conn').textContent = 'Not connected';
      $('sb-account').textContent = 'Account: -';
    }
    const st = await invoke('get_status');
    if (st && !st.mock) {
      $('sb-login').textContent = st.running ? 'Sending...' : 'Logged In';
    } else {
      $('sb-login').textContent = 'Logged In';
    }
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

// shared helpers usable by pages
window.blastwa = { invoke, listen, isTauri };
