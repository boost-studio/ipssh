use crate::remote_path::RemoteImagePath;

pub fn render_template(template: &str, path: &RemoteImagePath) -> String {
    template
        .replace("{remote_path}", &path.remote_path)
        .replace("{remote_dir}", &path.remote_dir)
        .replace("{filename}", &path.filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_default_template() {
        let path = RemoteImagePath {
            remote_dir: "~/uploads".to_string(),
            filename: "a.png".to_string(),
            remote_path: "~/uploads/a.png".to_string(),
        };
        assert_eq!(render_template("{remote_path}", &path), "~/uploads/a.png");
    }

    #[test]
    fn renders_markdown_template() {
        let path = RemoteImagePath {
            remote_dir: "~/uploads".to_string(),
            filename: "a.png".to_string(),
            remote_path: "~/uploads/a.png".to_string(),
        };
        assert_eq!(
            render_template("![image]({remote_path})", &path),
            "![image](~/uploads/a.png)"
        );
    }
}
