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
