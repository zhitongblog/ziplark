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

use ziplark_core::{
    create as core_create, detect, extract as core_extract, list as core_list, test as core_test,
    ArchiveInfo, CreateOptions, CreateReport, Error, ExtractOptions, ExtractReport, Format, Level,
    ListOptions, MatchMode, Progress, TestReport,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

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
///
/// Goes to a file as well as stderr: when the OS launches the app (rather than
/// a shell), there is no terminal to print to.
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
async fn extract_archive(
    app: AppHandle,
    running: State<'_, Arc<Running>>,
    path: String,
    dest: String,
    password: Option<String>,
    overwrite: bool,
    include: Option<Vec<String>>,
    // `exact` treats `include` as complete entry paths rather than patterns —
    // what the window sends when the user has ticked specific rows.
    // `keep_broken` carries on past entries that cannot be extracted, listing
    // them in the report, instead of stopping at the first one.
    exact: Option<bool>,
    keep_broken: Option<bool>,
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
        };
        core_extract(&path, &opts, Some(progress))
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
        std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn()
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
        std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn()
    } else {
        // No portable "select the file" on Linux; open the containing folder.
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(Running::default()))
        .manage(PendingFile::default())
        .setup(|app| {
            if let Some(path) = archive_from_args() {
                if let Ok(mut pending) = app.state::<PendingFile>().0.lock() {
                    *pending = Some(path);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            detect_format,
            list_archive,
            test_archive,
            extract_archive,
            create_archive,
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
        // macOS delivers "Open with" and dock drops as an event, which can
        // arrive either before the webview is listening or long after.
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
