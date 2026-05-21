use crate::remote_path::RemoteImagePath;
use crate::ssh_args::SshInvocation;
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

pub trait CommandRunner {
    fn run(&mut self, program: &str, args: &[String]) -> Result<CommandResult>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub success: bool,
    pub code: Option<i32>,
    pub stderr: String,
}

pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&mut self, program: &str, args: &[String]) -> Result<CommandResult> {
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("failed to run {program}"))?;
        Ok(CommandResult {
            success: output.status.success(),
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

pub struct Uploader<R> {
    pub runner: R,
}

impl<R: CommandRunner> Uploader<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn upload(
        &mut self,
        invocation: &SshInvocation,
        local_file: &Path,
        remote: &RemoteImagePath,
    ) -> Result<()> {
        let mut mkdir_args = invocation.ssh_args.clone();
        mkdir_args.push("mkdir".to_string());
        mkdir_args.push("-p".to_string());
        mkdir_args.push(remote.remote_dir.clone());
        let mkdir = self.runner.run("ssh", &mkdir_args)?;
        if !mkdir.success {
            bail!(
                "remote directory creation failed with code {:?}: {}",
                mkdir.code,
                mkdir.stderr.trim()
            );
        }

        let mut scp_args = invocation.scp_args()?;
        scp_args.push(local_file.display().to_string());
        scp_args.push(format!("{}:{}", invocation.target, remote.remote_path));
        let scp = self.runner.run("scp", &scp_args)?;
        if !scp.success {
            bail!(
                "scp upload failed with code {:?}: {}",
                scp.code,
                scp.stderr.trim()
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[derive(Default)]
    struct FakeRunner {
        calls: Vec<(String, Vec<String>)>,
    }

    impl CommandRunner for FakeRunner {
        fn run(&mut self, program: &str, args: &[String]) -> Result<CommandResult> {
            self.calls.push((program.to_string(), args.to_vec()));
            Ok(CommandResult {
                success: true,
                code: Some(0),
                stderr: String::new(),
            })
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn runs_mkdir_before_scp() {
        let invocation =
            SshInvocation::parse(strings(&["-p", "2222", "user@example.com"])).unwrap();
        let remote = RemoteImagePath {
            remote_dir: "~/uploads".to_string(),
            filename: "a.png".to_string(),
            remote_path: "~/uploads/a.png".to_string(),
        };
        let mut uploader = Uploader::new(FakeRunner::default());

        uploader
            .upload(&invocation, &PathBuf::from("C:/tmp/a.png"), &remote)
            .unwrap();

        assert_eq!(uploader.runner.calls[0].0, "ssh");
        assert!(uploader.runner.calls[0].1.ends_with(&strings(&[
            "user@example.com",
            "mkdir",
            "-p",
            "~/uploads",
        ])));
        assert_eq!(uploader.runner.calls[1].0, "scp");
        assert_eq!(
            uploader.runner.calls[1].1,
            strings(&[
                "-P",
                "2222",
                "C:/tmp/a.png",
                "user@example.com:~/uploads/a.png",
            ])
        );
    }
}
