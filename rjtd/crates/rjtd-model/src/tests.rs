//! rjtd-model の最小ユニットテスト。
//!
//! 旧 tests/ ディレクトリ（診断・レイアウト系テスト群）は縮約に伴い削除済み。
//! ここでは生存 API（`Document::plain_text` のシート連結規則、`read_footnote_text`
//! のラベル+本文ペアリング、`from_plain_text`）と /Header（ヘッダ・フッタ本文）の
//! parse 後の保持（P-H＋F）のみを網羅する。

use std::io::{Cursor, Write};

use crate::{Document, DocumentSheet, IchitaroParser, read_footnote_text};

/// 足注アンカーレコード:
/// `[0x001c,0x0001,0x0007,0x0000,0x0000,0x0001], 0x001d, <ラベル>, 0x001e,
///  0x0005, 0x0000, 0x0001, 0x001f, <本文>`
/// core の `parse_document_text` が InlineText(ラベル, selector 0x0001) と
/// 直後の TextRun(本文) に分解することを前提とする（cli の streams/footnote テスト
/// と同一のバイト列構造）。
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

#[test]
fn from_plain_text_round_trips_lines_into_paragraphs() {
    let doc = Document::from_plain_text("一行目\r\n二行目\n\n三行目\n");

    assert_eq!(doc.blocks().len(), 3);
    assert_eq!(doc.plain_text(), "一行目\n二行目\n三行目\n");
}

#[test]
fn plain_text_single_sheet_appends_trimmed_footnote() {
    let mut doc = Document::from_plain_text("正文です。\n");
    doc.push_sheet(
        DocumentSheet::new(0, "タイトル", "", None, "正文です。\n")
            .with_footnote_text("[1] 脚注本文  烏瓜"),
    );

    // 旧実装の規則: document_plain_text が段落ごとに文末 \n を付加し、
    // それに \n\n と trim 済み脚注が連結される（= 段落末尾の \n と脚注の間が3改行）。
    assert_eq!(doc.plain_text(), "正文です。\n\n\n[1] 脚注本文  烏瓜");
}

#[test]
fn plain_text_multiple_sheets_uses_bare_name_and_single_newline() {
    let mut doc = Document::from_plain_text("本体テキスト\n");
    doc.push_sheet(
        DocumentSheet::new(0, "シートA", "", None, "A本文\n").with_footnote_text("A脚注"),
    );
    doc.push_sheet(DocumentSheet::new(
        1,
        "シートB",
        "/ObjectSheets/DocSheet/DOCS_0000",
        None,
        "B本文\n",
    ));
    doc.push_sheet(
        DocumentSheet::new(
            2,
            "シートC",
            "/ObjectSheets/DocSheet/DOCS_0001",
            None,
            "C本文\n",
        )
        .with_footnote_text("C脚注"),
    );

    // 新規則（Tika Excel 準拠）: 各シートの先頭に装飾なしのシート名単独行を出す。
    // シート間は空行（\n\n）区切り、シート名行の直後だけは単一改行（\n）。
    // 脚注は従来どおり \n\n で連結する。
    assert_eq!(
        doc.plain_text(),
        "シートA\nA本文\n\nA脚注\n\nシートB\nB本文\n\nシートC\nC本文\n\nC脚注"
    );
    // `# ` プレフィックスは廃止: シート名行の行頭に `#` がつかないことを明確に検証する。
    let rendered = doc.plain_text();
    assert!(!rendered.starts_with("# "));
    for name in ["シートA", "シートB", "シートC"] {
        assert!(
            rendered.contains(&format!("\n{}\n", name))
                || rendered.starts_with(&format!("{}\n", name)),
            "bare sheet name line missing: {name}"
        );
    }
}

#[test]
fn read_footnote_text_pairs_each_anchor_label_with_its_body() {
    let bytes = footnote_stream_with_entries(&[
        ("[1]", "烏瓜（からすうり） ウリ科の植物。\n"),
        ("(2)", "星祭 銀河のお祭り。\n"),
        ("1)", "川の光は青く光る。\n"),
    ]);

    assert_eq!(
        read_footnote_text(&bytes),
        Some(
            "[1] 烏瓜（からすうり） ウリ科の植物。\n(2) 星祭 銀河のお祭り。\n1) 川の光は青く光る。"
                .to_owned()
        )
    );
}

#[test]
fn read_footnote_text_drops_trailing_sentinel_label_without_body() {
    // 末尾に本文を持たないアンカー（内部テンプレート用の sentinel ラベル `Note`）を付加する。
    let mut bytes = footnote_stream_with_entries(&[("[1]", "ケンタウル祭 銀河のお祭り。\n")]);
    for unit in [0x001cu16, 0x0001, 0x0007, 0x0000, 0x0000, 0x0001, 0x001d] {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    for unit in "Note".encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    for unit in [0x001eu16, 0x0005, 0x0000, 0x0001, 0x001f] {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }

    let text = read_footnote_text(&bytes).expect("footnote text present");
    assert_eq!(text, "[1] ケンタウル祭 銀河のお祭り。");
    assert!(!text.contains("Note"));
}

#[test]
fn read_footnote_text_returns_none_for_body_without_anchor() {
    let mut bytes = b"SsmgV.01".to_vec();
    bytes.extend_from_slice(&[0x00, 0x1f]);
    for unit in "アンカーなし本文".encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }

    // アンカーラベルが存在しない本文はスペース接頭のまま連結される（trim 後は非空）。
    assert_eq!(
        read_footnote_text(&bytes),
        Some("アンカーなし本文".to_owned())
    );
}

// ------------------------------------------------------------------
// P-H＋F（PARTIAL-LOSS-REPORT.md 付録2）: /Header（ヘッダ・フッタ本文）の保持
//
// core の header_text 合成フィクスチャ（tests/streams/header_text.rs と同構造）を
// 再利用する最小モック。本文は宮沢賢治「銀河鉄道の夜」（青空文庫・パブリック
// ドメイン）からの合成で、官公庁原本の文言は使用しない。

const HEADER_TEXT_SEGMENT_NAME: [u16; 4] = [0x5465, 0x7874, 0x562e, 0x3031]; // "TextV.01"
const HEADER_TEXT_SLOT_PITCH_WORDS: usize = 128;
const HEADER_TEXT_TAIL_TABLE: [u16; 8] = [
    0x001b, 0x0000, 0x0001, 0x0000, 0x0001, 0x0000, 0x005d, 0x00a9,
];

fn extend_words(bytes: &mut Vec<u8>, words: &[u16]) {
    for word in words {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
}

/// raw span（マーカー無し・全文 UTF-16BE 生）。
fn header_raw_span(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// `TextV.01` スロット: 名（4語）+ [0x0000, span 長]（2語）+ span 内容 + 0x0000 パディング。
fn header_textv_slot(span: &[u16]) -> Vec<u16> {
    let mut slot = Vec::new();
    slot.extend_from_slice(&HEADER_TEXT_SEGMENT_NAME);
    slot.extend_from_slice(&[0x0000, span.len() as u16]);
    slot.extend_from_slice(span);
    slot.resize(HEADER_TEXT_SLOT_PITCH_WORDS, 0x0000);
    slot
}

/// `SsmgV.01` ヘッダ + スロット列 + 末尾テーブル列（core フィクスチャと同型）。
fn header_payload(slots: &[Vec<u16>]) -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    extend_words(
        &mut bytes,
        &[
            0x5373,
            0x6d67,
            0x562e,
            0x3031, // SsmgV.01
            0x0000,
            0x0001,
            0x0000,
            0x0100,
            0x0000,
            slots.len() as u16,
        ],
    );
    for slot in slots {
        extend_words(&mut bytes, slot);
    }
    extend_words(&mut bytes, &HEADER_TEXT_TAIL_TABLE);
    bytes
}

fn cfb_with_streams(document_text: &[u8], header: Option<&[u8]>) -> Vec<u8> {
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
        .write_all(document_text)
        .unwrap();
    if let Some(header) = header {
        compound
            .create_stream("/Header")
            .unwrap()
            .write_all(header)
            .unwrap();
    }
    compound.into_inner().into_inner()
}

/// parse 後に /Header の復元本文が `Document::header_text` で保持される（cat /
/// export-txt 前置き用の core read_header_text 共用経路）。
#[test]
fn parse_keeps_header_text_from_cfb() {
    // /DocumentText: SsmgV.01 + 0x001f + run 本文（マーカー型）+ run 末尾。
    let mut document_text: Vec<u8> = Vec::new();
    extend_words(&mut document_text, &[0x5373, 0x6d67, 0x562e, 0x3031]);
    extend_words(&mut document_text, &[0x001f]);
    extend_words(&mut document_text, header_raw_span("銀河鉄道\n").as_slice());
    extend_words(&mut document_text, &[0x0000]);

    let header = header_payload(&[
        header_textv_slot(&header_raw_span("- ? -")),
        header_textv_slot(&header_raw_span("Ver.1.0")),
    ]);
    let bytes = cfb_with_streams(&document_text, Some(&header));
    let document = IchitaroParser
        .parse_with_budget(
            &bytes,
            &mut rjtd_core::ParseLimits::DEFAULT.resource_budget(),
        )
        .expect("parse should succeed");

    // raw span の `?` は F 裁定で先頭ページ "1" に解決され、ストリーム順に連結される。
    assert_eq!(
        document.header_text().map(str::to_owned),
        Some("- 1 -\nVer.1.0".to_owned())
    );
}

// ゲート: /Header 欠落は `header_text()` が None のまま（従来出力不変）。
#[test]
fn parse_keeps_header_text_none_when_stream_missing() {
    let mut document_text: Vec<u8> = Vec::new();
    extend_words(&mut document_text, &[0x5373, 0x6d67, 0x562e, 0x3031]);
    extend_words(&mut document_text, &[0x001f]);
    extend_words(&mut document_text, header_raw_span("銀河鉄道\n").as_slice());
    extend_words(&mut document_text, &[0x0000]);

    let bytes = cfb_with_streams(&document_text, None);
    let document = IchitaroParser
        .parse_with_budget(
            &bytes,
            &mut rjtd_core::ParseLimits::DEFAULT.resource_budget(),
        )
        .expect("parse should succeed");

    assert_eq!(document.header_text(), None);
}
