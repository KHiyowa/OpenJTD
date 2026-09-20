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
    trim_trailing_exposed_controls(&output).to_string()
}

pub fn to_markdown(document: &Document) -> String {
    let output = if document.sheets().len() > 1 {
        let mut output = String::new();
        for (i, sheet) in document.sheets().iter().enumerate() {
            if i > 0 {
                output.push_str("\n\n");
            }
            output.push_str(&format!("# {}\n\n", sheet.name()));
            output.push_str(sheet.text().trim());
        }
        output
    } else {
        let mut output = String::new();

        for block in document.blocks() {
            match block {
                Block::Paragraph(paragraph) => {
                    for inline in paragraph.inlines() {
                        push_inline_visible_text(&mut output, inline);
                    }
                    output.push_str("\n\n");
                }
                Block::Unknown(_) => {
                    output.push_str("<!-- UnknownBlock preserved by rjtd -->\n\n");
                }
            }
        }

        output
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
