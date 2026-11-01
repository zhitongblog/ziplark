//! CLI wrapper around `ziplark_core::shell`.

use std::process::ExitCode;
use ziplark_core::shell;

pub fn run(args: &[String]) -> anyhow::Result<ExitCode> {
    let action = args.first().map(String::as_str).unwrap_or("status");
    match action {
        "install" => {
            shell::install()?;
            println!("Ziplark right-click menu installed.");
            #[cfg(target_os = "linux")]
            println!("(Log out/in or restart the file manager if items don't appear yet.)");
        }
        "uninstall" => {
            shell::uninstall()?;
            println!("Ziplark right-click menu removed.");
        }
        "status" => {
            let st = shell::status()?;
            if !st.supported {
                println!("  (unsupported platform)");
            } else {
                for (label, present) in st.details {
                    println!("  {label:<18} {}", if present { "installed" } else { "—" });
                }
            }
        }
        "-h" | "--help" | "help" => {
            println!("usage: ziplark shell-integration <install|uninstall|status>");
        }
        other => anyhow::bail!("unknown action '{other}' (install|uninstall|status)"),
    }
    Ok(ExitCode::SUCCESS)
}
