//! Ziplark desktop backend. Thin Tauri commands over `ziplark-core` — all the real
//! work lives in the shared engine, so the GUI behaves identically to the CLI.

use ziplark_core::{
    create as core_create, detect, extract as core_extract, list as core_list, test as core_test,
    ArchiveInfo, CreateOptions, CreateReport, ExtractOptions, ExtractReport, Format, Level,
    ListOptions, TestReport,
};
use std::path::PathBuf;

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

#[tauri::command]
fn detect_format(path: String) -> Option<String> {
    detect(std::path::Path::new(&path)).map(|f| f.label().to_string())
}

#[tauri::command]
fn list_archive(path: String, password: Option<String>) -> Result<ArchiveInfo, String> {
    core_list(&path, &ListOptions { password }).map_err(|e| e.to_string())
}

#[tauri::command]
fn test_archive(path: String, password: Option<String>) -> Result<TestReport, String> {
    core_test(&path, &ListOptions { password }, None).map_err(|e| e.to_string())
}

#[tauri::command]
fn extract_archive(
    path: String,
    dest: String,
    password: Option<String>,
    overwrite: bool,
) -> Result<ExtractReport, String> {
    let opts = ExtractOptions {
        password,
        dest: PathBuf::from(dest),
        overwrite,
        include: Vec::new(),
    };
    core_extract(&path, &opts, None).map_err(|e| e.to_string())
}

#[tauri::command]
fn create_archive(
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
    let inputs: Vec<PathBuf> = inputs.into_iter().map(PathBuf::from).collect();
    core_create(&output, &inputs, &opts, None).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            detect_format,
            list_archive,
            test_archive,
            extract_archive,
            create_archive
        ])
        .run(tauri::generate_context!())
        .expect("error while running Ziplark");
}
