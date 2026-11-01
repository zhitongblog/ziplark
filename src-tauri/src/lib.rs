//! Ziplark desktop backend. Thin Tauri commands over `ziplark-core` — all the real
//! work lives in the shared engine, so the GUI behaves identically to the CLI.
//!
//! Two things the commands here have to get right, because a window is not a
//! terminal:
//!
//! * **Nothing long-running may sit on the main thread.** A plain (non-async)
//!   Tauri command runs on the main thread, so extracting a large archive froze
//!   the whole window — the spinner stopped animating and macOS offered to force
//!   quit. Every command that touches the engine hands the work to a blocking
//!   task and awaits it.
//! * **The user must be able to watch it and stop it.** The engine reports
//!   progress and takes a "stop" answer from that same callback; we forward the
//!   reports to the webview as events and answer "stop" when the window asks.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use ziplark_core::shell::{self, ShellStatus};
use ziplark_core::{
    add as core_add, create as core_create, detect, extract as core_extract, list as core_list,
    read_entry as core_read_entry, smart_dest, test as core_test, ArchiveInfo, CreateOptions,
    CreateReport, Error, ExtractOptions, ExtractReport, Format, Level, ListOptions, MatchMode,
    Progress, ReadOptions, TestReport,
};

/// Event names the webview listens for.
const EVENT_PROGRESS: &str = "ziplark://progress";
const EVENT_OPEN_FILE: &str = "ziplark-open-file";

/// The engine can report thousands of times a second on a tree of small files;
/// the webview only needs enough to look alive.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// The window runs one archive operation at a time; this is how it stops it.
#[derive(Default)]
struct Running {
    cancel: AtomicBool,
}

/// An archive the OS asked us to open — by "Open with", by dropping it on the
/// dock icon, or as an argument — before the webview was ready to hear about it.
#[derive(Default)]
struct PendingFile(Mutex<Option<String>>);

/// Payload returned to the GUI Quick Preview modal.
#[derive(serde::Serialize)]
struct PreviewPayload {
    path: String,
    size: u64,
    returned_bytes: usize,
    truncated: bool,
    kind: &'static str, // "text" | "image" | "binary"
    text: Option<String>,
    data_url: Option<String>,
    hex_preview: Option<String>,
}

fn parse_format(s: &str) -> Result<Format, String> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "zip" => Format::Zip,
        "7z" | "sevenz" => Format::SevenZ,
        "tar" => Format::Tar,
        "tar.gz" | "tgz" => Format::TarGz,
        "tar.bz2" => Format::TarBz2,
        "tar.xz" => Format::TarXz,
        "tar.zst" => Format::TarZst,
        "tar.lz4" | "tlz4" => Format::TarLz4,
        "gz" => Format::Gz,
        "bz2" => Format::Bz2,
        "xz" => Format::Xz,
        "zst" => Format::Zst,
        "lz4" => Format::Lz4,
        other => return Err(format!("unknown format '{other}'")),
    })
}

fn parse_level(s: &str) -> Level {
    match s {
        "store" => Level::Store,
        "fast" => Level::Fast,
        "best" => Level::Best,
        _ => Level::Default,
    }
}

/// Run `job` off the main thread, feeding it a progress callback that forwards
/// to the webview and reports the window's cancel request back to the engine.
async fn in_background<T, F>(app: AppHandle, running: Arc<Running>, job: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut dyn FnMut(Progress) -> bool) -> ziplark_core::Result<T> + Send + 'static,
{
    running.cancel.store(false, Ordering::SeqCst);
    let flag = running.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut last_sent: Option<Instant> = None;
        let mut on_progress = |p: Progress| {
            if flag.cancel.load(Ordering::Relaxed) {
                return false;
            }
            let due = last_sent.is_none_or(|t| t.elapsed() >= PROGRESS_INTERVAL);
            if due {
                last_sent = Some(Instant::now());
                if let Err(e) = app.emit(EVENT_PROGRESS, &p) {
                    eprintln!("ziplark: could not deliver progress to the window: {e}");
                }
            }
            true
        };
        job(&mut on_progress)
    })
    .await
    .map_err(|e| format!("background task failed: {e}"))?;

    result.map_err(|e| match e {
        // The window already knows it asked; don't show it as a failure.
        Error::Cancelled => "cancelled".to_string(),
        other => other.to_string(),
    })
}

#[tauri::command]
fn detect_format(path: String) -> Option<String> {
    detect(std::path::Path::new(&path)).map(|f| f.label().to_string())
}

/// Ask the running operation to stop. The engine finishes the chunk it is on
/// and returns, so this takes effect within a fraction of a second even inside
/// a multi-gigabyte file.
#[tauri::command]
fn cancel_operation(running: State<'_, Arc<Running>>) {
    running.cancel.store(true, Ordering::SeqCst);
}

/// A scripted action for driving the window in a self-test, from
/// `ZIPLARK_SELFTEST`. Debug builds only — a shipped app must never take
/// instructions from the environment.
#[tauri::command]
fn selftest_action() -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var("ZIPLARK_SELFTEST").ok()
    } else {
        None
    }
}

/// Let a self-test report what the window actually saw, so a check does not
/// depend on catching the right pixels at the right moment. Debug builds only.
#[tauri::command]
fn selftest_log(line: String) {
    selftest_note(&line);
}

fn selftest_note(line: &str) {
    if !cfg!(debug_assertions) {
        return;
    }
    eprintln!("ziplark-selftest: {line}");
    use std::io::Write;
    let path = std::env::temp_dir().join("ziplark-selftest.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

/// Hand over the archive the OS asked us to open, if there is one waiting.
#[tauri::command]
fn take_pending_file(pending: State<'_, PendingFile>) -> Option<String> {
    pending.0.lock().ok().and_then(|mut p| p.take())
}

#[tauri::command]
async fn list_archive(
    app: AppHandle,
    running: State<'_, Arc<Running>>,
    path: String,
    password: Option<String>,
) -> Result<ArchiveInfo, String> {
    let running = running.inner().clone();
    in_background(app, running, move |_| {
        core_list(&path, &ListOptions { password })
    })
    .await
}

#[tauri::command]
async fn test_archive(
    app: AppHandle,
    running: State<'_, Arc<Running>>,
    path: String,
    password: Option<String>,
) -> Result<TestReport, String> {
    let running = running.inner().clone();
    in_background(app, running, move |progress| {
        core_test(&path, &ListOptions { password }, Some(progress))
    })
    .await
}

#[tauri::command]
async fn preview_archive_entry(
    app: AppHandle,
    running: State<'_, Arc<Running>>,
    path: String,
    entry: String,
    password: Option<String>,
) -> Result<PreviewPayload, String> {
    let running = running.inner().clone();
    in_background(app, running, move |_| {
        let mime = image_mime(&entry);
        let max_bytes = if mime.is_some() {
            4 * 1024 * 1024
        } else {
            256 * 1024
        };
        let content = core_read_entry(
            &path,
            &entry,
            &ReadOptions {
                password,
                offset: 0,
                max_bytes,
            },
        )?;
        let returned_bytes = content.bytes.len();
        if let Some(m) = mime {
            if !content.truncated {
                let b64 = base64_encode(&content.bytes);
                return Ok(PreviewPayload {
                    path: content.path,
                    size: content.size,
                    returned_bytes,
                    truncated: false,
                    kind: "image",
                    text: None,
                    data_url: Some(format!("data:{m};base64,{b64}")),
                    hex_preview: None,
                });
            }
        }
        let is_binary = content.bytes.contains(&0) || std::str::from_utf8(&content.bytes).is_err();
        if !is_binary {
            Ok(PreviewPayload {
                path: content.path,
                size: content.size,
                returned_bytes,
                truncated: content.truncated,
                kind: "text",
                text: Some(String::from_utf8_lossy(&content.bytes).into_owned()),
                data_url: None,
                hex_preview: None,
            })
        } else {
            Ok(PreviewPayload {
                path: content.path,
                size: content.size,
                returned_bytes,
                truncated: content.truncated,
                kind: "binary",
                text: None,
                data_url: None,
                hex_preview: Some(format_hex_dump(&content.bytes, 2048)),
            })
        }
    })
    .await
}

fn image_mime(entry_path: &str) -> Option<&'static str> {
    let l = entry_path.to_ascii_lowercase();
    if l.ends_with(".png") {
        Some("image/png")
    } else if l.ends_with(".jpg") || l.ends_with(".jpeg") {
        Some("image/jpeg")
    } else if l.ends_with(".gif") {
        Some("image/gif")
    } else if l.ends_with(".webp") {
        Some("image/webp")
    } else if l.ends_with(".svg") {
        Some("image/svg+xml")
    } else if l.ends_with(".bmp") {
        Some("image/bmp")
    } else if l.ends_with(".ico") {
        Some("image/x-icon")
    } else {
        None
    }
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn format_hex_dump(bytes: &[u8], limit: usize) -> String {
    let slice = &bytes[..bytes.len().min(limit)];
    let mut lines = Vec::new();
    for (row, chunk) in slice.chunks(16).enumerate() {
        let mut hex = String::with_capacity(48);
        let mut ascii = String::with_capacity(16);
        for (i, &b) in chunk.iter().enumerate() {
            if i == 8 {
                hex.push(' ');
            }
            hex.push_str(&format!("{b:02x} "));
            ascii.push(if (0x20..=0x7e).contains(&b) {
                b as char
            } else {
                '.'
            });
        }
        lines.push(format!("{:08x}  {:<49} |{}|", row * 16, hex, ascii));
    }
    lines.join("\n")
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn extract_archive(
    app: AppHandle,
    running: State<'_, Arc<Running>>,
    path: String,
    dest: String,
    password: Option<String>,
    overwrite: bool,
    include: Option<Vec<String>>,
    exact: Option<bool>,
    keep_broken: Option<bool>,
    smart: Option<bool>,
) -> Result<ExtractReport, String> {
    let running = running.inner().clone();
    in_background(app, running, move |progress| {
        let opts = ExtractOptions {
            password,
            dest: PathBuf::from(dest),
            overwrite,
            include: include.unwrap_or_default(),
            match_mode: if exact.unwrap_or(false) {
                MatchMode::Exact
            } else {
                MatchMode::Auto
            },
            keep_broken: keep_broken.unwrap_or(false),
            smart: smart.unwrap_or(false),
        };
        core_extract(&path, &opts, Some(progress))
    })
    .await
}

/// Smart-extract an archive right next to itself in its parent directory.
#[tauri::command]
async fn extract_archive_here(
    app: AppHandle,
    running: State<'_, Arc<Running>>,
    path: String,
    password: Option<String>,
    keep_broken: Option<bool>,
) -> Result<ExtractReport, String> {
    let running = running.inner().clone();
    in_background(app, running, move |progress| {
        let arc_path = Path::new(&path);
        let parent = arc_path.parent().unwrap_or(Path::new("."));
        let info = core_list(
            arc_path,
            &ListOptions {
                password: password.clone(),
            },
        )?;
        let dest = smart_dest(&info.path, parent, &info.entries, true);
        let opts = ExtractOptions {
            password,
            dest,
            overwrite: true,
            include: Vec::new(),
            match_mode: MatchMode::Auto,
            keep_broken: keep_broken.unwrap_or(false),
            smart: false,
        };
        core_extract(arc_path, &opts, Some(progress))
    })
    .await
}

#[tauri::command]
async fn create_archive(
    app: AppHandle,
    running: State<'_, Arc<Running>>,
    output: String,
    inputs: Vec<String>,
    format: String,
    level: String,
    password: Option<String>,
) -> Result<CreateReport, String> {
    if inputs.is_empty() {
        return Err("select at least one file or folder".into());
    }
    let opts = CreateOptions {
        format: parse_format(&format)?,
        level: parse_level(&level),
        password: password.filter(|p| !p.is_empty()),
    };
    let running = running.inner().clone();
    in_background(app, running, move |progress| {
        let inputs: Vec<PathBuf> = inputs.into_iter().map(PathBuf::from).collect();
        core_create(&output, &inputs, &opts, Some(progress))
    })
    .await
}

#[tauri::command]
async fn add_to_archive(
    app: AppHandle,
    running: State<'_, Arc<Running>>,
    archive: String,
    inputs: Vec<String>,
    password: Option<String>,
) -> Result<CreateReport, String> {
    if inputs.is_empty() {
        return Err("select at least one file or folder".into());
    }
    let fmt = detect(Path::new(&archive))
        .ok_or_else(|| "cannot modify this archive format".to_string())?;
    if !fmt.can_modify() {
        return Err(format!("{} archives are read-only", fmt.label()));
    }
    let opts = CreateOptions {
        format: fmt,
        level: Level::Default,
        password: password.filter(|p| !p.is_empty()),
    };
    let running = running.inner().clone();
    in_background(app, running, move |progress| {
        let inputs: Vec<PathBuf> = inputs.into_iter().map(PathBuf::from).collect();
        core_add(&archive, &inputs, &opts, Some(progress))
    })
    .await
}

#[tauri::command]
fn shell_integration_status() -> Result<ShellStatus, String> {
    shell::status().map_err(|e| e.to_string())
}

#[tauri::command]
fn shell_integration_install() -> Result<ShellStatus, String> {
    shell::install().map_err(|e| e.to_string())?;
    shell::status().map_err(|e| e.to_string())
}

#[tauri::command]
fn shell_integration_uninstall() -> Result<ShellStatus, String> {
    shell::uninstall().map_err(|e| e.to_string())?;
    shell::status().map_err(|e| e.to_string())
}

/// Ziplark's own version, for the app's About/footer.
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Open an http(s) URL in the user's default browser (not the app webview).
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http(s) URLs are allowed".into());
    }
    let spawn = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&url).spawn()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&url).spawn()
    };
    spawn.map(|_| ()).map_err(|e| e.to_string())
}

/// Show a finished file in the OS file manager.
#[tauri::command]
fn reveal_in_file_manager(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err("that path no longer exists".into());
    }
    let spawn = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-R").arg(&p).spawn()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer")
            .arg(format!("/select,{path}"))
            .spawn()
    } else {
        let dir = p.parent().unwrap_or(&p);
        std::process::Command::new("xdg-open").arg(dir).spawn()
    };
    spawn.map(|_| ()).map_err(|e| e.to_string())
}

/// The first command-line argument that looks like a file we were asked to
/// open. This is how "Open with" arrives on Windows and Linux.
fn archive_from_args() -> Option<String> {
    std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-') && std::path::Path::new(a).is_file())
}

/// Handle headless OS right-click context menu invocations (`extract-here` and
/// `compress-zip`) when the context menu was installed from the desktop app.
fn handle_headless_menu_command() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("extract-here") => {
            if let Err(e) = shell::extract_here_batch(&args[1..], None, false) {
                eprintln!("ziplark: extract-here failed: {e}");
                std::process::exit(1);
            }
            true
        }
        Some("compress-zip") => {
            if let Err(e) = shell::compress_zip_batch(&args[1..], Level::Default, None) {
                eprintln!("ziplark: compress-zip failed: {e}");
                std::process::exit(1);
            }
            true
        }
        _ => false,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if handle_headless_menu_command() {
        return;
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(Running::default()))
        .manage(PendingFile::default())
        .on_page_load(|webview, payload| {
            selftest_note(&format!(
                "page-load {:?} url={}",
                payload.event(),
                payload.url()
            ));
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = webview.eval(
                    "setTimeout(() => { const d = document, cs = (e) => e ? getComputedStyle(e) : null; \
                       const pick = (sel) => { const e = d.querySelector(sel); if (!e) return sel + '=MISSING'; const s = cs(e), r = e.getBoundingClientRect(); \
                         return sel + '{disp=' + s.display + ' vis=' + s.visibility + ' op=' + s.opacity + ' color=' + s.color + ' bg=' + s.backgroundColor + ' rect=' + Math.round(r.x) + ',' + Math.round(r.y) + ',' + Math.round(r.width) + 'x' + Math.round(r.height) + ' cls=' + e.className + '}'; }; \
                       const top = d.elementFromPoint(innerWidth/2, innerHeight/2); \
                       window.__TAURI__.core.invoke('selftest_log', {line: 'dom: ver=' + (d.getElementById('app-ver')||{}).textContent + ' lang=' + d.documentElement.lang + ' win=' + innerWidth + 'x' + innerHeight + \
                         ' sheets=' + d.styleSheets.length + ' center=' + (top && (top.id || top.className || top.tagName)) + ' | ' + \
                         ['html','body','.app','.topbar','main','#view-open','#busy','.busy','.modal:not(.hidden)'].map(pick).join(' | ')}); }, 1500)",
                );
            }
        })
        .setup(|app| {
            if let Some(path) = archive_from_args() {
                if let Ok(mut pending) = app.state::<PendingFile>().0.lock() {
                    *pending = Some(path);
                }
            }
            // A dev build launched from a terminal starts behind it and is easy
            // to lose; bring the window forward so it can actually be used.
            if cfg!(debug_assertions) {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.center();
                    let _ = win.show();
                    let _ = win.unminimize();
                    let _ = win.set_always_on_top(true);
                    let _ = win.set_focus();
                    let w = win.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        let _ = w.set_always_on_top(false);
                    });
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            detect_format,
            list_archive,
            test_archive,
            preview_archive_entry,
            extract_archive,
            extract_archive_here,
            create_archive,
            add_to_archive,
            shell_integration_status,
            shell_integration_install,
            shell_integration_uninstall,
            cancel_operation,
            take_pending_file,
            selftest_action,
            selftest_log,
            app_version,
            open_url,
            reveal_in_file_manager
        ])
        .build(tauri::generate_context!())
        .expect("error while building Ziplark");

    app.run(|app_handle, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Opened { urls } = event {
            for url in urls {
                if let Ok(path) = url.to_file_path() {
                    let path = path.to_string_lossy().to_string();
                    selftest_note(&format!("opened by the OS: {path}"));
                    if let Ok(mut pending) = app_handle.state::<PendingFile>().0.lock() {
                        *pending = Some(path.clone());
                    }
                    let _ = app_handle.emit(EVENT_OPEN_FILE, path);
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (app_handle, &event);
        }
    });
}
