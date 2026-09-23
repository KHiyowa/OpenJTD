use std::fs;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SAMPLE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn tiny_cfb_path() -> PathBuf {
    let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    compound
        .create_stream("/\u{4}JSRV_SegmentInformation")
        .unwrap()
        .write_all(b"segment")
        .unwrap();
    compound
        .create_stream("/DocInfo")
        .unwrap()
        .write_all(b"doc")
        .unwrap();
    compound.create_storage("/BodyText").unwrap();
    compound
        .create_stream("/BodyText/Section0")
        .unwrap()
        .write_all(b"hello")
        .unwrap();
    compound
        .create_stream("/DocumentText")
        .unwrap()
        .write_all(&document_text_fixture())
        .unwrap();

    write_sample(compound.into_inner().into_inner())
}

pub(crate) fn compressed_jttc_path() -> PathBuf {
    let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    compound
        .create_stream("/JSCompDocument")
        .unwrap()
        .write_all(b"\x26\0JustCompressedDocument\0-lh5-\0payload")
        .unwrap();

    write_sample(compound.into_inner().into_inner())
}

pub(crate) fn embedded_document_text_path() -> PathBuf {
    let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    let mut embedded = b"prefix SsmgV.01".to_vec();
    embedded.extend_from_slice(&[0x00, 0x1f]);
    for unit in "Note".encode_utf16() {
        embedded.extend_from_slice(&unit.to_be_bytes());
    }
    compound
        .create_stream("/JSSlipObject1")
        .unwrap()
        .write_all(&embedded)
        .unwrap();

    write_sample(compound.into_inner().into_inner())
}

pub(crate) fn write_sample(bytes: Vec<u8>) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "rjtd-streams-{}-{nonce}-{counter}.jtd",
        std::process::id()
    ));
    fs::write(&path, bytes).unwrap();
    path
}

pub(crate) fn document_text_fixture() -> Vec<u8> {
    let mut bytes = b"SsmgV.01".to_vec();
    bytes.extend_from_slice(&[0x00, 0x1f]);
    for unit in "銀河".encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    bytes.extend_from_slice(&[0x00, 0x1c]);
    bytes.extend_from_slice(&[0x00, 0x1f]);
    for unit in "鉄道\n".encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    bytes
}

pub(crate) fn document_text_from_str(text: &str) -> Vec<u8> {
    let mut bytes = b"SsmgV.01".to_vec();
    bytes.extend_from_slice(&[0x00, 0x1f]);
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    bytes
}
