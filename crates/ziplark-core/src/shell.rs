//! OS file-manager right-click integration (`Extract here` / `Compress to ZIP`),
//! shared by both the `ziplark` CLI and the Ziplark desktop app.
//!
//! Per platform:
//!   • macOS   — Automator Quick Actions (Services) in `~/Library/Services`
//!   • Windows — `HKCU` shell verbs (per-user, no admin required)
//!   • Linux   — KDE Plasma service menus + GNOME Nautilus scripts

use crate::error::{Error, Result};
use crate::model::{CreateReport, ExtractReport, Format};
use crate::{
    create, extract, list, smart_dest, unique_sibling_dir, CreateOptions, ExtractOptions, Level,
    ListOptions, MatchMode,
};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Current status of the OS file-manager context menu integration.
#[derive(Debug, Clone, Serialize)]
pub struct ShellStatus {
    pub supported: bool,
    pub installed: bool,
    pub platform: &'static str,
    pub details: Vec<(String, bool)>,
}

/// Extract each archive into a sibling destination using smart anti-tarbomb
/// folder rules (used by the OS right-click "Extract here with Ziplark" action).
pub fn extract_here_batch(
    archives: &[String],
    password: Option<String>,
    keep_broken: bool,
) -> Result<Vec<ExtractReport>> {
    if archives.is_empty() {
        return Err(Error::other("extract-here requires at least one archive"));
    }
    let mut reports = Vec::new();
    let mut errors = Vec::new();

    for a in archives {
        let path = Path::new(a);
        let parent = path.parent().unwrap_or(Path::new("."));
        let dest = match list(
            path,
            &ListOptions {
                password: password.clone(),
            },
        ) {
            Ok(info) => smart_dest(&info.path, parent, &info.entries, true),
            Err(_) => {
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or(a);
                unique_sibling_dir(parent.join(crate::archive_stem(name)))
            }
        };
        let opts = ExtractOptions {
            password: password.clone(),
            dest,
            overwrite: true,
            include: Vec::new(),
            match_mode: MatchMode::Auto,
            keep_broken,
            smart: false,
        };
        match extract(path, &opts, None) {
            Ok(r) => reports.push(r),
            Err(e) => errors.push(format!("{a}: {e}")),
        }
    }

    if errors.is_empty() {
        notify("Ziplark", &format!("Extracted {} archive(s)", reports.len()));
        Ok(reports)
    } else {
        let msg = errors.join("; ");
        notify("Ziplark — extract failed", &msg);
        Err(Error::other(msg))
    }
}

/// Compress `items` into a sibling `.zip` archive (used by the OS right-click
/// "Compress to ZIP with Ziplark" action).
pub fn compress_zip_batch(
    items: &[String],
    level: Level,
    password: Option<String>,
) -> Result<CreateReport> {
    if items.is_empty() {
        return Err(Error::other(
            "compress-zip requires at least one file or folder",
        ));
    }
    let first = Path::new(&items[0]);
    let parent = first.parent().unwrap_or(Path::new(".")).to_path_buf();
    let out = if items.len() == 1 {
        let base = first.file_name().and_then(|s| s.to_str()).unwrap_or("Archive");
        parent.join(format!("{base}.zip"))
    } else {
        parent.join("Archive.zip")
    };
    let out = unique_file_path(out);
    let inputs: Vec<PathBuf> = items.iter().map(PathBuf::from).collect();
    let opts = CreateOptions {
        format: Format::Zip,
        level,
        password,
        split_size: None,
    };
    let report = create(&out, &inputs, &opts, None)?;
    notify(
        "Ziplark",
        &format!(
            "Created {}",
            out.file_name().and_then(|s| s.to_str()).unwrap_or("archive")
        ),
    );
    Ok(report)
}

fn unique_file_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let ext = path.extension().and_then(|s| s.to_str());
    for n in 2..1000 {
        let name = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let cand = parent.join(name);
        if !cand.exists() {
            return cand;
        }
    }
    path
}

fn notify(title: &str, body: &str) {
    if cfg!(target_os = "macos") {
        let script = format!("display notification {:?} with title {:?}", body, title);
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .status();
    }
}

fn current_exe_string() -> Result<String> {
    Ok(std::env::current_exe()?.to_string_lossy().into_owned())
}

pub fn install() -> Result<()> {
    imp::install()
}

pub fn uninstall() -> Result<()> {
    imp::uninstall()
}

pub fn status() -> Result<ShellStatus> {
    imp::status()
}

// ───────────────────────────── macOS ─────────────────────────────
#[cfg(target_os = "macos")]
mod imp {
    use super::{current_exe_string, Error, Result, ShellStatus};
    use std::fs;
    use std::path::{Path, PathBuf};

    const EXTRACT_WF: &str = "Extract here with Ziplark.workflow";
    const COMPRESS_WF: &str = "Compress to ZIP with Ziplark.workflow";

    fn services_dir() -> Result<PathBuf> {
        let home = std::env::var("HOME").map_err(|_| Error::other("HOME is not set"))?;
        Ok(PathBuf::from(home).join("Library/Services"))
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn write_workflow(
        dir: &Path,
        menu_name: &str,
        send_type: &str,
        command: &str,
    ) -> Result<()> {
        let contents = dir.join("Contents");
        fs::create_dir_all(&contents)?;
        let info = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>NSServices</key>
  <array>
    <dict>
      <key>NSMenuItem</key>
      <dict><key>default</key><string>{name}</string></dict>
      <key>NSMessage</key><string>runWorkflowAsService</string>
      <key>NSRequiredContext</key>
      <dict><key>NSApplicationIdentifier</key><string>com.apple.finder</string></dict>
      <key>NSSendFileTypes</key>
      <array><string>{stype}</string></array>
    </dict>
  </array>
</dict>
</plist>
"#,
            name = xml_escape(menu_name),
            stype = send_type
        );
        let wflow = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>AMApplicationBuild</key><string>523</string>
  <key>AMApplicationVersion</key><string>2.10</string>
  <key>AMDocumentVersion</key><string>2</string>
  <key>actions</key>
  <array>
    <dict>
      <key>action</key>
      <dict>
        <key>AMAccepts</key>
        <dict><key>Container</key><string>List</string><key>Optional</key><false/><key>Types</key><array><string>com.apple.cocoa.path</string></array></dict>
        <key>AMActionVersion</key><string>2.0.3</string>
        <key>AMApplication</key><array><string>Automator</string></array>
        <key>AMProvides</key>
        <dict><key>Container</key><string>List</string><key>Types</key><array><string>com.apple.cocoa.path</string></array></dict>
        <key>ActionBundlePath</key><string>/System/Library/Automator/Run Shell Script.action</string>
        <key>ActionName</key><string>Run Shell Script</string>
        <key>ActionParameters</key>
        <dict>
          <key>COMMAND_STRING</key><string>{cmd}</string>
          <key>CheckedForUserDefaultShell</key><true/>
          <key>inputMethod</key><integer>1</integer>
          <key>shell</key><string>/bin/bash</string>
          <key>source</key><string></string>
        </dict>
        <key>BundleIdentifier</key><string>com.apple.RunShellScript</string>
        <key>Class Name</key><string>RunShellScriptAction</string>
        <key>InputUUID</key><string>11111111-1111-1111-1111-111111111111</string>
        <key>UUID</key><string>22222222-2222-2222-2222-222222222222</string>
        <key>arguments</key><dict/>
        <key>isViewVisible</key><integer>1</integer>
      </dict>
    </dict>
  </array>
  <key>connectors</key><dict/>
  <key>workflowMetaData</key>
  <dict>
    <key>serviceInputTypeIdentifier</key><string>com.apple.Automator.fileSystemObject</string>
    <key>serviceOutputTypeIdentifier</key><string>com.apple.Automator.nothing</string>
    <key>serviceApplicationBundleID</key><string>com.apple.finder</string>
    <key>serviceApplicationPath</key><string>/System/Library/CoreServices/Finder.app</string>
    <key>workflowTypeIdentifier</key><string>com.apple.Automator.servicesMenu</string>
  </dict>
</dict>
</plist>
"#,
            cmd = xml_escape(command)
        );
        fs::write(contents.join("Info.plist"), info)?;
        fs::write(contents.join("document.wflow"), wflow)?;
        Ok(())
    }

    fn refresh() {
        let _ = std::process::Command::new("/System/Library/CoreServices/pbs")
            .arg("-flush")
            .status();
        let _ = std::process::Command::new("/System/Library/CoreServices/pbs")
            .arg("-update")
            .status();
    }

    pub fn install() -> Result<()> {
        let bin = current_exe_string()?;
        let dir = services_dir()?;
        write_workflow(
            &dir.join(EXTRACT_WF),
            "Extract here with Ziplark",
            "public.data",
            &format!("\"{bin}\" extract-here \"$@\""),
        )?;
        write_workflow(
            &dir.join(COMPRESS_WF),
            "Compress to ZIP with Ziplark",
            "public.item",
            &format!("\"{bin}\" compress-zip \"$@\""),
        )?;
        refresh();
        Ok(())
    }

    pub fn uninstall() -> Result<()> {
        let dir = services_dir()?;
        for wf in [EXTRACT_WF, COMPRESS_WF] {
            let p = dir.join(wf);
            if p.exists() {
                fs::remove_dir_all(&p)?;
            }
        }
        refresh();
        Ok(())
    }

    pub fn status() -> Result<ShellStatus> {
        let dir = services_dir()?;
        let ex = dir.join(EXTRACT_WF).exists();
        let cm = dir.join(COMPRESS_WF).exists();
        Ok(ShellStatus {
            supported: true,
            installed: ex && cm,
            platform: "macOS Finder Quick Actions",
            details: vec![
                ("Extract here".into(), ex),
                ("Compress to ZIP".into(), cm),
            ],
        })
    }
}

// ───────────────────────────── Windows ─────────────────────────────
#[cfg(target_os = "windows")]
mod imp {
    use super::{current_exe_string, Error, Result, ShellStatus};
    use std::process::Command;

    const EXTS: &[&str] = &[
        ".zip", ".7z", ".rar", ".tar", ".gz", ".bz2", ".xz", ".zst", ".lz4", ".tgz", ".tbz2",
        ".txz", ".tzst", ".tlz4", ".iso", ".deb", ".rpm", ".cpio", ".ar", ".cab", ".jar", ".apk",
        ".whl", ".nupkg", ".crx", ".cbz", ".cbr", ".cb7", ".z01", ".001", ".r00", ".r01", ".r02",
        ".r03", ".r04", ".r05", ".r06", ".r07", ".r08", ".r09",
    ];
    const EXTRACT_VERB: &str = "Ziplark.Extract";
    const COMPRESS_VERB: &str = "Ziplark.Compress";

    fn reg_add(key: &str, value_name: Option<&str>, data: &str) -> Result<()> {
        let mut c = Command::new("reg");
        c.args(["add", key]);
        match value_name {
            Some(v) => {
                c.args(["/v", v]);
            }
            None => {
                c.arg("/ve");
            }
        }
        c.args(["/t", "REG_SZ", "/d", data, "/f"]);
        let st = c.status()?;
        if !st.success() {
            return Err(Error::other(format!("reg add failed for {key}")));
        }
        Ok(())
    }

    fn reg_delete(key: &str) {
        let _ = Command::new("reg").args(["delete", key, "/f"]).status();
    }

    pub fn install() -> Result<()> {
        let bin = current_exe_string()?;
        for ext in EXTS {
            let base = format!(
                "HKCU\\Software\\Classes\\SystemFileAssociations\\{ext}\\shell\\{EXTRACT_VERB}"
            );
            reg_add(&base, None, "Extract here with Ziplark")?;
            reg_add(&base, Some("Icon"), &bin)?;
            reg_add(
                &format!("{base}\\command"),
                None,
                &format!("\"{bin}\" extract-here \"%1\""),
            )?;
        }
        for root in ["*", "Directory"] {
            let base = format!("HKCU\\Software\\Classes\\{root}\\shell\\{COMPRESS_VERB}");
            reg_add(&base, None, "Compress to ZIP with Ziplark")?;
            reg_add(&base, Some("Icon"), &bin)?;
            reg_add(
                &format!("{base}\\command"),
                None,
                &format!("\"{bin}\" compress-zip \"%1\""),
            )?;
        }
        Ok(())
    }

    pub fn uninstall() -> Result<()> {
        for ext in EXTS {
            reg_delete(&format!(
                "HKCU\\Software\\Classes\\SystemFileAssociations\\{ext}\\shell\\{EXTRACT_VERB}"
            ));
        }
        for root in ["*", "Directory"] {
            reg_delete(&format!(
                "HKCU\\Software\\Classes\\{root}\\shell\\{COMPRESS_VERB}"
            ));
        }
        Ok(())
    }

    pub fn status() -> Result<ShellStatus> {
        let key = format!(
            "HKCU\\Software\\Classes\\SystemFileAssociations\\.zip\\shell\\{EXTRACT_VERB}"
        );
        let present = Command::new("reg")
            .args(["query", &key])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        Ok(ShellStatus {
            supported: true,
            installed: present,
            platform: "Windows Explorer context menu",
            details: vec![("Extract / Compress".into(), present)],
        })
    }
}

// ───────────────────────────── Linux ─────────────────────────────
#[cfg(target_os = "linux")]
mod imp {
    use super::{current_exe_string, Result, ShellStatus};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn data_home() -> PathBuf {
        std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share")
            })
    }

    fn kde_dir() -> PathBuf {
        data_home().join("kio/servicemenus")
    }
    fn nautilus_dir() -> PathBuf {
        data_home().join("nautilus/scripts")
    }

    const ARCHIVE_MIMES: &str = "application/zip;application/x-7z-compressed;application/vnd.rar;\
application/x-tar;application/gzip;application/x-bzip2;application/x-xz;application/zstd;\
application/x-compressed-tar;application/x-bzip-compressed-tar;application/x-xz-compressed-tar;\
application/vnd.debian.binary-package;application/x-rpm;application/x-cpio;application/vnd.ms-cab-compressed;";

    pub fn install() -> Result<()> {
        let bin = current_exe_string()?;
        let kde = kde_dir();
        fs::create_dir_all(&kde)?;
        fs::write(
            kde.join("ziplark-extract.desktop"),
            format!(
                "[Desktop Entry]\nType=Service\nServiceTypes=KonqPopupMenu/Plugin\n\
MimeType={ARCHIVE_MIMES}\nActions=ziplarkExtract;\nX-KDE-Priority=TopLevel\n\n\
[Desktop Action ziplarkExtract]\nName=Extract here with Ziplark\nIcon=ziplark\n\
Exec={bin} extract-here %F\n"
            ),
        )?;
        fs::write(
            kde.join("ziplark-compress.desktop"),
            format!(
                "[Desktop Entry]\nType=Service\nServiceTypes=KonqPopupMenu/Plugin\n\
MimeType=all/allfiles;inode/directory;\nActions=ziplarkCompress;\nX-KDE-Priority=TopLevel\n\n\
[Desktop Action ziplarkCompress]\nName=Compress to ZIP with Ziplark\nIcon=ziplark\n\
Exec={bin} compress-zip %F\n"
            ),
        )?;

        let naut = nautilus_dir();
        fs::create_dir_all(&naut)?;
        write_script(
            &naut.join("Extract here with Ziplark"),
            &format!(
                "#!/usr/bin/env bash\nIFS=$'\\n'\nexec {bin} extract-here \
$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\n"
            ),
        )?;
        write_script(
            &naut.join("Compress to ZIP with Ziplark"),
            &format!(
                "#!/usr/bin/env bash\nIFS=$'\\n'\nexec {bin} compress-zip \
$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\n"
            ),
        )?;
        Ok(())
    }

    fn write_script(path: &PathBuf, body: &str) -> Result<()> {
        fs::write(path, body)?;
        let mut perm = fs::metadata(path)?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(path, perm)?;
        Ok(())
    }

    pub fn uninstall() -> Result<()> {
        for p in [
            kde_dir().join("ziplark-extract.desktop"),
            kde_dir().join("ziplark-compress.desktop"),
            nautilus_dir().join("Extract here with Ziplark"),
            nautilus_dir().join("Compress to ZIP with Ziplark"),
        ] {
            if p.exists() {
                let _ = fs::remove_file(&p);
            }
        }
        Ok(())
    }

    pub fn status() -> Result<ShellStatus> {
        let kde = kde_dir().join("ziplark-extract.desktop").exists();
        let naut = nautilus_dir().join("Extract here with Ziplark").exists();
        Ok(ShellStatus {
            supported: true,
            installed: kde && naut,
            platform: "KDE & Nautilus menus",
            details: vec![
                ("KDE service menu".into(), kde),
                ("Nautilus script".into(), naut),
            ],
        })
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod imp {
    use super::{Error, Result, ShellStatus};
    pub fn install() -> Result<()> {
        Err(Error::other(
            "shell integration is not supported on this platform",
        ))
    }
    pub fn uninstall() -> Result<()> {
        Ok(())
    }
    pub fn status() -> Result<ShellStatus> {
        Ok(ShellStatus {
            supported: false,
            installed: false,
            platform: "unsupported",
            details: Vec::new(),
        })
    }
}
