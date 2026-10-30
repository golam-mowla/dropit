use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APP_NAME: &str = "DropIt";

/// Retrieves current executable path
pub fn current_exe_path() -> Result<PathBuf> {
    std::env::current_exe().context("Failed to get current executable path")
}

/// Enable or disable Windows startup
pub fn set_run_on_startup(enable: bool) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(RUN_KEY)
        .context("Failed to open Windows Run registry key")?;

    if enable {
        let exe = current_exe_path()?;
        let cmd = format!("\"{}\" --tray", exe.display());
        key.set_value(APP_NAME, &cmd)
            .context("Failed to write to Run registry key")?;
    } else {
        let _ = key.delete_value(APP_NAME);
    }

    Ok(())
}

/// Check if Windows startup is enabled
pub fn get_run_on_startup() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ) {
        key.get_value::<String, _>(APP_NAME).is_ok()
    } else {
        false
    }
}

/// Enable or disable Windows Explorer Context Menu: "Send with Drop It"
pub fn set_context_menu_enabled(enable: bool) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let exe = current_exe_path()?;
    let exe_str = exe.display().to_string();
    let cmd_str = format!("\"{}\" send \"%1\"", exe_str);

    let targets = [
        r"Software\Classes\*\shell\DropIt",
        r"Software\Classes\Directory\shell\DropIt",
    ];

    if enable {
        for target in &targets {
            let (key, _) = hkcu
                .create_subkey(target)
                .with_context(|| format!("Failed to create registry key {}", target))?;
            let _ = key.set_value("", &"Send with Drop It");
            let _ = key.set_value("Icon", &exe_str);

            let cmd_path = format!(r"{}\command", target);
            let (cmd_key, _) = hkcu
                .create_subkey(&cmd_path)
                .with_context(|| format!("Failed to create command key {}", cmd_path))?;
            let _ = cmd_key.set_value("", &cmd_str);
        }
    } else {
        for target in &targets {
            let _ = hkcu.delete_subkey_all(target);
        }
    }

    Ok(())
}

/// Check if Windows Explorer Context Menu is enabled
pub fn get_context_menu_enabled() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey_with_flags(r"Software\Classes\*\shell\DropIt", KEY_READ)
        .is_ok()
}

/// Open a file with the default Windows application
pub fn open_file(path: &Path) {
    let _ = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", &path.display().to_string()])
        .spawn();
}

/// Highlight and open a file in Windows Explorer folder
pub fn open_in_folder(path: &Path) {
    if path.is_file() {
        let arg = format!("/select,\"{}\"", path.display());
        let _ = std::process::Command::new("explorer.exe").arg(arg).spawn();
    } else {
        let _ = std::process::Command::new("explorer.exe")
            .arg(path.display().to_string())
            .spawn();
    }
}

/// Load the circular icon from bytes into an RGBA image buffer
pub fn load_app_icon_rgba() -> Result<(Vec<u8>, u32, u32)> {
    let bytes = include_bytes!("../assets/icon.png");
    let img = image::load_from_memory(bytes)
        .context("Failed to decode icon.png")?
        .into_rgba8();
    let (w, h) = img.dimensions();
    Ok((img.into_raw(), w, h))
}
