use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn prints_help_for_ipssh() {
    let mut cmd = Command::cargo_bin("ipssh").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("OpenSSH wrapper"));
}

#[test]
fn errors_when_target_is_missing() {
    let mut cmd = Command::cargo_bin("ipssh").unwrap();
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("missing SSH target"));
}
