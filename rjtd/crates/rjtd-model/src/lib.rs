//! Plain-text extraction model for Ichitaro JTD documents.

use std::collections::BTreeMap;

use rjtd_core::container::{
    CfbEntryReadMode, EntryKind, inspect_cfb_entries_with_mode, inspect_cfb_stream_chain,
    read_cfb_stream,
};
use rjtd_core::document_text::{
    DocumentTextElement, DocumentTextPayload, ParsedDocumentText, parse_document_text,
    read_document_text_payload_with_budget,
};
use rjtd_core::header_text::read_header_text;
use rjtd_core::sheet::read_document_sheets;
pub use rjtd_core::sheet::{DocumentSheetInfo, SheetItem};
use rjtd_core::{Error, ParseLimits, ResourceBudget, Result};

mod block_text_model;
mod parse;

pub use parse::{parse_document, parse_document_with_limits};

use block_text_model::*;
pub use block_text_model::{
    Block, Inline, Paragraph, RubyAnnotation, StyleRef, TextRun, UnknownBlock, UnknownObject,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    pub(crate) title: Option<String>,
}

impl Default for Metadata {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Metadata {
    pub fn new(title: Option<String>) -> Self {
        Self { title }
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSheet {
    index: usize,
    name: String,
    storage_path: String,
    original_path: Option<String>,
    text: String,
    footnote_text: Option<String>,
}

impl DocumentSheet {
    pub fn new(
        index: usize,
        name: impl Into<String>,
        storage_path: impl Into<String>,
        original_path: Option<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            index,
            name: name.into(),
            storage_path: storage_path.into(),
            original_path,
            text: text.into(),
            footnote_text: None,
        }
    }

    pub fn with_footnote_text(mut self, footnote_text: impl Into<String>) -> Self {
        self.footnote_text = Some(footnote_text.into());
        self
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn storage_path(&self) -> &str {
        &self.storage_path
    }

    pub fn original_path(&self) -> Option<&str> {
        self.original_path.as_deref()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn footnote_text(&self) -> Option<&str> {
        self.footnote_text.as_deref()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    metadata: Metadata,
    blocks: Vec<Block>,
    sheets: Vec<DocumentSheet>,
    header_text: Option<String>,
}

impl Document {
    pub fn new(metadata: Metadata, blocks: Vec<Block>) -> Self {
        Self {
            metadata,
            blocks,
            sheets: Vec::new(),
            header_text: None,
        }
    }

    /// /Header（ヘッダ・フッタ本文）を保持する（PARTIAL-LOSS-REPORT.md 付録2 P-H＋F）。
    /// Tika 準拠で本文前（先頭行）への前置き出力用（cat / export-txt 共用）。
    pub fn with_header_text(mut self, header_text: impl Into<String>) -> Self {
        self.header_text = Some(header_text.into());
        self
    }

    pub fn header_text(&self) -> Option<&str> {
        self.header_text.as_deref()
    }

    pub fn from_plain_text(text: &str) -> Self {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let lines = normalized
            .strip_suffix('\n')
            .unwrap_or(&normalized)
            .split('\n');
        let blocks = lines
            .filter(|line| !line.is_empty())
            .map(|line| Block::Paragraph(Paragraph::from_text(line)))
            .collect();

        Self::new(Metadata::default(), blocks)
    }

    pub fn from_document_text(text: &ParsedDocumentText) -> Self {
        let mut builder = DocumentTextModelBuilder::default();

        for element in text.elements() {
            match element {
                DocumentTextElement::TextRun(text) => builder.push_text_run(text),
                DocumentTextElement::InlineText(segment) => builder.push_inline_text(segment),
                DocumentTextElement::SkippedInlineText(segment) => {
                    builder.push_skipped_inline(segment)
                }
                DocumentTextElement::ControlBoundary(control) => {
                    builder.push_control_boundary(control);
                }
            }
        }

        let blocks = builder.finish();
        Self::new(Metadata::default(), blocks)
    }

    pub fn from_document_text_payload(payload: &DocumentTextPayload) -> Self {
        let mut builder = DocumentTextModelBuilder::default();

        for element in payload.parsed_text().elements() {
            match element {
                DocumentTextElement::TextRun(text) => builder.push_text_run(text),
                DocumentTextElement::InlineText(segment) => builder.push_inline_text(segment),
                DocumentTextElement::SkippedInlineText(segment) => {
                    builder.push_skipped_inline(segment)
                }
                DocumentTextElement::ControlBoundary(control) => {
                    builder.push_control_boundary(control);
                }
            }
        }

        let blocks = builder.finish();
        Self::new(Metadata::default(), blocks)
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn sheets(&self) -> &[DocumentSheet] {
        &self.sheets
    }

    pub fn push_sheet(&mut self, sheet: DocumentSheet) {
        self.sheets.push(sheet);
    }

    pub fn sheet_plain_text(&self, index: usize) -> Option<&str> {
        self.sheets.get(index).map(|s| s.text())
    }

    pub fn sheet_by_name(&self, name: &str) -> Option<&DocumentSheet> {
        self.sheets.iter().find(|s| s.name() == name)
    }

    pub fn plain_text(&self) -> String {
        if self.sheets.len() <= 1 {
            let mut text = document_plain_text(self);
            if let Some(sheet) = self.sheets.first()
                && let Some(fn_text) = sheet.footnote_text()
            {
                text.push_str("\n\n");
                text.push_str(fn_text.trim());
            }
            text
        } else {
            let mut output = String::new();
            for (i, sheet) in self.sheets.iter().enumerate() {
                if i > 0 {
                    output.push_str("\n\n");
                }
                output.push_str(&format!("{}\n", sheet.name()));
                output.push_str(sheet.text().trim());
                if let Some(fn_text) = sheet.footnote_text() {
                    output.push_str("\n\n");
                    output.push_str(fn_text.trim());
                }
            }
            output
        }
    }
}

pub trait DocumentParser {
    fn parse(&self, data: &[u8]) -> Result<Document>;
}

pub struct IchitaroParser;

impl IchitaroParser {
    pub(crate) fn parse_with_budget(
        &self,
        data: &[u8],
        budget: &mut ResourceBudget,
    ) -> Result<Document> {
        reserve_and_verify_cfb_streams(data, budget)?;
        let payload =
            read_document_text_payload_with_budget(data, budget.decompression_budget_mut())?;
        let mut document = Document::from_document_text_payload(&payload);

        // P-H＋F（PARTIAL-LOSS-REPORT.md 付録2）: /Header のヘッダ・フッタ本文を保持する
        // （core の read_header_text 共用で cat と同一規則）。/Header 欠落・全 span 空・
        // 読み取り失敗は None のまま（従来出力不変）。
        if let Ok(Some(header)) = read_header_text(data)
            && !header.text().trim().is_empty()
        {
            document = document.with_header_text(header.text());
        }

        if let Ok(sheet_infos) = read_document_sheets(data) {
            if !sheet_infos.is_empty() {
                for sheet_info in sheet_infos {
                    let text = if sheet_info.storage_path().is_empty()
                        || sheet_info.storage_path() == "/"
                    {
                        document_plain_text(&document)
                    } else {
                        let text_path = sheet_info.document_text_path();
                        if let Ok(stream_bytes) = read_cfb_stream(data, &text_path) {
                            parse_document_text(&stream_bytes).plain_text()
                        } else {
                            String::new()
                        }
                    };
                    let footnote_path = sheet_info.footnote_path();
                    let footnote_text = read_cfb_stream(data, &footnote_path)
                        .ok()
                        .and_then(|stream_bytes| read_footnote_text(&stream_bytes));
                    let mut sheet = DocumentSheet::new(
                        sheet_info.index(),
                        sheet_info.name(),
                        sheet_info.storage_path(),
                        sheet_info.original_path().map(str::to_string),
                        text,
                    );
                    if let Some(fn_text) = footnote_text {
                        sheet = sheet.with_footnote_text(fn_text);
                    }
                    document.push_sheet(sheet);
                }
            }
        }
        if document.sheets().is_empty() {
            let root_text = document_plain_text(&document);
            let root_footnote = read_cfb_stream(data, "/Footnote")
                .ok()
                .and_then(|stream_bytes| read_footnote_text(&stream_bytes));
            let mut sheet = DocumentSheet::new(0, "タイトル", "", None, root_text);
            if let Some(fn_text) = root_footnote {
                sheet = sheet.with_footnote_text(fn_text);
            }
            document.push_sheet(sheet);
        }
        Ok(document)
    }
}

impl DocumentParser for IchitaroParser {
    fn parse(&self, data: &[u8]) -> Result<Document> {
        let mut budget = ParseLimits::DEFAULT.resource_budget();
        budget.check_input_size(data.len())?;
        self.parse_with_budget(data, &mut budget)
    }
}

// Footnote stream entries are cross-reference anchor labels
// ([0x001c,0x0001,0x0007,0x0000,0x0000,0x0001], 0x001d, <label>, 0x001e) each
// followed by the note body. The decoded label carries the Ichitaro-rendered
// numbering itself, so any decoration works ([1], (1), 1), *1, ①, [注1], ...).
// Walking the elements structurally pairs each label (selector 0x0001) with the
// note body TextRun(s) that follow it; a trailing label with no body (the
// internal template sentinel, e.g. the default label `Note`) never pairs with
// anything and is dropped without hardcoding its text.
const FOOTNOTE_ANCHOR_SELECTOR: u16 = 0x0001;

fn read_footnote_text(stream_bytes: &[u8]) -> Option<String> {
    let parsed = parse_document_text(stream_bytes);
    let mut output = String::new();
    let mut current_label: Option<&str> = None;

    for element in parsed.elements() {
        match element {
            DocumentTextElement::InlineText(segment) => {
                if segment.selector() == FOOTNOTE_ANCHOR_SELECTOR {
                    current_label = Some(segment.text());
                }
            }
            DocumentTextElement::TextRun(text) => {
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                if let Some(label) = current_label.take() {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(label);
                    output.push(' ');
                    output.push_str(text);
                } else {
                    output.push(' ');
                    output.push_str(text);
                }
            }
            DocumentTextElement::SkippedInlineText(_) | DocumentTextElement::ControlBoundary(_) => {
            }
        }
    }

    let trimmed = output.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn document_plain_text(document: &Document) -> String {
    let mut output = String::new();

    for block in document.blocks() {
        if let Block::Paragraph(paragraph) = block {
            output.push_str(&paragraph_text(paragraph));
            output.push('\n');
        }
    }

    rjtd_core::document_text::trim_trailing_exposed_controls(&output).to_string()
}

fn paragraph_text(paragraph: &Paragraph) -> String {
    let mut text = String::new();

    for inline in paragraph.inlines() {
        match inline {
            Inline::Text(run) => text.push_str(run.text()),
            Inline::Ruby(ruby) => text.push_str(ruby.base_text()),
            Inline::Unknown(_) => {}
        }
    }

    text
}

fn reserve_and_verify_cfb_streams(data: &[u8], budget: &mut ResourceBudget) -> Result<()> {
    let Ok((entries, mode)) = inspect_cfb_entries_with_mode(data) else {
        return Ok(());
    };
    let mut streams = BTreeMap::new();

    for entry in entries
        .iter()
        .filter(|entry| entry.kind() == EntryKind::Stream)
    {
        streams
            .entry(entry.path())
            .and_modify(|size: &mut u64| *size = (*size).max(entry.size()))
            .or_insert(entry.size());
    }

    for (path, declared) in streams {
        let accounted = match mode {
            CfbEntryReadMode::Strict => cfb_stream_bytes_from_u64(declared)?,
            CfbEntryReadMode::Lenient => reachable_cfb_stream_bytes(data, path).unwrap_or(0),
        };
        budget.reserve_streams(1, accounted)?;

        let Ok(stream) = read_cfb_stream(data, path) else {
            continue;
        };
        budget.verify_stream_bytes(accounted, stream.len())?;
    }

    Ok(())
}

fn cfb_stream_bytes_from_u64(size: u64) -> Result<usize> {
    usize::try_from(size).map_err(|_| Error::ResourceLimit {
        resource: "document stream bytes",
        limit: usize::MAX,
        actual: usize::MAX,
    })
}

fn reachable_cfb_stream_bytes(data: &[u8], path: &str) -> Result<usize> {
    let chain = inspect_cfb_stream_chain(data, path)?;
    let capacity = u64::try_from(chain.capacity_bytes()).unwrap_or(u64::MAX);
    cfb_stream_bytes_from_u64(chain.location().size().min(capacity))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
