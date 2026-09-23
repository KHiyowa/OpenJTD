//! rjtd-model の最小ユニットテスト。
//!
//! 旧 tests/ ディレクトリ（診断・レイアウト系テスト群）は縮約に伴い削除済み。
//! ここでは生存 API（`Document::plain_text` のシート連結規則、`read_footnote_text`
//! のラベル+本文ペアリング、`from_plain_text`）のみを網羅する。

use crate::{Document, DocumentSheet, read_footnote_text};

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
fn plain_text_multiple_sheets_uses_hash_name_and_blank_line_separators() {
    let mut doc = Document::from_plain_text("本体テキスト\n");
    doc.push_sheet(
        DocumentSheet::new(0, "シートA", "", None, "A本文\n")
            .with_footnote_text("A脚注"),
    );
    doc.push_sheet(DocumentSheet::new(1, "シートB", "/ObjectSheets/DocSheet/DOCS_0000", None, "B本文\n"));
    doc.push_sheet(
        DocumentSheet::new(2, "シートC", "/ObjectSheets/DocSheet/DOCS_0001", None, "C本文\n")
            .with_footnote_text("C脚注"),
    );

    assert_eq!(
        doc.plain_text(),
        "# シートA\n\nA本文\n\nA脚注\n\n# シートB\n\nB本文\n\n# シートC\n\nC本文\n\nC脚注"
    );
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
        Some("[1] 烏瓜（からすうり） ウリ科の植物。\n(2) 星祭 銀河のお祭り。\n1) 川の光は青く光る。".to_owned())
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
    assert_eq!(read_footnote_text(&bytes), Some("アンカーなし本文".to_owned()));
}
