// BlastWA router + Tauri IPC helpers. vanilla js, no build step.
// works standalone in a browser (mock invoke) and inside tauri v2 (real ipc).

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
  navEl.querySelectorAll('.nav-item').forEach((btn) => {
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

    // per-page init hook if defined by that page's script tag
    const initFn = window[`init_${page}`];
    if (typeof initFn === 'function') initFn();
  } catch (e) {
    contentEl.innerHTML = `<div class="empty-state">Failed to load page: ${e.message}</div>`;
  }
}

navEl.addEventListener('click', (ev) => {
  const btn = ev.target.closest('.nav-item');
  if (!btn) return;
  location.hash = `#/${btn.dataset.page}`;
});

window.addEventListener('hashchange', route);
window.addEventListener('DOMContentLoaded', route);

// shared helpers usable by pages
window.blastwa = { invoke, listen, isTauri };
