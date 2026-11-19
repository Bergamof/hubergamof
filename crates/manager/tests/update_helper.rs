//! The real `hubergamof-update-helper` binary, driven the way systemd does:
//! the connection is its standard input and output. Every case here is refused
//! before any package manager would run (the signature is checked first), so
//! nothing is installed.
#![cfg(test)]

use std::io::{Read as _, Write as _};
use std::net::Shutdown;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};

use hubergamof_manager::update::wire::{Bundle, MAX_CHECKSUMS_BYTES, Response, encode_bundle};

const HELPER: &str = env!("CARGO_BIN_EXE_hubergamof-update-helper");

/// Runs the helper on a socket, sends `bytes`, and returns its exit code and
/// the line it answered.
fn talk(bytes: &[u8]) -> (Option<i32>, String) {
    let (mut client, server) = UnixStream::pair().unwrap();
    let server: OwnedFd = server.into();
    let mut child = Command::new(HELPER)
        .stdin(Stdio::from(server.try_clone().unwrap()))
        .stdout(Stdio::from(server))
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // The helper may refuse and exit before everything is sent.
    let _ = client.write_all(bytes);
    let _ = client.shutdown(Shutdown::Write);
    let mut answer = String::new();
    client.read_to_string(&mut answer).unwrap();
    let status = child.wait().unwrap();
    (status.code(), answer)
}

fn bundle_with(signature: Vec<u8>) -> Vec<u8> {
    encode_bundle(&Bundle {
        checksums: format!("{}  hubergamof_999.0.0_amd64.deb\n", "a".repeat(64)).into_bytes(),
        signature,
        package: b"evil".to_vec(),
    })
}

#[test]
fn checksums_that_were_not_signed_by_the_release_key_are_refused() {
    let (code, answer) = talk(&bundle_with(vec![7; 64]));
    assert_eq!(code, Some(1));
    assert!(answer.starts_with("error "), "{answer}");
    assert!(answer.contains("signature does not match"), "{answer}");
    assert_eq!(answer.matches('\n').count(), 1, "one line only: {answer}");
    assert!(matches!(
        Response::parse(answer.as_bytes()),
        Response::Failed(_)
    ));
}

#[test]
fn a_malformed_signature_is_refused() {
    let (code, answer) = talk(&bundle_with(vec![7; 10]));
    assert_eq!(code, Some(1));
    assert!(answer.contains("malformed"), "{answer}");
}

#[test]
fn an_empty_connection_is_refused() {
    let (code, answer) = talk(&[]);
    assert_eq!(code, Some(1));
    assert!(
        answer.starts_with("error cannot read the update"),
        "{answer}"
    );
}

#[test]
fn a_truncated_bundle_is_refused() {
    let mut bytes = bundle_with(vec![7; 64]);
    bytes.truncate(bytes.len() - 2);
    let (code, answer) = talk(&bytes);
    assert_eq!(code, Some(1));
    assert!(
        answer.starts_with("error cannot read the update"),
        "{answer}"
    );
}

#[test]
fn an_oversized_frame_is_refused_without_being_read() {
    let announced = u32::try_from(MAX_CHECKSUMS_BYTES + 1).unwrap();
    let (code, answer) = talk(&announced.to_be_bytes());
    assert_eq!(code, Some(1));
    assert!(
        answer.contains("checksums frame is larger than allowed"),
        "{answer}"
    );
}

#[test]
fn nothing_but_frames_is_ever_taken_as_a_path() {
    // Whatever a caller writes, the helper opens no file of its choosing: a
    // path as the first frame is only (unsigned) checksums.
    let (code, answer) = talk(&encode_bundle(&Bundle {
        checksums: b"/etc/shadow".to_vec(),
        signature: vec![0; 64],
        package: Vec::new(),
    }));
    assert_eq!(code, Some(1));
    assert!(answer.contains("signature"), "{answer}");
}
