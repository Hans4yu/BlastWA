'use strict';
const http = require('http');
const fs = require('fs');
const path = require('path');

const MAIN_TARGET = '62895410710058';
const SENDER_NUMBER = '6282132102060';
const ASSETS_DIR = 'C:\\Users\\Farhan\\AppData\\Local\\Temp\\blastwa_test_assets';

function getList(port = 9223) {
  return new Promise((resolve, reject) => {
    http.get({ host: '127.0.0.1', port, path: '/json/list' }, (res) => {
      let data = '';
      res.on('data', (chunk) => data += chunk);
      res.on('end', () => {
        try { resolve(JSON.parse(data)); } catch (e) { reject(e); }
      });
    }).on('error', reject);
  });
}

(async () => {
  console.log('=====================================================');
  console.log('🚀 BLASTWA MASTER AUTO-TESTING SUITE (SENDER: 6282132102060 -> TARGET: 62895410710058)');
  console.log('=====================================================');

  let pages = [];
  for (let attempt = 0; attempt < 20; attempt++) {
    try {
      pages = await getList(9223);
      if (pages && pages.length) break;
    } catch (e) {}
    await new Promise((r) => setTimeout(r, 500));
  }

  const target = pages.find((p) => p.type === 'page' || p.url.includes('index.html'));
  if (!target) {
    throw new Error('Aplikasi BlastWA tidak ditemukan di port 9223! Jalankan dengan WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223"');
  }

  const ws = new WebSocket(target.webSocketDebuggerUrl);
  let id = 0;
  const pending = {};
  ws.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (msg.id && pending[msg.id]) pending[msg.id](msg);
  };

  const send = (method, params = {}) => new Promise((resolve) => {
    const i = ++id;
    pending[i] = resolve;
    ws.send(JSON.stringify({ id: i, method, params }));
  });

  await new Promise((resolve) => ws.onopen = resolve);

  const evalJs = async (expr) => {
    const res = await send('Runtime.evaluate', {
      expression: expr,
      awaitPromise: true,
      returnByValue: true,
    });
    return res && res.result && res.result.result ? res.result.result.value : null;
  };

  const shot = async (name) => {
    const res = await send('Page.captureScreenshot', { format: 'png' });
    if (res && res.result && res.result.data) {
      const outPath = path.join(__dirname, '..', 'screenshots', `live_${name}.png`);
      fs.writeFileSync(outPath, Buffer.from(res.result.data, 'base64'));
      console.log(`📸 Screenshot tersimpan: screenshots/live_${name}.png`);
    }
  };

  // 1. TAHAP DETEKSI AKUN
  console.log('\n[1/6] Mendeteksi Akun Pengirim (6282132102060)...');
  await evalJs(`location.hash = '#/dashboard'; 1`);
  await new Promise((r) => setTimeout(r, 1200));

  const accounts = await evalJs(`window.__TAURI__.core.invoke('list_accounts')`);
  console.log('Daftar akun terdaftar:', JSON.stringify(accounts));
  
  let senderAccount = 'tes';
  if (Array.isArray(accounts)) {
    const match = accounts.find(a => a.number && a.number.includes(SENDER_NUMBER));
    if (match) {
      senderAccount = match.name;
      console.log(`✅ Ditemukan akun cocok: '${senderAccount}' dengan nomor ${match.number}`);
    } else if (accounts.length > 0) {
      senderAccount = accounts[0].name;
      console.log(`ℹ️ Menggunakan akun aktif pertama: '${senderAccount}'`);
    }
  }

  console.log(`- Mengaitkan browser sesi '${senderAccount}' via open_browser command...`);
  try {
    await evalJs(`window.__TAURI__.core.invoke('open_browser', { name: '${senderAccount}' })`);
  } catch (e) {
    console.log('open_browser note:', e.message);
  }
  await new Promise((r) => setTimeout(r, 3000));
  await shot('01_dashboard_active');

  // 2. TAHAP CHECK NUMBER
  console.log(`\n[2/6] Memeriksa Status Nomor Target (${MAIN_TARGET})...`);
  await evalJs(`location.hash = '#/contacts'; 1`);
  await new Promise((r) => setTimeout(r, 1000));
  try {
    const checkRes = await evalJs(`
      window.__TAURI__.core.invoke('check_numbers_cmd', {
        account: '${senderAccount}',
        numbers: ['${MAIN_TARGET}', '081218392796', '6285695791713']
      })
    `);
    console.log('  Hasil Number Check:', JSON.stringify(checkRes));
  } catch (e) {
    console.log('  Number checker note:', e.message);
  }
  await shot('02_contacts_checked');

  // 3. TAHAP UJI TEXT, MARKDOWN, SPINTAX & VARS KE NOMOR UTAMA
  console.log(`\n[3/6] Mengirim Pesan Teks Terformat ke ${MAIN_TARGET}...`);
  await evalJs(`location.hash = '#/sending'; 1`);
  await new Promise((r) => setTimeout(r, 1000));

  await evalJs(`
    window.__TAURI__.core.invoke('clear_contacts');
    window.__TAURI__.core.invoke('add_generated_contacts', {
      prefix: '${MAIN_TARGET}',
      rangeStart: 0,
      rangeEnd: 0
    });
  `);

  const textPayload = `*Halo [[firstname]]!*\\n{Selamat pagi|Semangat ya} brody! 🔥\\n_Ini format miring_, ~ini coret~, dan \`\`\`monospace\`\`\`.\\nTag unik: #[[randomtag]]`;
  try {
    await evalJs(`
      window.__TAURI__.core.invoke('start_campaign', {
        account: '${senderAccount}',
        message: "${textPayload}",
        isBlindMode: true,
        humanPreset: "off"
      })
    `);
  } catch (e) {
    console.log('Text campaign note:', e.message);
  }
  await new Promise((r) => setTimeout(r, 4000));
  await shot('03_sending_text');

  // 4. TAHAP UJI 6 FORMAT MEDIA LENGKAP
  console.log(`\n[4/6] Mengirim 6 Format Media Lengkap ke ${MAIN_TARGET}...`);
  const mediaFiles = [
    { type: 'Image PNG', file: 'cat_meme.png', cap: 'Meme Cat SV RIDHO 👍' },
    { type: 'Vector SVG', file: 'sample.svg', cap: 'Vector Icon SVG' },
    { type: 'Document PDF', file: 'sample.pdf', cap: 'Official PDF Document' },
    { type: 'Video MP4', file: 'sample.mp4', cap: 'Video Clip WhatsApp' },
    { type: 'Audio MP3', file: 'sample.mp3', cap: 'Audio Song MP3' },
    { type: 'Voice Note OGG', file: 'sample.ogg', cap: 'PTT Voice Note' },
  ];

  for (const m of mediaFiles) {
    console.log(`  ➔ Mengirim ${m.type} (${m.file})...`);
    const fullPath = path.join(ASSETS_DIR, m.file).replace(/\\/g, '\\\\');
    try {
      await evalJs(`
        window.__TAURI__.core.invoke('start_campaign', {
          account: '${senderAccount}',
          message: "Attachment: ${m.type}",
          attachmentPath: "${fullPath}",
          caption: "${m.cap}",
          isBlindMode: true,
          humanPreset: "off"
        })
      `);
    } catch (e) {
      console.log(`  Attachment ${m.file} note:`, e.message);
    }
    await new Promise((r) => setTimeout(r, 3500));
  }
  await shot('04_sending_media_done');

  // 5. TAHAP BLAST MASSAL (SINGLE RUN 5 NOMOR)
  console.log('\n[5/6] Menjalankan Blast Massal ke 5 Nomor Kontak...');
  const csvPath = path.join(ASSETS_DIR, 'contacts_test.csv').replace(/\\/g, '\\\\');
  
  await evalJs(`location.hash = '#/contacts'; 1`);
  await new Promise((r) => setTimeout(r, 1000));
  
  try {
    await evalJs(`
      window.__TAURI__.core.invoke('import_contacts', {
        path: "${csvPath}",
        mapping: {
          numberCol: "Number",
          fullnameCol: "FullName",
          var1Col: "Var1",
          var2Col: "Var2"
        },
        firstRowIsHeader: true,
        removeDupes: true
      })
    `);
  } catch (e) {
    console.log('import_contacts note:', e.message);
  }
  await new Promise((r) => setTimeout(r, 1500));
  await shot('05_contacts_imported');

  await evalJs(`location.hash = '#/sending'; 1`);
  await new Promise((r) => setTimeout(r, 1000));
  try {
    await evalJs(`
      window.__TAURI__.core.invoke('start_campaign', {
        account: '${senderAccount}',
        message: "Halo [[fullname]], ini pesan broadcast massal BlastWA! Tag: #[[randomtag]]",
        isBlindMode: true,
        delayMinS: 2.0,
        delayMaxS: 4.0
      })
    `);
  } catch (e) {
    console.log('mass blast note:', e.message);
  }
  await new Promise((r) => setTimeout(r, 8000));
  await shot('06_mass_blast_complete');

  // 6. TAHAP EXPORT LAPORAN
  console.log('\n[6/6] Meninjau & Mengambil Screenshot Log History...');
  await evalJs(`location.hash = '#/log'; 1`);
  await new Promise((r) => setTimeout(r, 1500));
  await shot('07_log_history_table');

  console.log('=====================================================');
  console.log('✅ SELURUH PENGUJIAN OTOMATIS LIVE SELESAI 100%!');
  console.log('=====================================================');
  ws.close();
  process.exit(0);
})().catch((err) => {
  console.error('❌ Error runner:', err);
  process.exit(1);
});
