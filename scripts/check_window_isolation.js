// profile isolation guardrail: the main window MUST be built in rust with a
// per-profile webview data_directory. tauri.conf.json windows were moved out
// because on windows tauri forces every instance onto
// AppData\Local\{identifier} unless a data_directory is set — which made the
// default and all profiles share one localStorage (groups cache, checker
// cache, compose draft) and clobber each other while running side by side.
// this file exists so a revert to config-declared windows gets caught.
const fs = require('fs');
const path = require('path');

let failures = 0;
function assert(name, cond) {
  console.log(`${cond ? 'PASS' : 'FAIL'} ${name}`);
  if (!cond) failures++;
}

const conf = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'src-tauri', 'tauri.conf.json'), 'utf8'));
assert('tauri.conf.json declares no static windows (rust builds them)',
  Array.isArray(conf.app.windows) && conf.app.windows.length === 0);

const mainRs = fs.readFileSync(path.join(__dirname, '..', 'src-tauri', 'src', 'main.rs'), 'utf8');
assert('main window is built via WebviewWindowBuilder',
  /WebviewWindowBuilder::new\(app,\s*"main"/.test(mainRs));
assert('builder sets inner_size and min_inner_size (mirrors the old config)',
  /\.inner_size\(1280\.0,\s*820\.0\)/.test(mainRs) &&
  /\.min_inner_size\(980\.0,\s*640\.0\)/.test(mainRs));
assert('profile instances get an isolated webview data_directory',
  /data_directory\(dir\)/.test(mainRs) &&
  /AppConfig::app_dir\(\)\.join\("webview"\)/.test(mainRs));
assert('data_directory applies ONLY to profile instances (default keeps tauri default storage)',
  /if let Some\(p\) = AppConfig::active_profile\(\)/.test(mainRs) &&
  mainRs.indexOf('data_directory(dir)') > mainRs.indexOf('if let Some(p) = AppConfig::active_profile()'));

// contacts persistence: the send list must survive restarts
const contactsRs = fs.readFileSync(path.join(__dirname, '..', 'src-tauri', 'src', 'commands', 'contacts.rs'), 'utf8');
assert('every contacts mutation persists through persist_contacts',
  (contactsRs.match(/persist_contacts\(&ctx\)/g) || []).length >= 5);
assert('contacts restore is wired at startup',
  /load_json\(&contacts_path\)/.test(mainRs) &&
  /contacts\.json/.test(mainRs));

console.log('');
console.log(failures ? `${failures} FAILURES` : 'ALL WINDOW ISOLATION CHECKS PASSED');
process.exit(failures ? 1 : 0);
