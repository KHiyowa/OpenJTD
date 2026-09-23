use std::fs;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::process::Command;

use super::support::*;

fn tiny_cfb_with_footnote_path() -> PathBuf {
    let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    compound
        .create_stream("/\u{4}JSRV_SegmentInformation")
        .unwrap()
        .write_all(b"segment")
        .unwrap();
    compound
        .create_stream("/DocumentText")
        .unwrap()
        .write_all(&document_text_from_str(
            "それはこんやの星祭に青いあかりをこしらえて川へ流す烏瓜を取りに行く相談らしかったのです。[1]\n",
        ))
        .unwrap();
    compound
        .create_stream("/Footnote")
        .unwrap()
        .write_all(&document_text_from_str(
            "[1] 烏瓜（からすうり） ウリ科の植物。星祭で青いあかりを灯して川に流す。\n",
        ))
        .unwrap();

    write_sample(compound.into_inner().into_inner())
}

#[test]
fn export_command_outputs_footnote_references() {
    let path = tiny_cfb_with_footnote_path();
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
    assert!(stdout.contains("烏瓜を取りに行く相談らしかったのです。[1]"));
    assert!(stdout.contains("[1] 烏瓜（からすうり） ウリ科の植物。"));
}

// Footnote stream that ends with the internal field/template marker `0x001d Note 0x001e`
// (surfacing as an exposed "Note" after the last reference). Unit tests embed no thesis text;
// using Aozora Bunko "Night on the Galactic Railroad" text instead (Miyazawa Kenji).
fn tiny_cfb_with_footnote_trailing_note_path() -> PathBuf {
    let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    compound
        .create_stream("/\u{4}JSRV_SegmentInformation")
        .unwrap()
        .write_all(b"segment")
        .unwrap();
    compound
        .create_stream("/DocumentText")
        .unwrap()
        .write_all(&document_text_from_str(
            "「ではみなさんは、そういうふうに川だと云われたり、乳の流れたあとだと云われたりしていたこのぼんやりと白いものがほんとうは何かご承知ですか。」[1]\n",
        ))
        .unwrap();

    let mut footnote_bytes = document_text_from_str(
        "[1] ケンタウル祭 銀河のお祭り。星祭ともいう。\n",
    );
    // Append the internal sentinel field for Note as observed in Ichitaro Footnote streams:
    // [0x001c, 0x0001, 0x0007, 0x0000, 0x0000, 0x0001], 0x001d, "Note", 0x001e, 0x0005, 0x0000, 0x0001, 0x001f
    for unit in [0x001cu16, 0x0001, 0x0007, 0x0000, 0x0000, 0x0001, 0x001d] {
        footnote_bytes.extend_from_slice(&unit.to_be_bytes());
    }
    for unit in "Note".encode_utf16() {
        footnote_bytes.extend_from_slice(&unit.to_be_bytes());
    }
    for unit in [0x001eu16, 0x0005, 0x0000, 0x0001, 0x001f] {
        footnote_bytes.extend_from_slice(&unit.to_be_bytes());
    }

    compound
        .create_stream("/Footnote")
        .unwrap()
        .write_all(&footnote_bytes)
        .unwrap();

    write_sample(compound.into_inner().into_inner())
}

#[test]
fn export_command_trims_footnote_trailing_sentinel_note() {
    let path = tiny_cfb_with_footnote_trailing_note_path();
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
        stdout.contains("[1] ケンタウル祭 銀河のお祭り。星祭ともいう。"),
        "expected footnote text, got: {stdout:?}"
    );
    assert!(
        !stdout.contains("Note"),
        "trailing sentinel 'Note' should not be exposed in footnote export, got: {stdout:?}"
    );
}

// Note numbering decoration is document-configured in Ichitaro, so labels are not
// always "[1]". The Footnote stream decodes them structurally (anchor selector
// 0x0001 followed by the body TextRun), so labels like (1) must also start a
// new note line instead of being glued to the previous note.
fn footnote_stream_with_entries(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut bytes = b"SsmgV.01".to_vec();
    for (label, body) in entries {
        for unit in [0x001cu16, 0x0001, 0x0007, 0x0000, 0x0000, 0x0001, 0x001d] {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        for unit in label.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        for unit in [0x001eu16, 0x0005, 0x0000, 0x0001, 0x001f] {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        for unit in body.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
    }
    bytes
}

fn tiny_cfb_with_decorated_footnote_path() -> PathBuf {
    let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    compound
        .create_stream("/\u{4}JSRV_SegmentInformation")
        .unwrap()
        .write_all(b"segment")
        .unwrap();
    compound
        .create_stream("/DocumentText")
        .unwrap()
        .write_all(&document_text_from_str("星祭の夜。\n"))
        .unwrap();
    compound
        .create_stream("/Footnote")
        .unwrap()
        .write_all(&footnote_stream_with_entries(&[
            ("[1]", "烏瓜（からすうり） ウリ科の植物。\n"),
            ("(2)", "星祭 銀河のお祭り。\n"),
            ("1)", "川の光は青く光る。\n"),
        ]))
        .unwrap();

    write_sample(compound.into_inner().into_inner())
}

#[test]
fn export_command_respects_non_square_bracket_footnote_decorations() {
    let path = tiny_cfb_with_decorated_footnote_path();
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
        stdout.contains("[1] 烏瓜（からすうり） ウリ科の植物。\n(2) 星祭 銀河のお祭り。\n1) 川の光は青く光る。"),
        "expected each decorated label to start its own note line, got: {stdout:?}"
    );
}
