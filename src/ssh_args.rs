use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshInvocation {
    pub ssh_args: Vec<String>,
    pub target: String,
}

impl SshInvocation {
    pub fn parse(args: Vec<String>) -> Result<Self> {
        if args.is_empty() {
            bail!("missing SSH target");
        }

        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            if arg == "--" {
                index += 1;
                continue;
            }
            if is_option_with_value(arg) {
                index += 2;
                continue;
            }
            if is_attached_option_with_value(arg) {
                index += 1;
                continue;
            }
            if arg.starts_with('-') {
                index += 1;
                continue;
            }
            let target = arg.clone();
            return Ok(Self {
                ssh_args: args,
                target,
            });
        }

        bail!("missing SSH target")
    }

    pub fn scp_args(&self) -> Result<Vec<String>> {
        let mut output = Vec::new();
        let mut index = 0;

        while index < self.ssh_args.len() {
            let arg = &self.ssh_args[index];
            if arg == &self.target {
                index += 1;
                continue;
            }
            match arg.as_str() {
                "-p" => {
                    let value = self
                        .ssh_args
                        .get(index + 1)
                        .ok_or_else(|| anyhow::anyhow!("missing value for -p"))?;
                    output.push("-P".to_string());
                    output.push(value.clone());
                    index += 2;
                }
                "-i" | "-F" | "-J" | "-o" => {
                    let value = self
                        .ssh_args
                        .get(index + 1)
                        .ok_or_else(|| anyhow::anyhow!("missing value for {arg}"))?;
                    output.push(arg.clone());
                    output.push(value.clone());
                    index += 2;
                }
                "--" => index += 1,
                value if value.starts_with("-o") && value.len() > 2 => {
                    output.push(value.to_string());
                    index += 1;
                }
                value if value.starts_with("-p") && value.len() > 2 => {
                    output.push("-P".to_string());
                    output.push(value[2..].to_string());
                    index += 1;
                }
                value if value.starts_with('-') => {
                    output.push(value.to_string());
                    index += 1;
                }
                _ => index += 1,
            }
        }

        Ok(output)
    }
}

fn is_option_with_value(arg: &str) -> bool {
    matches!(arg, "-p" | "-i" | "-F" | "-J" | "-o")
}

fn is_attached_option_with_value(arg: &str) -> bool {
    (arg.starts_with("-o") && arg.len() > 2) || (arg.starts_with("-p") && arg.len() > 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_simple_target() {
        let invocation = SshInvocation::parse(strings(&["user@example.com"])).unwrap();
        assert_eq!(invocation.target, "user@example.com");
    }

    #[test]
    fn converts_ssh_port_to_scp_port() {
        let invocation =
            SshInvocation::parse(strings(&["-p", "2222", "user@example.com"])).unwrap();
        assert_eq!(invocation.scp_args().unwrap(), strings(&["-P", "2222"]));
    }

    #[test]
    fn converts_attached_ssh_port_to_scp_port() {
        let invocation = SshInvocation::parse(strings(&["-p2222", "user@example.com"])).unwrap();
        assert_eq!(invocation.scp_args().unwrap(), strings(&["-P", "2222"]));
    }

    #[test]
    fn preserves_common_options() {
        let invocation = SshInvocation::parse(strings(&[
            "-i",
            "key.pem",
            "-F",
            "ssh_config",
            "-J",
            "jump",
            "-o",
            "StrictHostKeyChecking=no",
            "my-host",
        ]))
        .unwrap();
        assert_eq!(
            invocation.scp_args().unwrap(),
            strings(&[
                "-i",
                "key.pem",
                "-F",
                "ssh_config",
                "-J",
                "jump",
                "-o",
                "StrictHostKeyChecking=no"
            ])
        );
    }

    #[test]
    fn reports_missing_target() {
        let error = SshInvocation::parse(strings(&["-p", "2222"])).unwrap_err();
        assert!(error.to_string().contains("missing SSH target"));
    }
}
