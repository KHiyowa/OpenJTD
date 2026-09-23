use std::fs;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::process::Command;

use super::support::*;

#[test]
fn export_command_writes_text_from_document_model() {
    let path = tiny_cfb_path();
    let output = Command::new(env!("CARGO_BIN_EXE_rjtd"))
        .arg("export")
        .arg(&path)
        .arg("--format")
        .arg("text")
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
fn export_command_accepts_txt_alias() {
    let path = tiny_cfb_path();
    let output = Command::new(env!("CARGO_BIN_EXE_rjtd"))
        .arg("export")
        .arg(&path)
        .arg("--format")
        .arg("txt")
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
fn export_command_rejects_unknown_format() {
    let path = tiny_cfb_path();
    let output = Command::new(env!("CARGO_BIN_EXE_rjtd"))
        .arg("export")
        .arg(&path)
        .arg("--format")
        .arg("docx")
        .output()
        .unwrap();

    fs::remove_file(&path).unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unsupported export format: docx"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// DocumentText whose body ends with the internal terminal control units 0xFE14 and 0x0490
// (the `︔`/`Г` pair surfaced as exposed characters at the very end of the stream). Unit
// tests embed no thesis text; the body is an Aozora Bunko "Night on the Galactic Railroad" line.
fn tiny_cfb_with_trailing_controls_path() -> PathBuf {
    // Miyazawa Kenji, Night on the Galactic Railroad (Aozora Bunko 456_15050).
    let body = "「大きな望遠鏡で銀河をよっく調べると銀河は大体何でしょう。」";
    let mut bytes = b"SsmgV.01".to_vec();
    bytes.extend_from_slice(&[0x00, 0x1f]);
    for unit in body.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    // Trailing internal terminal markers as they appear in real stream tails.
    bytes.extend_from_slice(&0xFE14u16.to_be_bytes());
    bytes.extend_from_slice(&0x0490u16.to_be_bytes());

    let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    compound
        .create_stream("/\u{4}JSRV_SegmentInformation")
        .unwrap()
        .write_all(b"segment")
        .unwrap();
    compound
        .create_stream("/DocumentText")
        .unwrap()
        .write_all(&bytes)
        .unwrap();

    write_sample(compound.into_inner().into_inner())
}

#[test]
fn export_command_trims_trailing_exposed_terminal_controls() {
    let path = tiny_cfb_with_trailing_controls_path();
    let output = Command::new(env!("CARGO_BIN_EXE_rjtd"))
        .arg("export")
        .arg(&path)
        .arg("-f")
        .arg("text")
        .output()
        .unwrap();

    fs::remove_file(&path).unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("大きな望遠鏡で銀河をよっく調べると銀河は大体何でしょう。"),
        "expected clean body text, got: {stdout:?}"
    );
    assert!(
        !stdout.contains('\u{0490}') && !stdout.contains('\u{FE14}'),
        "trailing terminal controls (U+0490 / U+FE14) should be trimmed, got: {stdout:?}"
    );
}

fn tiny_cfb_with_trailing_isolated_text_marker_leak_path() -> PathBuf {
    let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    compound
        .create_stream("/\u{4}JSRV_SegmentInformation")
        .unwrap()
        .write_all(b"segment")
        .unwrap();

    let mut doc_text_bytes = document_text_from_str(
        "「ではみなさんは、そういうふうに川だと云われたり、乳の流れたあとだと云われたりしていたこのぼんやりと白いものがほんとうは何かご承知ですか。」\n",
    );
    // Simulate end of prose followed by non-record binary padding, then an isolated 0x001f marker
    // and trailing metadata record codes as observed in 計画交通課パブコメ2025.jtd (unit 76819-76825):
    // 0x000e, 0x0000, padding, 0x001f, 0xfe01 (︁), 0x0200 (Ȁ), 0x0302 (̂), 0x0200 (Ȁ), 0x03ff (Ͽ), 0x0000
    for unit in [0x000eu16, 0x0000, 0x1234, 0x5678, 0x001f, 0xfe01, 0x0200, 0x0302, 0x0200, 0x03ff, 0x0000] {
        doc_text_bytes.extend_from_slice(&unit.to_be_bytes());
    }

    compound
        .create_stream("/DocumentText")
        .unwrap()
        .write_all(&doc_text_bytes)
        .unwrap();

    write_sample(compound.into_inner().into_inner())
}

#[test]
fn export_command_suppresses_trailing_isolated_marker_binary_leak() {
    let path = tiny_cfb_with_trailing_isolated_text_marker_leak_path();
    let output = Command::new(env!("CARGO_BIN_EXE_rjtd"))
        .arg("export")
        .arg(&path)
        .arg("-f")
        .arg("text")
        .arg("-s")
        .arg("0")
        .output()
        .unwrap();

    fs::remove_file(&path).unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("「ではみなさんは、そういうふうに川だと云われたり"),
        "expected body prose, got: {stdout:?}"
    );
    assert!(
        !stdout.contains('\u{FE01}')
            && !stdout.contains('\u{0200}')
            && !stdout.contains('\u{0302}')
            && !stdout.contains('\u{03FF}'),
        "trailing binary leak (︁Ȁ̂ȀϿ) from isolated marker should be structurally suppressed from plain text export, got: {stdout:?}"
    );
}
