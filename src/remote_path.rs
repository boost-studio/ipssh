use chrono::Local;
use rand::{distributions::Alphanumeric, Rng};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteImagePath {
    pub remote_dir: String,
    pub filename: String,
    pub remote_path: String,
}

pub fn generate_remote_image_path(remote_dir: &str, pattern: &str, ext: &str) -> RemoteImagePath {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let random: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(6)
        .map(char::from)
        .collect();
    let filename = pattern
        .replace("{timestamp}", &timestamp)
        .replace("{random}", &random)
        .replace("{ext}", ext);
    let remote_path = format!("{}/{}", remote_dir.trim_end_matches('/'), filename);

    RemoteImagePath {
        remote_dir: remote_dir.to_string(),
        filename,
        remote_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_path_without_double_slash() {
        let path = generate_remote_image_path("~/uploads/", "{random}.{ext}", "png");
        assert!(path.remote_path.starts_with("~/uploads/"));
        assert!(path.remote_path.ends_with(".png"));
        assert!(!path.remote_path.contains("//"));
    }
}
