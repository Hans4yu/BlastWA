#![windows_subsystem = "windows"]

use std::path::{Path, PathBuf};
use std::process::Command;
use winreg::enums::*;
use winreg::RegKey;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const APP_NAME: &str = "BlastWA";
const APP_TITLE: &str = "BlastWA Setup Wizard";
const APP_VERSION: &str = "0.2.0";
const APP_PUBLISHER: &str = "BlastWA Team";
const MIN_CHROME_MAJOR: u32 = 115;

const IDC_BTN_NEXT: isize = 1001;
const IDC_BTN_BACK: isize = 1002;
const IDC_BTN_CANCEL: isize = 1003;
const IDC_CHK_DESKTOP: isize = 1004;
const IDC_CHK_STARTMENU: isize = 1005;
const IDC_CHK_LAUNCH: isize = 1006;
const IDC_EDIT_PATH: isize = 1007;
const IDC_PROGRESS: isize = 1008;
const IDC_BTN_BROWSE: isize = 1009;

const ICON_RAW_BYTES: &[u8] = include_bytes!("../../src-tauri/icons/128x128.png");

#[derive(Clone, Copy, PartialEq)]
enum WizardStep {
    Welcome,
    ChooseLocation,
    SelectOptions,
    Installing,
    Finish,
}

static mut CURRENT_STEP: WizardStep = WizardStep::Welcome;
static mut HWND_MAIN: HWND = 0;
static mut HWND_NEXT: HWND = 0;
static mut HWND_BACK: HWND = 0;
static mut HWND_CANCEL: HWND = 0;
static mut HWND_CHK_DESKTOP: HWND = 0;
static mut HWND_CHK_STARTMENU: HWND = 0;
static mut HWND_CHK_LAUNCH: HWND = 0;
static mut HWND_EDIT_PATH: HWND = 0;
static mut HWND_BTN_BROWSE: HWND = 0;
static mut HWND_PROGRESS: HWND = 0;
static mut FONT_TITLE: HFONT = 0;
static mut FONT_BODY: HFONT = 0;
static mut HICON_APP: HICON = 0;

static mut OPT_DESKTOP: bool = true;
static mut OPT_STARTMENU: bool = true;
static mut OPT_LAUNCH: bool = true;

// ---------- uninstall wizard ----------
// same window shell as the installer; UNINSTALL_MODE switches the page flow

#[derive(Clone, Copy, PartialEq)]
enum UninstallStep {
    Welcome,
    Options,
    Working,
    Done,
}

const IDC_CHK_DATA: isize = 1010;
const IDC_STEP_LABEL: isize = 1011;

static mut UNINSTALL_MODE: bool = false;
static mut UNINSTALL_STEP: UninstallStep = UninstallStep::Welcome;
static mut UNINSTALL_KEEP_DATA: bool = true;
static mut UNINSTALL_DIR: Option<PathBuf> = None;
static mut SWEEPER_SPAWNED: bool = false;
static mut HWND_CHK_DATA: HWND = 0;
static mut HWND_STEP_LABEL: HWND = 0;

/// clone the stored dir through a raw pointer so the shared-reference
/// lint (static_mut_refs) stays quiet; single-threaded GUI app
unsafe fn uninstall_dir_snapshot() -> Option<PathBuf> {
    (*std::ptr::addr_of!(UNINSTALL_DIR)).clone()
}

fn resolve_uninstall_dir() -> PathBuf {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let uninstall_path = format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{}", APP_NAME);
    hkcu.open_subkey(&uninstall_path)
        .ok()
        .and_then(|key| key.get_value::<String, _>("InstallLocation").ok())
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok().and_then(|path| path.parent().map(Path::to_path_buf)))
        .unwrap_or_else(default_install_dir)
}

fn kill_running_app() {
    let mut kill = Command::new("taskkill");
    kill.args(["/F", "/IM", "blastwa.exe"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        kill.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = kill.spawn();
}

fn remove_profile_shortcuts() {
    if let Some(desktop) = dirs::desktop_dir() {
        let _ = std::fs::remove_file(desktop.join("BlastWA.lnk"));
        if let Ok(entries) = std::fs::read_dir(&desktop) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_profile_shortcut = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.starts_with("BlastWA - ") && name.ends_with(".lnk"))
                    .unwrap_or(false);
                if is_profile_shortcut {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }

    if let Some(data_dir) = dirs_localappdata() {
        let start_menu = data_dir
            .join("..")
            .join("Roaming")
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        let _ = std::fs::remove_file(start_menu.join("BlastWA.lnk"));
    }
}

fn remove_dir_with_retry(path: &Path, attempts: u32) -> bool {
    for _ in 0..attempts {
        if !path.exists() {
            return true;
        }
        if std::fs::remove_dir_all(path).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    !path.exists()
}

/// write the detached sweeper that removes the install dir after this
/// process exits (the running uninstaller locks its own exe). optionally
/// also retries the data dir when a live chrome profile kept it locked.
/// must be spawned with its working directory OUTSIDE the removed tree.
unsafe fn spawn_uninstall_sweeper(delete_data: bool) {
    if SWEEPER_SPAWNED {
        return;
    }
    let install_dir = unsafe { uninstall_dir_snapshot().unwrap_or_else(resolve_uninstall_dir) };
    let mut lines = String::from("@echo off\r\nfor /l %%i in (1,1,120) do (\r\n");
    if delete_data {
        lines.push_str(&format!(
            "  rmdir /s /q \"{}\" >nul 2>&1\r\n",
            app_data_dir().to_string_lossy().replace('"', "")
        ));
    }
    lines.push_str(&format!(
        "  rmdir /s /q \"{}\" >nul 2>&1\r\n  if not exist \"{}\" del \"%~f0\" >nul 2>&1\r\n  if not exist \"{}\" exit\r\n  ping -n 2 127.0.0.1 >nul\r\n)\r\ndel \"%~f0\" >nul 2>&1\r\n",
        install_dir.to_string_lossy().replace('"', ""),
        install_dir.to_string_lossy().replace('"', ""),
        install_dir.to_string_lossy().replace('"', ""),
    ));
    let sweeper = std::env::temp_dir().join("blastwa_uninstall_sweeper.cmd");
    if std::fs::write(&sweeper, lines).is_ok() {
        let mut cleanup = Command::new("cmd.exe");
        cleanup.args(["/C", &sweeper.to_string_lossy()]);
        cleanup.current_dir(std::env::temp_dir());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cleanup.creation_flags(0x0800_0000 | 0x0000_0008);
        }
        let _ = cleanup.spawn();
        unsafe { SWEEPER_SPAWNED = true };
    }
}

unsafe fn set_uninstall_step_text(text: &str) {
    SetWindowTextW(HWND_STEP_LABEL, to_wide(&format!("{text}\0")).as_ptr());
}

/// the real removal work, run on the wizard's worker thread. reports via
/// the step label + progress bar; WM_DESTROY guarantees the sweeper runs
/// even if the user closes the window mid-uninstall.
unsafe fn run_uninstall_steps() {
    let keep_data = UNINSTALL_KEEP_DATA;
    let install_dir = unsafe { uninstall_dir_snapshot().unwrap_or_else(resolve_uninstall_dir) };
    let data_dir = app_data_dir();

    set_uninstall_step_text("Closing BlastWA...");
    SendMessageW(HWND_PROGRESS, PBM_SETPOS, 10, 0);
    kill_running_app();
    std::thread::sleep(std::time::Duration::from_millis(400));

    set_uninstall_step_text("Removing shortcuts...");
    SendMessageW(HWND_PROGRESS, PBM_SETPOS, 25, 0);
    remove_profile_shortcuts();

    set_uninstall_step_text("Cleaning registry...");
    SendMessageW(HWND_PROGRESS, PBM_SETPOS, 40, 0);
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let uninstall_path = format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{}", APP_NAME);
    let _ = hkcu.delete_subkey_all(&uninstall_path);

    set_uninstall_step_text("Removing account data...");
    SendMessageW(HWND_PROGRESS, PBM_SETPOS, 55, 0);
    let mut data_gone = true;
    if !keep_data {
        data_gone = remove_dir_with_retry(&data_dir, 10);
    }

    set_uninstall_step_text("Removing program files...");
    SendMessageW(HWND_PROGRESS, PBM_SETPOS, 75, 0);
    let self_exe = std::env::current_exe().ok();
    let inside_install_dir = self_exe.as_ref().and_then(|p| p.parent()) == Some(install_dir.as_path());
    if !inside_install_dir {
        remove_dir_with_retry(&install_dir, 5);
    } else {
        // delete everything in the install dir except the running
        // uninstaller; the sweeper takes the rest after this process exits
        if let Ok(entries) = std::fs::read_dir(&install_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if Some(&path) == self_exe.as_ref() {
                    continue;
                }
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                } else {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    set_uninstall_step_text("Finishing up...");
    SendMessageW(HWND_PROGRESS, PBM_SETPOS, 90, 0);
    std::thread::sleep(std::time::Duration::from_millis(300));
    SendMessageW(HWND_PROGRESS, PBM_SETPOS, 100, 0);

    // keep data can only be honored when it is actually still there
    UNINSTALL_KEEP_DATA = keep_data && data_dir.exists();
    let _ = data_gone;
    UNINSTALL_STEP = UninstallStep::Done;
    update_wizard_view();
}

#[inline]
fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

struct ChromeInstall {
    path: PathBuf,
    version: String,
}

fn find_chrome() -> Option<ChromeInstall> {
    let app_paths = |hive: RegKey| -> Option<ChromeInstall> {
        let key = hive
            .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe")
            .ok()?;
        let p: String = key.get_value("").ok()?;
        let pb = PathBuf::from(&p);
        pb.exists().then(|| ChromeInstall {
            path: pb,
            version: read_version(),
        })
    };

    if let Some(hit) = app_paths(RegKey::predef(HKEY_LOCAL_MACHINE)) {
        return Some(hit);
    }
    if let Some(local) = dirs_localappdata() {
        let candidate = local.join(r"Google\Chrome\Application\chrome.exe");
        if candidate.exists() {
            return Some(ChromeInstall {
                path: candidate,
                version: read_version(),
            });
        }
    }
    app_paths(RegKey::predef(HKEY_CURRENT_USER))
}

fn read_version() -> String {
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        if let Ok(key) = RegKey::predef(hive).open_subkey(r"SOFTWARE\Google\Chrome\BLBeacon") {
            if let Ok(v) = key.get_value::<String, _>("version") {
                return v;
            }
        }
    }
    String::new()
}

fn chrome_major(version: &str) -> u32 {
    version
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0)
}

fn dirs_localappdata() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA").ok().map(PathBuf::from)
}

fn default_install_dir() -> PathBuf {
    dirs_localappdata()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Programs")
        .join(APP_NAME)
}

fn app_data_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join(APP_NAME)
}

fn create_shortcut(target_exe: &Path, shortcut_path: &Path, desc: &str) -> Result<(), String> {
    let target_str = target_exe.to_str().ok_or_else(|| "Invalid target exe path".to_string())?;
    let lnk_str = shortcut_path.to_str().ok_or_else(|| "Invalid shortcut path".to_string())?;
    let workdir = target_exe.parent().and_then(|p| p.to_str()).unwrap_or("");

    let ps_script = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         $s = $ws.CreateShortcut('{}'); \
         $s.TargetPath = '{}'; \
         $s.WorkingDirectory = '{}'; \
         $s.Description = '{}'; \
         $s.IconLocation = '{},0'; \
         $s.Save()",
        lnk_str.replace('\'', "''"),
        target_str.replace('\'', "''"),
        workdir.replace('\'', "''"),
        desc.replace('\'', "''"),
        target_str.replace('\'', "''")
    );

    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &ps_script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("Failed to create shortcut: {e}"))?;
    if !out.status.success() {
        return Err("Shortcut creation failed".into());
    }
    Ok(())
}

fn pick_folder(initial: &Path) -> Option<PathBuf> {
    let init_str = initial.to_str().unwrap_or("");
    let ps_script = format!(
        "[System.Reflection.Assembly]::LoadWithPartialName('System.windows.forms') | Out-Null; \
         $f = New-Object System.Windows.Forms.FolderBrowserDialog; \
         $f.Description = 'Select BlastWA Installation Folder'; \
         $f.SelectedPath = '{}'; \
         $f.ShowNewFolderButton = $true; \
         if ($f.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{ Write-Output $f.SelectedPath }}",
        init_str.replace('\'', "''")
    );

    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &ps_script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    if out.status.success() {
        let chosen = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !chosen.is_empty() {
            return Some(PathBuf::from(chosen));
        }
    }
    None
}

fn get_chosen_install_dir() -> PathBuf {
    unsafe {
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(HWND_EDIT_PATH, buf.as_mut_ptr(), 512);
        if len > 0 {
            let s = String::from_utf16_lossy(&buf[..len as usize]);
            let p = PathBuf::from(s.trim());
            if !p.as_os_str().is_empty() {
                return p;
            }
        }
    }
    default_install_dir()
}

fn register_windows_uninstaller(install_dir: &Path, exe_path: &Path, uninstaller_path: &Path) {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let uninstall_key = match hkcu.create_subkey(format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{}", APP_NAME)) {
        Ok((k, _)) => k,
        Err(_) => return,
    };

    let _ = uninstall_key.set_value("DisplayName", &APP_NAME);
    let _ = uninstall_key.set_value("DisplayVersion", &APP_VERSION);
    let _ = uninstall_key.set_value("Publisher", &APP_PUBLISHER);
    let _ = uninstall_key.set_value("DisplayIcon", &format!("{},0", exe_path.display()));
    let _ = uninstall_key.set_value("UninstallString", &format!("\"{}\" --uninstall", uninstaller_path.display()));
    let _ = uninstall_key.set_value("InstallLocation", &install_dir.to_string_lossy().to_string());
    let _ = uninstall_key.set_value("EstimatedSize", &42000u32); // ~42 MB in KB
    let _ = uninstall_key.set_value("NoModify", &1u32);
    let _ = uninstall_key.set_value("NoRepair", &1u32);
}

fn perform_installation() -> Result<(), String> {
    let chrome = match find_chrome() {
        Some(c) => c,
        None => return Err("Google Chrome not found! Please install Chrome first.".into()),
    };

    let major = chrome_major(&chrome.version);
    if major < MIN_CHROME_MAJOR {
        eprintln!("Warning: Chrome version {} is below recommended", chrome.version);
    }

    let app_dir = app_data_dir();
    let _ = std::fs::create_dir_all(&app_dir);
    let target_install_dir = get_chosen_install_dir();
    let _ = std::fs::create_dir_all(&target_install_dir);

    let config_path = app_dir.join("config.json");
    let mut config: serde_json::Value = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    config["chrome_path"] = serde_json::Value::String(chrome.path.to_string_lossy().to_string());
    config["chrome_version"] = serde_json::Value::String(chrome.version);
    config["api_enabled"] = serde_json::Value::Bool(true);
    config["api_port"] = serde_json::json!(8765);

    let _ = std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap());

    let current_exe = std::env::current_exe().unwrap_or_default();
    let exe_dir = current_exe.parent().unwrap_or_else(|| Path::new("."));
    let candidates = [
        exe_dir.join("blastwa.exe"),
        exe_dir.join("../blastwa.exe"),
        exe_dir.join("../../target/release/blastwa.exe"),
        exe_dir.join("../../target/debug/blastwa.exe"),
        PathBuf::from("D:\\Tes\\blastwa\\target\\release\\blastwa.exe"),
        PathBuf::from("D:\\Tes\\blastwa\\target\\debug\\blastwa.exe"),
    ];

    let found_src = candidates.iter().find(|p| p.exists());
    let installed_exe = target_install_dir.join("blastwa.exe");

    if let Some(src_path) = found_src {
        let _ = std::fs::copy(src_path, &installed_exe);
    }

    // Write uninstall.exe into the install folder (copy of setup.exe invoked with --uninstall)
    let uninstaller_path = target_install_dir.join("uninstall.exe");
    if current_exe.exists() {
        let _ = std::fs::copy(&current_exe, &uninstaller_path);
    }

    // Register in Windows Control Panel / Installed Apps
    register_windows_uninstaller(&target_install_dir, &installed_exe, &uninstaller_path);

    unsafe {
        let target_binary = if installed_exe.exists() { &installed_exe } else if let Some(s) = found_src { s } else { &installed_exe };
        if OPT_DESKTOP {
            if let Some(desktop) = dirs::desktop_dir() {
                let lnk = desktop.join("BlastWA.lnk");
                let _ = create_shortcut(target_binary, &lnk, "BlastWA - WhatsApp Bulk Sender");
            }
        }
        if OPT_STARTMENU {
            if let Some(data_dir) = dirs_localappdata() {
                let start_menu = data_dir
                    .join("..")
                    .join("Roaming")
                    .join("Microsoft")
                    .join("Windows")
                    .join("Start Menu")
                    .join("Programs");
                let lnk = start_menu.join("BlastWA.lnk");
                let _ = create_shortcut(target_binary, &lnk, "BlastWA - WhatsApp Bulk Sender");
            }
        }
    }

    Ok(())
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// TextOutW with the UTF-16 length computed for the caller
unsafe fn text_out(hdc: HDC, x: i32, y: i32, s: &str) {
    TextOutW(hdc, x, y, to_wide(s).as_ptr(), s.encode_utf16().count() as i32);
}

unsafe fn update_wizard_view() {
    if UNINSTALL_MODE {
        update_uninstall_view();
        return;
    }
    let step = CURRENT_STEP;

    let show_location = step == WizardStep::ChooseLocation;
    let show_options = step == WizardStep::SelectOptions;
    let show_progress = step == WizardStep::Installing;

    ShowWindow(HWND_EDIT_PATH, if show_location { SW_SHOW } else { SW_HIDE });
    ShowWindow(HWND_BTN_BROWSE, if show_location { SW_SHOW } else { SW_HIDE });
    ShowWindow(HWND_CHK_DESKTOP, if show_options { SW_SHOW } else { SW_HIDE });
    ShowWindow(HWND_CHK_STARTMENU, if show_options { SW_SHOW } else { SW_HIDE });
    ShowWindow(HWND_PROGRESS, if show_progress { SW_SHOW } else { SW_HIDE });
    ShowWindow(HWND_CHK_LAUNCH, if step == WizardStep::Finish { SW_SHOW } else { SW_HIDE });

    match step {
        WizardStep::Welcome => {
            ShowWindow(HWND_BACK, SW_HIDE);
            SetWindowTextW(HWND_NEXT, to_wide("Next >\0").as_ptr());
            SetWindowTextW(HWND_CANCEL, to_wide("Cancel\0").as_ptr());
        }
        WizardStep::ChooseLocation => {
            ShowWindow(HWND_BACK, SW_SHOW);
            SetWindowTextW(HWND_NEXT, to_wide("Next >\0").as_ptr());
            SetWindowTextW(HWND_CANCEL, to_wide("Cancel\0").as_ptr());
        }
        WizardStep::SelectOptions => {
            ShowWindow(HWND_BACK, SW_SHOW);
            SetWindowTextW(HWND_NEXT, to_wide("Install\0").as_ptr());
            SetWindowTextW(HWND_CANCEL, to_wide("Cancel\0").as_ptr());
        }
        WizardStep::Installing => {
            ShowWindow(HWND_BACK, SW_HIDE);
            EnableWindow(HWND_NEXT, 0);
            EnableWindow(HWND_CANCEL, 0);
        }
        WizardStep::Finish => {
            ShowWindow(HWND_BACK, SW_HIDE);
            ShowWindow(HWND_CANCEL, SW_HIDE);
            EnableWindow(HWND_NEXT, 1);
            SetWindowTextW(HWND_NEXT, to_wide("Finish\0").as_ptr());
        }
    }

    InvalidateRect(HWND_MAIN, std::ptr::null(), 1);
}

unsafe fn update_uninstall_view() {
    let step = UNINSTALL_STEP;

    // install-only controls stay hidden for the whole uninstall flow
    ShowWindow(HWND_EDIT_PATH, SW_HIDE);
    ShowWindow(HWND_BTN_BROWSE, SW_HIDE);
    ShowWindow(HWND_CHK_DESKTOP, SW_HIDE);
    ShowWindow(HWND_CHK_STARTMENU, SW_HIDE);
    ShowWindow(HWND_CHK_LAUNCH, SW_HIDE);

    let show_options = step == UninstallStep::Options;
    let show_working = step == UninstallStep::Working;
    ShowWindow(HWND_CHK_DATA, if show_options { SW_SHOW } else { SW_HIDE });
    ShowWindow(HWND_STEP_LABEL, if show_working { SW_SHOW } else { SW_HIDE });
    ShowWindow(HWND_PROGRESS, if show_working { SW_SHOW } else { SW_HIDE });

    match step {
        UninstallStep::Welcome => {
            ShowWindow(HWND_BACK, SW_HIDE);
            ShowWindow(HWND_CANCEL, SW_SHOW);
            EnableWindow(HWND_CANCEL, 1);
            EnableWindow(HWND_NEXT, 1);
            SetWindowTextW(HWND_NEXT, to_wide("Next >\0").as_ptr());
            SetWindowTextW(HWND_CANCEL, to_wide("Cancel\0").as_ptr());
        }
        UninstallStep::Options => {
            ShowWindow(HWND_BACK, SW_SHOW);
            ShowWindow(HWND_CANCEL, SW_SHOW);
            EnableWindow(HWND_CANCEL, 1);
            EnableWindow(HWND_NEXT, 1);
            SetWindowTextW(HWND_NEXT, to_wide("Uninstall\0").as_ptr());
            SetWindowTextW(HWND_CANCEL, to_wide("Cancel\0").as_ptr());
        }
        UninstallStep::Working => {
            ShowWindow(HWND_BACK, SW_HIDE);
            ShowWindow(HWND_CANCEL, SW_HIDE);
            EnableWindow(HWND_NEXT, 0);
        }
        UninstallStep::Done => {
            ShowWindow(HWND_BACK, SW_HIDE);
            ShowWindow(HWND_CANCEL, SW_HIDE);
            EnableWindow(HWND_NEXT, 1);
            SetWindowTextW(HWND_NEXT, to_wide("Finish\0").as_ptr());
        }
    }

    InvalidateRect(HWND_MAIN, std::ptr::null(), 1);
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            HWND_MAIN = hwnd;

            FONT_TITLE = CreateFontW(
                20, 0, 0, 0, FW_BOLD as i32, 0, 0, 0,
                DEFAULT_CHARSET as u32, OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32, CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32 | FF_DONTCARE as u32,
                to_wide("Segoe UI\0").as_ptr(),
            );

            FONT_BODY = CreateFontW(
                15, 0, 0, 0, FW_NORMAL as i32, 0, 0, 0,
                DEFAULT_CHARSET as u32, OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32, CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32 | FF_DONTCARE as u32,
                to_wide("Segoe UI\0").as_ptr(),
            );

            let hinstance = GetModuleHandleW(std::ptr::null());

            // Create Icon from memory
            HICON_APP = CreateIconFromResourceEx(
                ICON_RAW_BYTES.as_ptr() as *mut u8,
                ICON_RAW_BYTES.len() as u32,
                1,
                0x00030000,
                64,
                64,
                LR_DEFAULTCOLOR,
            );

            if HICON_APP != 0 {
                SendMessageW(hwnd, WM_SETICON, ICON_BIG as WPARAM, HICON_APP as LPARAM);
                SendMessageW(hwnd, WM_SETICON, ICON_SMALL as WPARAM, HICON_APP as LPARAM);
            }

            HWND_BACK = CreateWindowExW(
                0, to_wide("BUTTON\0").as_ptr(), to_wide("< Back\0").as_ptr(),
                WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON as u32,
                290, 315, 80, 26, hwnd, IDC_BTN_BACK as HMENU, hinstance, std::ptr::null(),
            );

            HWND_NEXT = CreateWindowExW(
                0, to_wide("BUTTON\0").as_ptr(), to_wide("Next >\0").as_ptr(),
                WS_CHILD | WS_VISIBLE | BS_DEFPUSHBUTTON as u32,
                380, 315, 85, 26, hwnd, IDC_BTN_NEXT as HMENU, hinstance, std::ptr::null(),
            );

            HWND_CANCEL = CreateWindowExW(
                0, to_wide("BUTTON\0").as_ptr(), to_wide("Cancel\0").as_ptr(),
                WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON as u32,
                480, 315, 80, 26, hwnd, IDC_BTN_CANCEL as HMENU, hinstance, std::ptr::null(),
            );

            HWND_EDIT_PATH = CreateWindowExW(
                WS_EX_CLIENTEDGE, to_wide("EDIT\0").as_ptr(),
                to_wide(&format!("{}\0", default_install_dir().display())).as_ptr(),
                WS_CHILD | ES_AUTOHSCROLL as u32,
                180, 160, 280, 24, hwnd, IDC_EDIT_PATH as HMENU, hinstance, std::ptr::null(),
            );

            HWND_BTN_BROWSE = CreateWindowExW(
                0, to_wide("BUTTON\0").as_ptr(), to_wide("📁 Browse...\0").as_ptr(),
                WS_CHILD | BS_PUSHBUTTON as u32,
                470, 159, 85, 26, hwnd, IDC_BTN_BROWSE as HMENU, hinstance, std::ptr::null(),
            );

            HWND_CHK_DESKTOP = CreateWindowExW(
                0, to_wide("BUTTON\0").as_ptr(), to_wide("Create a Desktop shortcut\0").as_ptr(),
                WS_CHILD | BS_AUTOCHECKBOX as u32,
                180, 130, 350, 24, hwnd, IDC_CHK_DESKTOP as HMENU, hinstance, std::ptr::null(),
            );
            SendMessageW(HWND_CHK_DESKTOP, BM_SETCHECK, BST_CHECKED as WPARAM, 0);

            HWND_CHK_STARTMENU = CreateWindowExW(
                0, to_wide("BUTTON\0").as_ptr(), to_wide("Create a Start Menu folder shortcut\0").as_ptr(),
                WS_CHILD | BS_AUTOCHECKBOX as u32,
                180, 160, 350, 24, hwnd, IDC_CHK_STARTMENU as HMENU, hinstance, std::ptr::null(),
            );
            SendMessageW(HWND_CHK_STARTMENU, BM_SETCHECK, BST_CHECKED as WPARAM, 0);

            HWND_PROGRESS = CreateWindowExW(
                0, to_wide("msctls_progress32\0").as_ptr(), std::ptr::null(),
                WS_CHILD | PBS_SMOOTH,
                180, 160, 375, 20, hwnd, IDC_PROGRESS as HMENU, hinstance, std::ptr::null(),
            );

            HWND_CHK_LAUNCH = CreateWindowExW(
                0, to_wide("BUTTON\0").as_ptr(), to_wide("Launch BlastWA now\0").as_ptr(),
                WS_CHILD | BS_AUTOCHECKBOX as u32,
                180, 200, 350, 24, hwnd, IDC_CHK_LAUNCH as HMENU, hinstance, std::ptr::null(),
            );
            SendMessageW(HWND_CHK_LAUNCH, BM_SETCHECK, BST_CHECKED as WPARAM, 0);

            // uninstall-only controls (options page + working page step text)
            HWND_CHK_DATA = CreateWindowExW(
                0, to_wide("BUTTON\0").as_ptr(), to_wide("Also delete account data (WhatsApp sessions, profiles)\0").as_ptr(),
                WS_CHILD | BS_AUTOCHECKBOX as u32,
                180, 155, 380, 24, hwnd, IDC_CHK_DATA as HMENU, hinstance, std::ptr::null(),
            );
            // unchecked by default: keeping sessions means a reinstall does
            // not require re-scanning every account's QR code

            HWND_STEP_LABEL = CreateWindowExW(
                0, to_wide("STATIC\0").as_ptr(), to_wide("Preparing...\0").as_ptr(),
                WS_CHILD,
                180, 195, 380, 22, hwnd, IDC_STEP_LABEL as HMENU, hinstance, std::ptr::null(),
            );

            for ctrl in [HWND_BACK, HWND_NEXT, HWND_CANCEL, HWND_EDIT_PATH, HWND_BTN_BROWSE, HWND_CHK_DESKTOP, HWND_CHK_STARTMENU, HWND_CHK_LAUNCH, HWND_CHK_DATA, HWND_STEP_LABEL] {
                SendMessageW(ctrl, WM_SETFONT, FONT_BODY as WPARAM, 1);
            }

            update_wizard_view();
            0
        }

        WM_COMMAND => {
            let id = loword(wparam as u32) as isize;
            if UNINSTALL_MODE {
                match id {
                    IDC_BTN_NEXT => {
                        match UNINSTALL_STEP {
                            UninstallStep::Welcome => {
                                UNINSTALL_STEP = UninstallStep::Options;
                                update_wizard_view();
                            }
                            UninstallStep::Options => {
                                // checkbox checked = delete data; unchecked = keep
                                UNINSTALL_KEEP_DATA =
                                    SendMessageW(HWND_CHK_DATA, BM_GETCHECK, 0, 0) != BST_CHECKED as isize;
                                UNINSTALL_DIR = Some(resolve_uninstall_dir());
                                UNINSTALL_STEP = UninstallStep::Working;
                                update_wizard_view();
                                std::thread::spawn(|| unsafe { run_uninstall_steps() });
                            }
                            UninstallStep::Working => {}
                            UninstallStep::Done => {
                                spawn_uninstall_sweeper(!UNINSTALL_KEEP_DATA);
                                PostQuitMessage(0);
                            }
                        }
                    }
                    IDC_BTN_BACK => {
                        if UNINSTALL_STEP == UninstallStep::Options {
                            UNINSTALL_STEP = UninstallStep::Welcome;
                            update_wizard_view();
                        }
                    }
                    IDC_BTN_CANCEL => PostQuitMessage(0),
                    _ => {}
                }
                return 0;
            }
            match id {
                IDC_BTN_BROWSE => {
                    let current = get_chosen_install_dir();
                    if let Some(folder) = pick_folder(&current) {
                        let final_path = if folder.file_name().and_then(|n| n.to_str()) == Some(APP_NAME) {
                            folder
                        } else {
                            folder.join(APP_NAME)
                        };
                        SetWindowTextW(HWND_EDIT_PATH, to_wide(&format!("{}\0", final_path.display())).as_ptr());
                    }
                }
                IDC_BTN_NEXT => {
                    match CURRENT_STEP {
                        WizardStep::Welcome => {
                            CURRENT_STEP = WizardStep::ChooseLocation;
                            update_wizard_view();
                        }
                        WizardStep::ChooseLocation => {
                            CURRENT_STEP = WizardStep::SelectOptions;
                            update_wizard_view();
                        }
                        WizardStep::SelectOptions => {
                            OPT_DESKTOP = SendMessageW(HWND_CHK_DESKTOP, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
                            OPT_STARTMENU = SendMessageW(HWND_CHK_STARTMENU, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;

                            CURRENT_STEP = WizardStep::Installing;
                            update_wizard_view();

                            std::thread::spawn(|| {
                                for p in (0..=100).step_by(10) {
                                    SendMessageW(HWND_PROGRESS, PBM_SETPOS, p, 0);
                                    std::thread::sleep(std::time::Duration::from_millis(60));
                                }
                                let _ = perform_installation();
                                CURRENT_STEP = WizardStep::Finish;
                                update_wizard_view();
                            });
                        }
                        WizardStep::Installing => {}
                        WizardStep::Finish => {
                            OPT_LAUNCH = SendMessageW(HWND_CHK_LAUNCH, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
                            if OPT_LAUNCH {
                                let installed = get_chosen_install_dir().join("blastwa.exe");
                                if installed.exists() {
                                    let _ = Command::new(installed).spawn();
                                }
                            }
                            PostQuitMessage(0);
                        }
                    }
                }
                IDC_BTN_BACK => {
                    match CURRENT_STEP {
                        WizardStep::ChooseLocation => {
                            CURRENT_STEP = WizardStep::Welcome;
                            update_wizard_view();
                        }
                        WizardStep::SelectOptions => {
                            CURRENT_STEP = WizardStep::ChooseLocation;
                            update_wizard_view();
                        }
                        _ => {}
                    }
                }
                IDC_BTN_CANCEL => {
                    PostQuitMessage(0);
                }
                _ => {}
            }
            0
        }

        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);

            let side_rect = RECT { left: 0, top: 0, right: 160, bottom: 300 };
            let brush_teal = CreateSolidBrush(rgb(18, 140, 126));
            FillRect(hdc, &side_rect, brush_teal);
            DeleteObject(brush_teal);

            let content_rect = RECT { left: 160, top: 0, right: 580, bottom: 300 };
            let brush_white = CreateSolidBrush(rgb(255, 255, 255));
            FillRect(hdc, &content_rect, brush_white);
            DeleteObject(brush_white);

            let bot_rect = RECT { left: 0, top: 300, right: 580, bottom: 360 };
            let brush_gray = CreateSolidBrush(rgb(240, 240, 240));
            FillRect(hdc, &bot_rect, brush_gray);
            DeleteObject(brush_gray);

            let line_rect = RECT { left: 0, top: 300, right: 580, bottom: 301 };
            let brush_line = CreateSolidBrush(rgb(215, 215, 215));
            FillRect(hdc, &line_rect, brush_line);
            DeleteObject(brush_line);

            if HICON_APP != 0 {
                DrawIconEx(hdc, 48, 35, HICON_APP, 64, 64, 0, 0, DI_NORMAL);
            }

            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, rgb(255, 255, 255));
            SelectObject(hdc, FONT_TITLE);
            TextOutW(hdc, 40, 115, to_wide("BlastWA\0").as_ptr(), 7);
            SelectObject(hdc, FONT_BODY);
            if UNINSTALL_MODE {
                TextOutW(hdc, 34, 145, to_wide("Uninstall\0").as_ptr(), 9);
            } else {
                TextOutW(hdc, 34, 145, to_wide("Setup Wizard\0").as_ptr(), 12);
            }
            TextOutW(hdc, 55, 170, to_wide("v0.2.0\0").as_ptr(), 6);

            SetTextColor(hdc, rgb(30, 30, 30));
            if UNINSTALL_MODE {
                match UNINSTALL_STEP {
                    UninstallStep::Welcome => {
                        SelectObject(hdc, FONT_TITLE);
                        text_out(hdc, 180, 25, "Uninstall BlastWA");
                        SelectObject(hdc, FONT_BODY);
                        text_out(hdc, 180, 70, "BlastWA will be removed from your computer.");
                        text_out(hdc, 180, 95, "Program files, shortcuts and registry entries will be deleted.");
                        text_out(hdc, 180, 145, "Click Next to continue.");
                    }
                    UninstallStep::Options => {
                        SelectObject(hdc, FONT_TITLE);
                        text_out(hdc, 180, 25, "Remove Account Data?");
                        SelectObject(hdc, FONT_BODY);
                        text_out(hdc, 180, 70, "Account data holds your WhatsApp sessions and profiles.");
                        text_out(hdc, 180, 95, "Leave the box unchecked to keep it for a future install.");
                        text_out(hdc, 180, 120, "Check the box to delete it permanently.");
                        text_out(hdc, 180, 190, "Data location:");
                        text_out(hdc, 180, 210, &app_data_dir().to_string_lossy());
                    }
                    UninstallStep::Working => {
                        SelectObject(hdc, FONT_TITLE);
                        text_out(hdc, 180, 25, "Uninstalling BlastWA...");
                        SelectObject(hdc, FONT_BODY);
                        text_out(hdc, 180, 70, "Please wait while BlastWA is being removed.");
                    }
                    UninstallStep::Done => {
                        SelectObject(hdc, FONT_TITLE);
                        text_out(hdc, 180, 25, "Uninstall Complete");
                        SelectObject(hdc, FONT_BODY);
                        text_out(hdc, 180, 70, "BlastWA has been removed from your computer.");
                        if UNINSTALL_KEEP_DATA {
                            text_out(hdc, 180, 95, "Your account data was kept at:");
                            text_out(hdc, 180, 115, &app_data_dir().to_string_lossy());
                        }
                        text_out(hdc, 180, 150, "Click Finish to exit.");
                    }
                }
                EndPaint(hwnd, &ps);
                return 0;
            }
            match CURRENT_STEP {
                WizardStep::Welcome => {
                    SelectObject(hdc, FONT_TITLE);
                    TextOutW(hdc, 180, 25, to_wide("Welcome to the BlastWA Setup\0").as_ptr(), 28);
                    SelectObject(hdc, FONT_BODY);
                    TextOutW(hdc, 180, 70, to_wide("This wizard will install BlastWA on your computer.\0").as_ptr(), 49);
                    TextOutW(hdc, 180, 100, to_wide("Chrome automation & campaign tools will be configured.\0").as_ptr(), 54);
                    TextOutW(hdc, 180, 145, to_wide("Click Next to continue with the setup.\0").as_ptr(), 37);
                }
                WizardStep::ChooseLocation => {
                    SelectObject(hdc, FONT_TITLE);
                    TextOutW(hdc, 180, 25, to_wide("Choose Install Location\0").as_ptr(), 23);
                    SelectObject(hdc, FONT_BODY);
                    TextOutW(hdc, 180, 70, to_wide("BlastWA will be installed in the destination folder.\0").as_ptr(), 51);
                    TextOutW(hdc, 180, 95, to_wide("To choose a different location, click Browse.\0").as_ptr(), 44);
                    TextOutW(hdc, 180, 135, to_wide("Destination Folder:\0").as_ptr(), 19);
                }
                WizardStep::SelectOptions => {
                    SelectObject(hdc, FONT_TITLE);
                    TextOutW(hdc, 180, 25, to_wide("Select Additional Tasks\0").as_ptr(), 23);
                    SelectObject(hdc, FONT_BODY);
                    TextOutW(hdc, 180, 70, to_wide("Choose the shortcuts you want created:\0").as_ptr(), 37);
                }
                WizardStep::Installing => {
                    SelectObject(hdc, FONT_TITLE);
                    TextOutW(hdc, 180, 25, to_wide("Installing BlastWA...\0").as_ptr(), 21);
                    SelectObject(hdc, FONT_BODY);
                    TextOutW(hdc, 180, 70, to_wide("Please wait while BlastWA is being installed.\0").as_ptr(), 45);
                    TextOutW(hdc, 180, 130, to_wide("Setting up files and Chrome automation...\0").as_ptr(), 41);
                }
                WizardStep::Finish => {
                    SelectObject(hdc, FONT_TITLE);
                    TextOutW(hdc, 180, 25, to_wide("Completing BlastWA Setup\0").as_ptr(), 24);
                    SelectObject(hdc, FONT_BODY);
                    TextOutW(hdc, 180, 70, to_wide("BlastWA has been installed on your computer.\0").as_ptr(), 43);
                    TextOutW(hdc, 180, 95, to_wide("All components and shortcuts are ready to use.\0").as_ptr(), 45);
                    TextOutW(hdc, 180, 140, to_wide("Click Finish to exit setup.\0").as_ptr(), 26);
                }
            }

            EndPaint(hwnd, &ps);
            0
        }

        WM_DESTROY => {
            // if the user closes the window mid-uninstall, the removal steps
            // already ran — make sure the self-cleanup sweeper still spawns
            if UNINSTALL_MODE
                && !matches!(UNINSTALL_STEP, UninstallStep::Welcome | UninstallStep::Options)
            {
                spawn_uninstall_sweeper(!UNINSTALL_KEEP_DATA);
            }
            if FONT_TITLE != 0 { DeleteObject(FONT_TITLE); }
            if FONT_BODY != 0 { DeleteObject(FONT_BODY); }
            if HICON_APP != 0 { DestroyIcon(HICON_APP); }
            PostQuitMessage(0);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn loword(l: u32) -> u16 {
    (l & 0xffff) as u16
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let exe_is_uninstaller = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase() == "uninstall"))
        .unwrap_or(false);

    if exe_is_uninstaller || args.iter().any(|a| a == "--uninstall") {
        unsafe { UNINSTALL_MODE = true; }
    }

    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS,
        };
        InitCommonControlsEx(&icc);

        let class_name = to_wide("BlastWASetupWizardClass\0");
        let hinstance = GetModuleHandleW(std::ptr::null());

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: 0,
            hCursor: LoadCursorW(0, IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: 0,
        };

        RegisterClassExW(&wc);

        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let win_w = 590;
        let win_h = 390;
        let pos_x = (screen_w - win_w) / 2;
        let pos_y = (screen_h - win_h) / 2;

        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class_name.as_ptr(),
            to_wide(if UNINSTALL_MODE { "BlastWA Uninstall" } else { APP_TITLE }).as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            pos_x, pos_y, win_w, win_h,
            0, 0, hinstance, std::ptr::null(),
        );

        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
