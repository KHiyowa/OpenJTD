use std::fs;
use std::process::Command;

use super::support::*;

#[test]
fn cat_command_extracts_document_text_runs() {
    let path = tiny_cfb_path();
    let output = Command::new(env!("CARGO_BIN_EXE_rjtd"))
        .arg("cat")
        .arg(&path)
        .output()
        .unwrap();

    fs::remove_file(&path).unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "銀河鉄道\n");
}

#[test]
fn cat_command_reports_invalid_compressed_jttc_payload() {
    let path = compressed_jttc_path();
    let output = Command::new(env!("CARGO_BIN_EXE_rjtd"))
        .arg("cat")
        .arg(&path)
        .output()
        .unwrap();

    fs::remove_file(&path).unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("invalid data"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cat_command_extracts_embedded_document_text() {
    let path = embedded_document_text_path();
    let output = Command::new(env!("CARGO_BIN_EXE_rjtd"))
        .arg("cat")
        .arg(&path)
        .output()
        .unwrap();

    fs::remove_file(&path).unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "Note");
}
