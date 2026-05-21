use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NonImagePaste {
    Text,
    PassThrough,
    Ignore,
}

impl NonImagePaste {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "text" => Ok(Self::Text),
            "pass_through" => Ok(Self::PassThrough),
            "ignore" => Ok(Self::Ignore),
            other => bail!("invalid non_image_paste value: {other}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub paste_hotkey: String,
    pub remote_dir: String,
    pub image_format: String,
    pub non_image_paste: NonImagePaste,
    pub template: String,
    pub filename_pattern: String,
}

#[derive(Debug, Default, Clone)]
pub struct ConfigOverrides {
    pub paste_hotkey: Option<String>,
    pub remote_dir: Option<String>,
    pub image_format: Option<String>,
    pub non_image_paste: Option<String>,
    pub template: Option<String>,
    pub filename_pattern: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    paste_hotkey: Option<String>,
    remote_dir: Option<String>,
    image_format: Option<String>,
    non_image_paste: Option<String>,
    template: Option<String>,
    upload: Option<RawUploadConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct RawUploadConfig {
    filename_pattern: Option<String>,
}

impl AppConfig {
    pub fn load(path: Option<&Path>, overrides: ConfigOverrides) -> Result<Self> {
        let raw = match path {
            Some(path) if path.exists() => {
                let text = fs::read_to_string(path)
                    .with_context(|| format!("failed to read config file {}", path.display()))?;
                toml::from_str::<RawConfig>(&text)
                    .with_context(|| format!("failed to parse config file {}", path.display()))?
            }
            _ => RawConfig::default(),
        };

        let paste_hotkey = overrides
            .paste_hotkey
            .or(raw.paste_hotkey)
            .unwrap_or_else(|| "ctrl+v".to_string());
        let remote_dir = overrides
            .remote_dir
            .or(raw.remote_dir)
            .unwrap_or_else(|| "~/Pictures/paste-ssh".to_string());
        let image_format = overrides
            .image_format
            .or(raw.image_format)
            .unwrap_or_else(|| "png".to_string());
        let non_image_value = overrides
            .non_image_paste
            .or(raw.non_image_paste)
            .unwrap_or_else(|| "text".to_string());
        let template = overrides
            .template
            .or(raw.template)
            .unwrap_or_else(|| "{remote_path}".to_string());
        let filename_pattern = overrides
            .filename_pattern
            .or(raw.upload.and_then(|upload| upload.filename_pattern))
            .unwrap_or_else(|| "{timestamp}-{random}.{ext}".to_string());

        if image_format != "png" {
            bail!("unsupported image_format: {image_format}; supported value is png");
        }

        Ok(Self {
            paste_hotkey,
            remote_dir,
            image_format,
            non_image_paste: NonImagePaste::parse(&non_image_value)?,
            template,
            filename_pattern,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn defaults_match_design_spec() {
        let cfg = AppConfig::load(None, ConfigOverrides::default()).unwrap();
        assert_eq!(cfg.paste_hotkey, "ctrl+v");
        assert_eq!(cfg.remote_dir, "~/Pictures/paste-ssh");
        assert_eq!(cfg.image_format, "png");
        assert_eq!(cfg.non_image_paste, NonImagePaste::Text);
        assert_eq!(cfg.template, "{remote_path}");
        assert_eq!(cfg.filename_pattern, "{timestamp}-{random}.{ext}");
    }

    #[test]
    fn config_file_overrides_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
paste_hotkey = "ctrl+shift+v"
remote_dir = "~/uploads"
image_format = "png"
non_image_paste = "ignore"
template = "![image]({remote_path})"

[upload]
filename_pattern = "{random}.{ext}"
"#,
        )
        .unwrap();

        let cfg = AppConfig::load(Some(&path), ConfigOverrides::default()).unwrap();
        assert_eq!(cfg.paste_hotkey, "ctrl+shift+v");
        assert_eq!(cfg.remote_dir, "~/uploads");
        assert_eq!(cfg.non_image_paste, NonImagePaste::Ignore);
        assert_eq!(cfg.template, "![image]({remote_path})");
        assert_eq!(cfg.filename_pattern, "{random}.{ext}");
    }

    #[test]
    fn cli_overrides_config_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "remote_dir = \"~/from-file\"").unwrap();

        let overrides = ConfigOverrides {
            remote_dir: Some("~/from-cli".to_string()),
            ..ConfigOverrides::default()
        };
        let cfg = AppConfig::load(Some(&path), overrides).unwrap();
        assert_eq!(cfg.remote_dir, "~/from-cli");
    }
}
