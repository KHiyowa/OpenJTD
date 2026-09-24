use rjtd_core::document_text::trim_trailing_exposed_controls;
use rjtd_model::{Block, Document, Inline};

pub fn to_plain_text(document: &Document) -> String {
    let output = if document.sheets().len() > 1 {
        document.plain_text()
    } else {
        let mut output = String::new();

        for block in document.blocks() {
            if let Block::Paragraph(paragraph) = block {
                for inline in paragraph.inlines() {
                    push_inline_visible_text(&mut output, inline);
                }
                output.push('\n');
            }
        }

        output
    };

    // P-H＋F: cat と同一規則でヘッダ・フッタ本文を先頭に前置き（Tika 準拠）。
    let output = match document.header_text() {
        Some(header) => format!("{header}\n\n{output}"),
        None => output,
    };
    trim_trailing_exposed_controls(&output).to_string()
}

fn push_inline_visible_text(output: &mut String, inline: &Inline) {
    match inline {
        Inline::Text(text) => output.push_str(text.text()),
        Inline::Ruby(ruby) => output.push_str(ruby.base_text()),
        Inline::Unknown(_) => {}
    }
}
