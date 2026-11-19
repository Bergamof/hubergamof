//! The real `hubergamof-update-helper` binary, driven the way `pkexec` does:
//! one argument, a download folder. Every case here is refused before any
//! package manager would run (the signature is checked first), so nothing is
//! installed.
#![cfg(test)]

use std::fs;
use std::process::{Command, Output};

const HELPER: &str = env!("CARGO_BIN_EXE_hubergamof-update-helper");

fn run(arguments: &[&std::ffi::OsStr]) -> Output {
    Command::new(HELPER).args(arguments).output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn it_wants_exactly_one_folder() {
    for arguments in [vec![], vec!["a".as_ref(), "b".as_ref()]] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(2));
        assert!(stderr(&output).contains("usage"), "{}", stderr(&output));
    }
}

#[test]
fn an_empty_download_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(&[dir.path().as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("SHA256SUMS"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn checksums_that_were_not_signed_by_the_release_key_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("SHA256SUMS"),
        format!("{}  hubergamof_999.0.0_amd64.deb\n", "a".repeat(64)),
    )
    .unwrap();
    fs::write(dir.path().join("SHA256SUMS.sig"), [7_u8; 64]).unwrap();
    fs::write(dir.path().join("hubergamof_999.0.0_amd64.deb"), b"evil").unwrap();
    let output = run(&[dir.path().as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("signature does not match"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_missing_folder_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(&[dir.path().join("absent").as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("cannot read SHA256SUMS"),
        "{}",
        stderr(&output)
    );
}
