//! OS file-manager right-click integration.
//!
//! Registers two context-menu actions, both backed by the `ziplark` CLI:
//!   • "Extract here with Ziplark"   → `ziplark extract-here <files>`
//!   • "Compress to ZIP with Ziplark" → `ziplark compress-zip <items>`
//!
//! Per platform:
//!   • macOS   — Automator Quick Actions (Services) in ~/Library/Services
//!   • Windows — HKCU shell verbs (per-user, no admin)
//!   • Linux   — KDE servicemenus + Nautilus scripts
//!
//! All entries point at the *current* executable, so the integration follows
//! wherever ziplark is installed. `install` is idempotent; `uninstall` removes
//! everything; `status` reports what is present.

use std::process::ExitCode;

pub fn run(args: &[String]) -> anyhow::Result<ExitCode> {
    let action = args.first().map(String::as_str).unwrap_or("status");
    match action {
        "install" => {
            install()?;
            println!("Ziplark right-click menu installed.");
            #[cfg(target_os = "linux")]
            println!("(Log out/in or restart the file manager if items don't appear yet.)");
        }
        "uninstall" => {
            uninstall()?;
            println!("Ziplark right-click menu removed.");
        }
        "status" => status()?,
        "-h" | "--help" | "help" => {
            println!("usage: ziplark shell-integration <install|uninstall|status>");
        }
        other => anyhow::bail!("unknown action '{other}' (install|uninstall|status)"),
    }
    Ok(ExitCode::SUCCESS)
}

fn exe() -> anyhow::Result<String> {
    Ok(std::env::current_exe()?.to_string_lossy().into_owned())
}

// ───────────────────────────── macOS ─────────────────────────────
#[cfg(target_os = "macos")]
mod imp {
    use super::exe;
    use std::fs;
    use std::path::PathBuf;

    const EXTRACT_WF: &str = "Extract here with Ziplark.workflow";
    const COMPRESS_WF: &str = "Compress to ZIP with Ziplark.workflow";

    fn services_dir() -> anyhow::Result<PathBuf> {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(home).join("Library/Services"))
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    /// Write one Automator Quick Action bundle.
    fn write_workflow(
        dir: &PathBuf,
        menu_name: &str,
        send_type: &str,
        command: &str,
    ) -> anyhow::Result<()> {
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

    pub fn install() -> anyhow::Result<()> {
        let bin = exe()?;
        let dir = services_dir()?;
        // public.data → files (archives); public.item → files *and* folders.
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

    pub fn uninstall() -> anyhow::Result<()> {
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

    pub fn status() -> anyhow::Result<()> {
        let dir = services_dir()?;
        for (label, wf) in [("Extract here", EXTRACT_WF), ("Compress to ZIP", COMPRESS_WF)] {
            let present = dir.join(wf).exists();
            println!("  {label:<16} {}", if present { "installed" } else { "—" });
        }
        Ok(())
    }
}

// ───────────────────────────── Windows ─────────────────────────────
#[cfg(target_os = "windows")]
mod imp {
    use super::exe;
    use std::process::Command;

    // Archive extensions that get an "Extract here" verb.
    const EXTS: &[&str] = &[
        ".zip", ".7z", ".rar", ".tar", ".gz", ".bz2", ".xz", ".zst", ".tgz", ".tbz2", ".txz",
        ".tzst",
    ];
    const EXTRACT_VERB: &str = "Ziplark.Extract";
    const COMPRESS_VERB: &str = "Ziplark.Compress";

    fn reg_add(key: &str, value_name: Option<&str>, data: &str) -> anyhow::Result<()> {
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
            anyhow::bail!("reg add failed for {key}");
        }
        Ok(())
    }

    fn reg_delete(key: &str) {
        let _ = Command::new("reg").args(["delete", key, "/f"]).status();
    }

    pub fn install() -> anyhow::Result<()> {
        let bin = exe()?;
        // Extract verb on each archive extension (per-user, non-invasive).
        for ext in EXTS {
            let base =
                format!("HKCU\\Software\\Classes\\SystemFileAssociations\\{ext}\\shell\\{EXTRACT_VERB}");
            reg_add(&base, None, "Extract here with Ziplark")?;
            reg_add(&base, Some("Icon"), &bin)?;
            reg_add(
                &format!("{base}\\command"),
                None,
                &format!("\"{bin}\" extract-here \"%1\""),
            )?;
        }
        // Compress verb on all files and on folders.
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

    pub fn uninstall() -> anyhow::Result<()> {
        for ext in EXTS {
            reg_delete(&format!(
                "HKCU\\Software\\Classes\\SystemFileAssociations\\{ext}\\shell\\{EXTRACT_VERB}"
            ));
        }
        for root in ["*", "Directory"] {
            reg_delete(&format!("HKCU\\Software\\Classes\\{root}\\shell\\{COMPRESS_VERB}"));
        }
        Ok(())
    }

    pub fn status() -> anyhow::Result<()> {
        let key = format!(
            "HKCU\\Software\\Classes\\SystemFileAssociations\\.zip\\shell\\{EXTRACT_VERB}"
        );
        let present = Command::new("reg")
            .args(["query", &key])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        println!("  Extract / Compress  {}", if present { "installed" } else { "—" });
        Ok(())
    }
}

// ───────────────────────────── Linux ─────────────────────────────
#[cfg(target_os = "linux")]
mod imp {
    use super::exe;
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
application/x-compressed-tar;application/x-bzip-compressed-tar;application/x-xz-compressed-tar;";

    pub fn install() -> anyhow::Result<()> {
        let bin = exe()?;

        // --- KDE / Plasma service menus ---
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

        // --- Nautilus (GNOME) scripts: selected paths via env var ---
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

    fn write_script(path: &PathBuf, body: &str) -> anyhow::Result<()> {
        fs::write(path, body)?;
        let mut perm = fs::metadata(path)?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(path, perm)?;
        Ok(())
    }

    pub fn uninstall() -> anyhow::Result<()> {
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

    pub fn status() -> anyhow::Result<()> {
        let kde = kde_dir().join("ziplark-extract.desktop").exists();
        let naut = nautilus_dir().join("Extract here with Ziplark").exists();
        println!("  KDE service menu    {}", if kde { "installed" } else { "—" });
        println!("  Nautilus script     {}", if naut { "installed" } else { "—" });
        Ok(())
    }
}

// Fallback for unsupported OSes.
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod imp {
    pub fn install() -> anyhow::Result<()> {
        anyhow::bail!("shell integration is not supported on this platform")
    }
    pub fn uninstall() -> anyhow::Result<()> {
        Ok(())
    }
    pub fn status() -> anyhow::Result<()> {
        println!("  (unsupported platform)");
        Ok(())
    }
}

use imp::{install, status, uninstall};
