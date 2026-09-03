'use strict';
const http = require('http');
const fs = require('fs');
const path = require('path');

const [,, page, out, action] = process.argv;
if (!page || !out) {
  console.error('usage: node capture_ui.js <page> <out.png> [addmodal|profilemodal]');
  process.exit(1);
}

function getList() {
  return new Promise((resolve, reject) => {
    http.get({ host: '127.0.0.1', port: 9223, path: '/json/list' }, (res) => {
      let data = '';
      res.on('data', (chunk) => data += chunk);
      res.on('end', () => {
        try {
          resolve(JSON.parse(data));
        } catch (e) {
          reject(e);
        }
      });
    }).on('error', reject);
  });
}

(async () => {
  let pages = [];
  for (let i = 0; i < 20; i++) {
    try {
      pages = await getList();
      if (pages && pages.length) break;
    } catch (e) {}
    await new Promise((r) => setTimeout(r, 500));
  }

  const target = pages.find((p) => p.type === 'page' || p.url.includes('index.html') || p.devtoolsFrontendUrl);
  if (!target) {
    console.error('no page target found on port 9223');
    process.exit(1);
  }

  const wsUrl = target.webSocketDebuggerUrl;
  const ws = new WebSocket(wsUrl);
  let id = 0;
  const pending = {};

  ws.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (msg.id && pending[msg.id]) {
      pending[msg.id](msg);
    }
  };

  const send = (method, params = {}) => new Promise((resolve) => {
    const i = ++id;
    pending[i] = resolve;
    ws.send(JSON.stringify({ id: i, method, params }));
  });

  await new Promise((resolve) => ws.onopen = resolve);

  const evaluate = async (expr) => {
    const res = await send('Runtime.evaluate', {
      expression: expr,
      awaitPromise: true,
      returnByValue: true,
    });
    return res && res.result && res.result.result ? res.result.result.value : null;
  };

  await evaluate(`location.hash = '#/${page}'; 1`);
  await new Promise((r) => setTimeout(r, 1000));

  if (action === 'addmodal') {
    await evaluate(`openAddAccount(); 1`);
    await new Promise((r) => setTimeout(r, 300));
  } else if (action === 'profilemodal') {
    await evaluate(`document.getElementById('profile-modal').classList.remove('hidden'); 1`);
    await new Promise((r) => setTimeout(r, 300));
  }

  const shot = await send('Page.captureScreenshot', { format: 'png' });
  if (shot && shot.result && shot.result.data) {
    const outDir = path.dirname(path.resolve(out));
    if (!fs.existsSync(outDir)) fs.mkdirSync(outDir, { recursive: true });
    fs.writeFileSync(out, Buffer.from(shot.result.data, 'base64'));
    console.log('Successfully saved screenshot:', out);
  } else {
    console.error('Failed to capture screenshot');
  }

  ws.close();
  process.exit(0);
})().catch((err) => {
  console.error('Error during capture:', err);
  process.exit(1);
});
