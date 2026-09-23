use rjtd_core::document_text::{
    DocumentTextControl, InlineTextSegment, SkippedInlineTextSegment,
};
use rjtd_core::record::UnknownRecordKind;

// document_text/types.rs の縮約により、ルビ判定に使う selector をブロック内にローカル定義する。
pub(crate) const DOCUMENT_TEXT_RUBY_BASE_SELECTOR: u16 = 0x0003;

pub(crate) const DOCUMENT_TEXT_RUBY_TEXT_SELECTOR: u16 = 0x0082;

const DOCUMENT_TEXT_INLINE_START_TAG: u32 = 0x001d;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Paragraph),
    Unknown(UnknownBlock),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(TextRun),
    Ruby(RubyAnnotation),
    Unknown(UnknownObject),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRun {
    pub(crate) text: String,
    pub(crate) style: Option<StyleRef>,
}

impl TextRun {
    pub fn new(text: impl Into<String>, style: Option<StyleRef>) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn style(&self) -> Option<&StyleRef> {
        self.style.as_ref()
    }

    pub(crate) fn push_text(&mut self, text: &str) {
        self.text.push_str(text);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleRef {
    pub(crate) id: String,
}

impl StyleRef {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paragraph {
    pub(crate) inlines: Vec<Inline>,
    pub(crate) style: Option<StyleRef>,
}

impl Paragraph {
    pub fn new(inlines: Vec<Inline>, style: Option<StyleRef>) -> Self {
        Self { inlines, style }
    }

    pub fn from_text(text: impl Into<String>) -> Self {
        Self::new(vec![Inline::Text(TextRun::new(text, None))], None)
    }

    pub fn inlines(&self) -> &[Inline] {
        &self.inlines
    }

    pub fn style(&self) -> Option<&StyleRef> {
        self.style.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RubyAnnotation {
    pub(crate) base_text: String,
    pub(crate) annotation_text: String,
    pub(crate) annotation_selector: u16,
    pub(crate) annotation_source: UnknownObject,
}

impl RubyAnnotation {
    pub fn new(
        base_text: impl Into<String>,
        annotation_text: impl Into<String>,
        annotation_selector: u16,
        annotation_source: UnknownObject,
    ) -> Self {
        Self {
            base_text: base_text.into(),
            annotation_text: annotation_text.into(),
            annotation_selector,
            annotation_source,
        }
    }

    pub fn base_text(&self) -> &str {
        &self.base_text
    }

    pub fn annotation_text(&self) -> &str {
        &self.annotation_text
    }

    pub fn annotation_selector(&self) -> u16 {
        self.annotation_selector
    }

    pub fn annotation_source(&self) -> &UnknownObject {
        &self.annotation_source
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownBlock {
    pub(crate) source: UnknownRecordKind,
    pub(crate) payload: Vec<u8>,
}

impl UnknownBlock {
    pub fn new(source: UnknownRecordKind, payload: Vec<u8>) -> Self {
        Self { source, payload }
    }

    pub fn source(&self) -> &UnknownRecordKind {
        &self.source
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownObject {
    pub(crate) source: UnknownRecordKind,
    pub(crate) payload: Vec<u8>,
}

impl UnknownObject {
    pub fn new(source: UnknownRecordKind, payload: Vec<u8>) -> Self {
        Self { source, payload }
    }

    pub fn source(&self) -> &UnknownRecordKind {
        &self.source
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelTextSource {
    TextRun,
    Inline,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocumentTextModelBuilder {
    pub(crate) current_inlines: Vec<Inline>,
    pub(crate) blocks: Vec<Block>,
    pub(crate) can_merge_current_text_run: bool,
    pub(crate) pending_ruby_base_inline_index: Option<usize>,
}

impl DocumentTextModelBuilder {
    pub(crate) fn push_text_run(&mut self, text: &str) {
        self.pending_ruby_base_inline_index = None;
        self.push_text(text, ModelTextSource::TextRun);
    }

    pub(crate) fn push_inline_text(&mut self, segment: &InlineTextSegment) {
        self.pending_ruby_base_inline_index = None;
        let previous_block_count = self.blocks.len();
        let previous_inline_count = self.current_inlines.len();

        self.push_text(segment.text(), ModelTextSource::Inline);

        if segment.selector() == DOCUMENT_TEXT_RUBY_BASE_SELECTOR
            && previous_block_count == self.blocks.len()
            && self.current_inlines.len() == previous_inline_count + 1
        {
            self.pending_ruby_base_inline_index = Some(previous_inline_count);
        }
    }

    pub(crate) fn push_skipped_inline(&mut self, segment: &SkippedInlineTextSegment) {
        if self.promote_ruby_annotation(segment) {
            return;
        }

        self.pending_ruby_base_inline_index = None;
        self.can_merge_current_text_run = false;
    }

    pub(crate) fn push_control_boundary(&mut self, _control: &DocumentTextControl) {
        self.can_merge_current_text_run = false;
    }

    fn push_text(&mut self, text: &str, source: ModelTextSource) {
        for part in source_text_parts(text) {
            if !part.text.is_empty() {
                self.push_text_part(&part.text, source);
            }

            if part.break_after {
                self.flush_paragraph();
            }
        }
    }

    pub(crate) fn finish(mut self) -> Vec<Block> {
        self.flush_paragraph();
        self.blocks
    }

    pub(crate) fn flush_paragraph(&mut self) {
        if self.current_inlines.is_empty() {
            self.can_merge_current_text_run = false;
            self.pending_ruby_base_inline_index = None;
            return;
        }

        let inlines = std::mem::take(&mut self.current_inlines);
        self.blocks
            .push(Block::Paragraph(Paragraph::new(inlines, None)));
        self.can_merge_current_text_run = false;
        self.pending_ruby_base_inline_index = None;
    }

    fn push_text_part(&mut self, text: &str, source: ModelTextSource) {
        if source == ModelTextSource::TextRun
            && self.can_merge_current_text_run
            && let Some(Inline::Text(run)) = self.current_inlines.last_mut()
        {
            run.push_text(text);
            return;
        }

        self.current_inlines
            .push(Inline::Text(TextRun::new(text, None)));
        self.can_merge_current_text_run = source == ModelTextSource::TextRun;
    }

    fn promote_ruby_annotation(&mut self, segment: &SkippedInlineTextSegment) -> bool {
        if segment.selector() != Some(DOCUMENT_TEXT_RUBY_TEXT_SELECTOR) {
            return false;
        }

        let Some(index) = self.pending_ruby_base_inline_index.take() else {
            return false;
        };

        let Some(inline) = self.current_inlines.get_mut(index) else {
            return false;
        };

        let Inline::Text(base_run) = inline else {
            return false;
        };

        let base_text = std::mem::take(&mut base_run.text);
        let annotation = RubyAnnotation::new(
            base_text,
            segment.text(),
            DOCUMENT_TEXT_RUBY_TEXT_SELECTOR,
            unknown_object_from_skipped_inline(segment),
        );
        *inline = Inline::Ruby(annotation);
        self.can_merge_current_text_run = false;
        true
    }
}

struct SourceTextPart {
    text: String,
    break_after: bool,
}

// 改行（\r\n / \r / \n）位置で TextRun を分割し、段落分割位置を mark する。
// スパン追跡なしの縮約版（分割位置の判定ロジックは旧実装と同一）。
fn source_text_parts(text: &str) -> Vec<SourceTextPart> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            '\r' => {
                parts.push(SourceTextPart {
                    text: std::mem::take(&mut current),
                    break_after: true,
                });
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            '\n' => {
                parts.push(SourceTextPart {
                    text: std::mem::take(&mut current),
                    break_after: true,
                });
            }
            character => {
                current.push(character);
            }
        }
    }

    parts.push(SourceTextPart {
        text: current,
        break_after: false,
    });
    parts
}

fn unknown_object_from_skipped_inline(
    segment: &SkippedInlineTextSegment,
) -> UnknownObject {
    UnknownObject::new(
        UnknownRecordKind::new(Some(DOCUMENT_TEXT_INLINE_START_TAG)),
        segment.raw_bytes().to_vec(),
    )
}
