use super::*;
use rjtd_core::record::UnknownRecordKind;
use rjtd_model::{
    Block, Document, Inline, Metadata, Paragraph, RubyAnnotation, TextRun, UnknownObject,
};

#[test]
fn exports_ruby_inline_as_visible_base_text() {
    let paragraph = Paragraph::new(
        vec![
            Inline::Text(TextRun::new("一、", None)),
            Inline::Ruby(RubyAnnotation::new(
                "午后",
                "ごご",
                0x0082,
                UnknownObject::new(UnknownRecordKind::new(Some(0x001d)), vec![1]),
            )),
            Inline::Text(TextRun::new("の授業", None)),
        ],
        None,
    );
    let document = Document::new(Metadata::default(), vec![Block::Paragraph(paragraph)]);

    assert_eq!(to_plain_text(&document), "一、午后の授業\n");
}

// P-H＋F（PARTIAL-LOSS-REPORT.md 付録2）: cat と同一規則で、保持された
// /Header（ヘッダ・フッタ本文）を Tika 準拠で本文前（先頭行 + 空行）に前置きする。
#[test]
fn prepends_stored_header_text_before_body() {
    let document = Document::new(
        Metadata::default(),
        vec![Block::Paragraph(Paragraph::from_text("銀河鉄道"))],
    )
    .with_header_text("- 1 -");

    assert_eq!(to_plain_text(&document), "- 1 -\n\n銀河鉄道\n");
}

// 複数行のヘッダ・フッタ本文は改行連結のまま先頭に前置きされる。
#[test]
fn prepends_multi_line_header_text() {
    let document = Document::new(
        Metadata::default(),
        vec![Block::Paragraph(Paragraph::from_text("銀河鉄道"))],
    )
    .with_header_text("- 1 -\nVer.1.0");

    assert_eq!(to_plain_text(&document), "- 1 -\nVer.1.0\n\n銀河鉄道\n");
}

// ゲート: header_text 未保持（従来経路）は出力完全不変（既存テストと同じ期待値）。
#[test]
fn unchanged_without_header_text() {
    let document = Document::new(
        Metadata::default(),
        vec![Block::Paragraph(Paragraph::from_text("銀河鉄道"))],
    );

    assert_eq!(to_plain_text(&document), "銀河鉄道\n");
    assert_eq!(document.header_text(), None);
}
