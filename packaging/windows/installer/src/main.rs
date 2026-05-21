use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const APP_NAME: &str = "ipssh";
const VERSION: &str = "0.1.2";
const EXE_BYTES: &[u8] = include_bytes!(env!("IPSSH_BIN"));
const DEFAULT_CONFIG: &str = include_str!("../../default-config.toml");

fn main() {
    let pause_before_exit = should_pause_after_run(current_console_process_count());
    let result = run();
    if let Err(error) = result {
        eprintln!("ipssh installer failed: {error}");
        if pause_before_exit {
            wait_for_enter();
        }
        std::process::exit(1);
    }

    if pause_before_exit {
        wait_for_enter();
    }
}

fn run() -> io::Result<()> {
    let args: Vec<OsString> = env::args_os().skip(1).collect();

    if has_arg(&args, "--help") || has_arg(&args, "/?") {
        print_help();
        return Ok(());
    }

    let install_dir = install_dir()?;

    if has_arg(&args, "--uninstall") || has_arg(&args, "/uninstall") {
        uninstall(&install_dir)?;
        return Ok(());
    }

    let update_path = !has_arg(&args, "--no-path");
    install(&install_dir, update_path)?;
    Ok(())
}

fn print_help() {
    println!("ipssh installer {VERSION}");
    println!();
    println!("Usage:");
    println!("  ipssh-installer.exe              Install ipssh for the current user");
    println!("  ipssh-installer.exe --no-path    Install without modifying user PATH");
    println!("  ipssh-installer.exe --uninstall  Remove the installed files and PATH entry");
    println!("  ipssh-installer.exe --help       Show this help");
}

fn should_pause_after_run(console_process_count: Option<u32>) -> bool {
    matches!(console_process_count, Some(0 | 1))
}

#[cfg(windows)]
fn current_console_process_count() -> Option<u32> {
    extern "system" {
        fn GetConsoleProcessList(process_list: *mut u32, process_count: u32) -> u32;
    }

    let mut process_ids = [0_u32; 16];
    let count =
        unsafe { GetConsoleProcessList(process_ids.as_mut_ptr(), process_ids.len() as u32) };
    if count == 0 {
        None
    } else {
        Some(count)
    }
}

#[cfg(not(windows))]
fn current_console_process_count() -> Option<u32> {
    None
}

fn wait_for_enter() {
    eprintln!();
    eprintln!("Press Enter to close this window...");
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
}

fn install(install_dir: &Path, update_path: bool) -> io::Result<()> {
    fs::create_dir_all(install_dir)?;

    let exe_path = install_dir.join("ipssh.exe");
    let mut exe = fs::File::create(&exe_path)?;
    exe.write_all(EXE_BYTES)?;
    exe.flush()?;

    if update_path {
        add_to_user_path(install_dir)?;
    }
    let config_path = install_default_config()?;

    println!("Installed ipssh {VERSION}");
    println!("Binary: {}", exe_path.display());
    println!("Config: {}", config_path.display());
    if update_path {
        println!("PATH: added {}", install_dir.display());
        println!("Open a new terminal window before running `ipssh` from PATH.");
    }
    Ok(())
}

fn uninstall(install_dir: &Path) -> io::Result<()> {
    remove_from_user_path(install_dir)?;

    let exe_path = install_dir.join("ipssh.exe");
    if exe_path.exists() {
        fs::remove_file(&exe_path)?;
    }

    if install_dir.exists() && install_dir.read_dir()?.next().is_none() {
        fs::remove_dir(install_dir)?;
    }

    println!("Uninstalled ipssh");
    println!("User config was left in place.");
    Ok(())
}

fn install_dir() -> io::Result<PathBuf> {
    let local_app_data = env::var_os("LOCALAPPDATA").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "LOCALAPPDATA is not set; cannot choose a per-user install directory",
        )
    })?;
    Ok(PathBuf::from(local_app_data)
        .join("Programs")
        .join(APP_NAME))
}

fn install_default_config() -> io::Result<PathBuf> {
    let app_data = env::var_os("APPDATA").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "APPDATA is not set; cannot choose a per-user config directory",
        )
    })?;
    install_default_config_in(Path::new(&app_data))
}

fn install_default_config_in(app_data: &Path) -> io::Result<PathBuf> {
    let config_path = config_path_from_app_data(app_data);
    let config_dir = config_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "failed to choose a per-user config directory",
        )
    })?;

    fs::create_dir_all(&config_dir)?;
    if !config_path.exists() {
        let legacy_config_path = legacy_config_path_from_app_data(app_data);
        if legacy_config_path.exists() {
            fs::copy(&legacy_config_path, &config_path)?;
        } else {
            fs::write(&config_path, DEFAULT_CONFIG)?;
        }
    }

    Ok(config_path)
}

fn config_path_from_app_data(app_data: &Path) -> PathBuf {
    app_data.join(APP_NAME).join("config.toml")
}

fn legacy_config_path_from_app_data(app_data: &Path) -> PathBuf {
    app_data.join("image-paste-ssh").join("config.toml")
}

fn has_arg(args: &[OsString], value: &str) -> bool {
    args.iter()
        .any(|arg| arg.to_string_lossy().eq_ignore_ascii_case(value))
}

fn add_to_user_path(path: &Path) -> io::Result<()> {
    let path_text = path.to_string_lossy();
    let current = read_user_path()?;
    let mut entries = split_path_list(&current);

    if !entries
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(&path_text))
    {
        entries.push(path_text.to_string());
        write_user_path(&entries.join(";"))?;
    }

    Ok(())
}

fn remove_from_user_path(path: &Path) -> io::Result<()> {
    let path_text = path.to_string_lossy();
    let current = read_user_path()?;
    let entries: Vec<String> = split_path_list(&current)
        .into_iter()
        .filter(|entry| !entry.eq_ignore_ascii_case(&path_text))
        .collect();
    write_user_path(&entries.join(";"))
}

fn split_path_list(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn read_user_path() -> io::Result<String> {
    let output = Command::new("reg")
        .args(["query", r"HKCU\Environment", "/v", "Path"])
        .output()?;

    if !output.status.success() {
        return Ok(String::new());
    }

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("Path") {
            let parts: Vec<&str> = trimmed.splitn(3, char::is_whitespace).collect();
            if parts.len() == 3 {
                return Ok(parts[2].trim().to_string());
            }
        }
    }

    Ok(String::new())
}

fn write_user_path(value: &str) -> io::Result<()> {
    let status = Command::new("reg")
        .args([
            "add",
            r"HKCU\Environment",
            "/v",
            "Path",
            "/t",
            "REG_EXPAND_SZ",
            "/d",
        ])
        .arg(value)
        .args(["/f"])
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "failed to update HKCU Environment Path",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_uses_ipssh_app_directory() {
        let path = config_path_from_app_data(Path::new(r"C:\Users\me\AppData\Roaming"));
        assert_eq!(
            path,
            PathBuf::from(r"C:\Users\me\AppData\Roaming\ipssh\config.toml")
        );
    }

    #[test]
    fn install_default_config_migrates_existing_legacy_config() {
        let root = env::temp_dir().join(format!("ipssh-installer-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let app_data = root.join("Roaming");
        let legacy_dir = app_data.join("image-paste-ssh");
        let legacy_path = legacy_dir.join("config.toml");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(&legacy_path, "remote_dir = \"~/custom\"").unwrap();

        let installed_path = install_default_config_in(&app_data).unwrap();

        assert_eq!(installed_path, app_data.join("ipssh").join("config.toml"));
        assert_eq!(
            fs::read_to_string(installed_path).unwrap(),
            "remote_dir = \"~/custom\""
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pauses_when_installer_owns_the_console() {
        assert!(should_pause_after_run(Some(1)));
    }

    #[test]
    fn does_not_pause_when_launched_from_an_existing_terminal() {
        assert!(!should_pause_after_run(Some(2)));
    }

    #[test]
    fn does_not_pause_when_console_process_count_is_unavailable() {
        assert!(!should_pause_after_run(None));
    }
}
