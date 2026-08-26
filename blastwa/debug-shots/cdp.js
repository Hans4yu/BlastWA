// CDP capture runner: node cdp.js <page> <out.png> [action]
// pages: dashboard contacts groups sending templates autoreply log settings
// actions: addmodal (open the add-account modal before shooting)
//          profilemodal (open the index-level profile launcher)
// drives the shell on port 9223 (launch blastwa.exe with
// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223")
'use strict';
const http = require('http');
const fs = require('path');

const [,, page, out, action] = process.argv;
if (!page || !out) { console.error('usage: node cdp.js <page> <out.png> [addmodal|profilemodal]'); process.exit(1); }

function getList() {
  return new Promise((res, rej) => {
    http.get({ host: '127.0.0.1', port: 9223, path: '/json/list' }, (r) => {
      let d = ''; r.on('data', (c) => d += c);
      r.on('end', () => res(JSON.parse(d)));
    }).on('error', rej);
  });
}

(async () => {
  const pages = await getList();
  const target = pages.find((p) => p.type === 'page');
  if (!target) { console.error('no page target'); process.exit(1); }
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  let id = 0; const pend = {};
  ws.onmessage = (e) => { const m = JSON.parse(e.data); if (m.id && pend[m.id]) pend[m.id](m); };
  const send = (method, params) => new Promise((r) => { const i = ++id; pend[i] = r; ws.send(JSON.stringify({ id: i, method, params })); });
  await new Promise((r) => ws.onopen = r);
  const evl = async (expr) => (await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true })).result.result.value;

  await evl(`location.hash = '#/${page}'; 1`);
  await new Promise((r) => setTimeout(r, 900));

  if (action === 'addmodal') {
    await evl(`openAddAccount(); 1`);
    await new Promise((r) => setTimeout(r, 300));
  } else if (action === 'profilemodal') {
    await evl(`document.getElementById('profile-modal').classList.remove('hidden'); 1`);
    await new Promise((r) => setTimeout(r, 300));
  }

  const shot = await send('Page.captureScreenshot', { format: 'png' });
  require('fs').writeFileSync(out, Buffer.from(shot.result.data, 'base64'));
  console.log('saved', out);
  ws.close();
  process.exit(0);
})().catch((e) => { console.error(e); process.exit(1); });
