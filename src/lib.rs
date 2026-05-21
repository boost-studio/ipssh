pub mod clipboard;
pub mod config;
pub mod hotkey;
pub mod remote_path;
pub mod ssh_args;
pub mod template;
pub mod terminal;
pub mod uploader;

use clap::Parser;
use config::{AppConfig, ConfigOverrides};
use ssh_args::SshInvocation;
use std::env;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "ipssh")]
#[command(about = "OpenSSH wrapper that uploads pasted clipboard images")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    paste_hotkey: Option<String>,
    #[arg(long)]
    remote_dir: Option<String>,
    #[arg(long)]
    image_format: Option<String>,
    #[arg(long)]
    non_image_paste: Option<String>,
    #[arg(long)]
    template: Option<String>,
    #[arg(long)]
    filename_pattern: Option<String>,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    ssh_args: Vec<String>,
}

pub fn main_entry() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.or_else(default_config_path);
    let config = AppConfig::load(
        config_path.as_deref(),
        ConfigOverrides {
            paste_hotkey: cli.paste_hotkey,
            remote_dir: cli.remote_dir,
            image_format: cli.image_format,
            non_image_paste: cli.non_image_paste,
            template: cli.template,
            filename_pattern: cli.filename_pattern,
        },
    )?;
    let ssh_args = strip_separator(cli.ssh_args);
    let invocation = SshInvocation::parse(ssh_args)?;
    terminal::run_terminal_session(config, invocation)
}

fn strip_separator(mut args: Vec<String>) -> Vec<String> {
    if args.first().map(|arg| arg == "--").unwrap_or(false) {
        args.remove(0);
    }
    args
}

fn default_config_path() -> Option<PathBuf> {
    env::var_os("APPDATA").map(|app_data| PathBuf::from(app_data).join("ipssh").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_path_uses_ipssh_app_directory() {
        let path = default_config_path().unwrap();
        assert_eq!(path.file_name().unwrap(), "config.toml");
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "ipssh");
    }
}
