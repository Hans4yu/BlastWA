// OKESENDER.exe string extractor for U19 research (UTF-16LE + ASCII)
const fs = require('fs');
const buf = fs.readFileSync(process.argv[2] || 'D:/Tes/OKESENDER.exe');

function extractUtf16(minLen) {
  const out = [];
  let cur = '';
  for (let i = 0; i + 1 < buf.length; i += 2) {
    const c = buf[i] | (buf[i + 1] << 8);
    if (c >= 0x20 && c < 0xfffd && !(c >= 0x80 && c < 0xa0)) {
      cur += String.fromCharCode(c);
    } else {
      if (cur.length >= minLen) out.push(cur);
      cur = '';
    }
  }
  return out;
}

const terms = (process.argv[3] || 'Familiar,Advanced,Blind,Button,List,Catalog').split(',');
for (const s of extractUtf16(4)) {
  if (terms.some((t) => new RegExp(t, 'i').test(s))) console.log(s);
}
