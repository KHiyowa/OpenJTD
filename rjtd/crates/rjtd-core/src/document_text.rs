use crate::compressed_document::{
    decompress_just_compressed_document_with_budget, is_just_compressed_document,
};
use crate::container::read_cfb_stream;
use crate::{DecompressionBudget, Error, ParseLimits, Result};

mod row_headers;
mod style_runs;

pub use row_headers::{
    DocumentTextRowHeaderFixedFields, DocumentTextRowHeaderPair,
    DocumentTextRowHeaderPairClassification, DocumentTextRowHeaderRecord,
    parse_document_text_row_headers,
};

pub use style_runs::{
    DocumentTextResolvedStyle, DocumentTextStyleDiagnostic, DocumentTextStyleDiagnosticKind,
    DocumentTextStyleEvent, DocumentTextStyleProperty, DocumentTextStylePropertyChangeEvent,
    DocumentTextStyleResolver, DocumentTextStyleRunEvent, DocumentTextStyleSection,
    DocumentTextStyleTypedValue, document_text_style_code_name, parse_document_text_style_section,
};

pub const DOCUMENT_TEXT_PATH: &str = "/DocumentText";
pub const COMPRESSED_DOCUMENT_PATH: &str = "/JSCompDocument";
pub const EMBEDDED_DOCUMENT_TEXT_PATH: &str = "/EmbeddedDocumentText";
const DOCUMENT_TEXT_MAGIC: &[u8; 8] = b"SsmgV.01";
const EMBEDDED_DOCUMENT_TEXT_MAX_SPAN: usize = 64 * 1024;
const TEXT_RUN_MARKER: u16 = 0x001f;
const INLINE_TEXT_START: u16 = 0x001d;
const INLINE_TEXT_END: u16 = 0x001e;
// 0x000e separates 0x001c/0x0030 table-cell records; reading_text stays true across it.
// 0x000c is a page break (form feed, see RFC 0003); reading_text stays true across it.
// 0x000a is a within-cell/intra-paragraph line break (see RFC 0009); treated as a plain
// text character ('\n') by is_control_boundary, which intentionally excludes 0x09/0x0a/0x0d.
const TEXT_ROW_DELIMITER: u16 = 0x000e;
pub const DOCUMENT_TEXT_PAGE_BREAK_CONTROL: u16 = 0x000c;
// 0x0010 inside a text run acts as an inline space / formatting boundary control; reading_text stays true across it.
pub const DOCUMENT_TEXT_INLINE_SPACE_CONTROL: u16 = 0x0010;
const SKIPPED_INLINE_MAX_UNITS: usize = 256;

// RFC 0009: 0x001c record class codes (decoded:false — structure proven, semantics partial)
pub const RECORD_CLASS_INLINE_CONTEXT: u16 = 0x0000;
pub const RECORD_CLASS_PARAGRAPH_LINE: u16 = 0x0010;
pub const RECORD_CLASS_TABLE_SECTION_TRANSITION: u16 = 0x0020;
pub const RECORD_CLASS_TABLE_CELL: u16 = 0x0030;
// RFC 0009: 0x001c record start marker (レコード開始マーカー). Unlike RECORD_CLASS_*
// (class codes that follow it), 0x001c opens a record, so it is kept as a separate constant.
pub const RECORD_START_MARKER: u16 = 0x001c;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedDocumentText {
    elements: Vec<DocumentTextElement>,
}

impl ParsedDocumentText {
    fn new(elements: Vec<DocumentTextElement>) -> Self {
        Self { elements }
    }

    pub fn from_text(text: impl Into<String>) -> Self {
        let text = text.into();
        if text.is_empty() {
            Self::default()
        } else {
            Self::new(vec![DocumentTextElement::TextRun(text)])
        }
    }

    pub fn elements(&self) -> &[DocumentTextElement] {
        &self.elements
    }

    pub fn plain_text(&self) -> String {
        let mut output = String::new();
        for element in &self.elements {
            match element {
                DocumentTextElement::TextRun(text) => output.push_str(text),
                DocumentTextElement::InlineText(segment) => output.push_str(segment.text()),
                DocumentTextElement::SkippedInlineText(_) => {}
                DocumentTextElement::ControlBoundary(_) => {}
            }
        }
        trim_trailing_exposed_controls(&output).to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentTextElement {
    TextRun(String),
    InlineText(InlineTextSegment),
    SkippedInlineText(SkippedInlineTextSegment),
    ControlBoundary(DocumentTextControl),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentTextMap {
    entries: Vec<DocumentTextMapEntry>,
}

impl DocumentTextMap {
    fn new(entries: Vec<DocumentTextMapEntry>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &[DocumentTextMapEntry] {
        &self.entries
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentTextMapEntry {
    byte_start: usize,
    byte_end: usize,
    unit_start: usize,
    unit_end: usize,
    kind: DocumentTextMapKind,
    selector: Option<u16>,
    code: Option<u16>,
    text: String,
}

impl DocumentTextMapEntry {
    fn new(
        unit_start: usize,
        unit_end: usize,
        kind: DocumentTextMapKind,
        selector: Option<u16>,
        code: Option<u16>,
        text: String,
    ) -> Self {
        Self {
            byte_start: unit_start * 2,
            byte_end: unit_end * 2,
            unit_start,
            unit_end,
            kind,
            selector,
            code,
            text,
        }
    }

    pub fn byte_start(&self) -> usize {
        self.byte_start
    }

    pub fn byte_end(&self) -> usize {
        self.byte_end
    }

    pub fn unit_start(&self) -> usize {
        self.unit_start
    }

    pub fn unit_end(&self) -> usize {
        self.unit_end
    }

    pub fn kind(&self) -> DocumentTextMapKind {
        self.kind
    }

    pub fn selector(&self) -> Option<u16> {
        self.selector
    }

    pub fn code(&self) -> Option<u16> {
        self.code
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn contains_byte_offset(&self, offset: usize) -> bool {
        self.byte_start <= offset && offset < self.byte_end
    }

    pub fn contains_unit_offset(&self, offset: usize) -> bool {
        self.unit_start <= offset && offset < self.unit_end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentTextMapKind {
    TextRun,
    InlineText,
    SkippedInlineText,
    ControlBoundary,
}

impl DocumentTextMapKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TextRun => "text",
            Self::InlineText => "inline",
            Self::SkippedInlineText => "skipped-inline",
            Self::ControlBoundary => "control",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineTextSegment {
    selector: u16,
    text: String,
}

impl InlineTextSegment {
    fn new(selector: u16, text: String) -> Self {
        Self { selector, text }
    }

    pub fn selector(&self) -> u16 {
        self.selector
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedInlineTextSegment {
    context: Vec<u16>,
    text: String,
    raw_bytes: Vec<u8>,
}

impl SkippedInlineTextSegment {
    fn new(context: Vec<u16>, text: String, raw_bytes: Vec<u8>) -> Self {
        Self {
            context,
            text,
            raw_bytes,
        }
    }

    pub fn selector(&self) -> Option<u16> {
        self.context.last().copied()
    }

    pub fn context(&self) -> &[u16] {
        &self.context
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentTextControl {
    code: u16,
}

impl DocumentTextControl {
    fn new(code: u16) -> Self {
        Self { code }
    }

    pub fn code(&self) -> u16 {
        self.code
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentTextSourceSpan {
    byte_start: usize,
    byte_end: usize,
    unit_start: usize,
    unit_end: usize,
}

impl DocumentTextSourceSpan {
    fn new(unit_start: usize, unit_end: usize) -> Self {
        Self {
            byte_start: unit_start * 2,
            byte_end: unit_end * 2,
            unit_start,
            unit_end,
        }
    }

    pub fn byte_start(&self) -> usize {
        self.byte_start
    }

    pub fn byte_end(&self) -> usize {
        self.byte_end
    }

    pub fn unit_start(&self) -> usize {
        self.unit_start
    }

    pub fn unit_end(&self) -> usize {
        self.unit_end
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentTextPayload {
    source_name: String,
    bytes: Vec<u8>,
    parsed_text: ParsedDocumentText,
    text: String,
}

impl DocumentTextPayload {
    fn new(
        source_name: impl Into<String>,
        bytes: Vec<u8>,
        parsed_text: ParsedDocumentText,
    ) -> Self {
        let text = parsed_text.plain_text();
        Self {
            source_name: source_name.into(),
            bytes,
            parsed_text,
            text,
        }
    }

    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn parsed_text(&self) -> &ParsedDocumentText {
        &self.parsed_text
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

pub fn read_document_text_payload(data: &[u8]) -> Result<DocumentTextPayload> {
    read_document_text_payload_with_limits(data, ParseLimits::DEFAULT)
}

/// Reads document text from already allocated input with explicit resource limits.
///
/// Each LH5 member and the combined output of every member reached by this call are limited. The
/// input check occurs after the caller has allocated `data`.
pub fn read_document_text_payload_with_limits(
    data: &[u8],
    limits: ParseLimits,
) -> Result<DocumentTextPayload> {
    let mut budget = limits.decompression_budget();
    read_document_text_payload_with_budget(data, &mut budget)
}

/// Reads document text while sharing a cumulative LH5 output budget with other readers.
///
/// This is public only for composition between rjtd crates; downstream callers should prefer
/// [`read_document_text_payload_with_limits`]. It is not a stable downstream API.
pub fn read_document_text_payload_with_budget(
    data: &[u8],
    budget: &mut DecompressionBudget,
) -> Result<DocumentTextPayload> {
    budget.check_input_size(data.len())?;
    match read_cfb_stream(data, DOCUMENT_TEXT_PATH) {
        Ok(stream) => Ok(DocumentTextPayload::new(
            DOCUMENT_TEXT_PATH,
            stream.clone(),
            parse_document_text(&stream),
        )),
        Err(Error::NotFound(_)) => read_compressed_or_embedded_document_text(data, budget),
        Err(error) => Err(error),
    }
}

pub fn read_document_text_stream(data: &[u8]) -> Result<Vec<u8>> {
    Ok(read_document_text_payload(data)?.bytes)
}

fn read_compressed_or_embedded_document_text(
    data: &[u8],
    budget: &mut DecompressionBudget,
) -> Result<DocumentTextPayload> {
    let stream = match read_cfb_stream(data, COMPRESSED_DOCUMENT_PATH) {
        Ok(stream) => stream,
        Err(Error::NotFound(_)) => return read_embedded_document_text(data),
        Err(error) => return Err(error),
    };

    if !is_just_compressed_document(&stream) {
        return read_embedded_document_text(data);
    }

    let inner_document = decompress_just_compressed_document_with_budget(&stream, budget)?;
    let bytes = read_cfb_stream(&inner_document, DOCUMENT_TEXT_PATH)?;
    let parsed_text = parse_document_text(&bytes);
    Ok(DocumentTextPayload::new(
        DOCUMENT_TEXT_PATH,
        bytes,
        parsed_text,
    ))
}

pub fn has_embedded_document_text(data: &[u8]) -> bool {
    embedded_document_text(data).is_some()
}

fn read_embedded_document_text(data: &[u8]) -> Result<DocumentTextPayload> {
    embedded_document_text(data)
        .ok_or_else(|| Error::NotFound(format!("stream `{DOCUMENT_TEXT_PATH}`")))
}

fn embedded_document_text(data: &[u8]) -> Option<DocumentTextPayload> {
    let offsets = find_document_text_magic_offsets(data);
    let mut bytes = Vec::new();
    let mut text_parts = Vec::new();

    for (index, start) in offsets.iter().copied().enumerate() {
        let next_start = offsets.get(index + 1).copied().unwrap_or(data.len());
        let end = next_start.min(start.saturating_add(EMBEDDED_DOCUMENT_TEXT_MAX_SPAN));
        if end <= start {
            continue;
        }
        let fragment = &data[start..end];
        let text = clean_embedded_text(&extract_document_text(fragment));
        if text.trim().is_empty() || text_parts.iter().any(|part| part == &text) {
            continue;
        }

        if !bytes.is_empty() {
            bytes.extend_from_slice(&[0, 0]);
        }
        bytes.extend_from_slice(fragment);
        text_parts.push(text);
    }

    if text_parts.is_empty() {
        None
    } else {
        Some(DocumentTextPayload::new(
            EMBEDDED_DOCUMENT_TEXT_PATH,
            bytes,
            ParsedDocumentText::from_text(text_parts.join("\n")),
        ))
    }
}

fn clean_embedded_text(text: &str) -> String {
    text.split(['\r', '\n'])
        .filter_map(|line| {
            let line = line.trim_matches('\0');
            if line.trim().is_empty() || !is_plausible_embedded_line(line) {
                None
            } else {
                Some(line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_plausible_embedded_line(line: &str) -> bool {
    let mut total = 0usize;
    let mut plausible = 0usize;

    for character in line.chars().filter(|character| !character.is_whitespace()) {
        total += 1;
        if is_plausible_embedded_character(character) {
            plausible += 1;
        }
    }

    total > 0 && plausible * 100 >= total * 80
}

fn is_plausible_embedded_character(character: char) -> bool {
    character.is_ascii_graphic()
        || matches!(
            character as u32,
            0x3000..=0x30ff
                | 0x31f0..=0x31ff
                | 0x3200..=0x33ff
                | 0x4e00..=0x9fff
                | 0xff00..=0xffef
        )
        || matches!(
            character,
            '、' | '。'
                | '・'
                | '「'
                | '」'
                | '『'
                | '』'
                | '【'
                | '】'
                | '（'
                | '）'
                | '［'
                | '］'
                | '→'
                | '←'
                | '↑'
                | '↓'
                | '～'
                | '…'
                | '◎'
                | '○'
                | '●'
                | '◆'
                | '☆'
                | '★'
                | '※'
        )
}

fn find_document_text_magic_offsets(data: &[u8]) -> Vec<usize> {
    data.windows(DOCUMENT_TEXT_MAGIC.len())
        .enumerate()
        .filter_map(|(offset, window)| (window == DOCUMENT_TEXT_MAGIC).then_some(offset))
        .collect()
}

pub fn extract_document_text(data: &[u8]) -> String {
    parse_document_text(data).plain_text()
}

// SsmgV.01 segment-count field: w[9]=0x0001 means a single raw-text TextV.01 segment
// with no paragraph records; w[9]=0x0002 is the normal paragraph-record format.
const SSMG_RAW_TEXT_SEGMENT_COUNT: u16 = 0x0001;
const SSMG_HEADER_WORDS: usize = 10; // SsmgV.01 (4) + header (4) + segment-count (2)
const TEXT_SEGMENT_NAME: &[u8; 8] = b"TextV.01";
const CONTENT_UNIT_COUNT_OFFSET: usize = 28;
const TEXT_CONTENT_HEADER_WORDS: usize = 16; // 32 bytes

fn document_text_unit_limit(data: &[u8]) -> Option<usize> {
    if data.starts_with(DOCUMENT_TEXT_MAGIC)
        && data.len() >= TEXT_CONTENT_HEADER_WORDS * 2
        && data.get(SSMG_HEADER_WORDS * 2..SSMG_HEADER_WORDS * 2 + TEXT_SEGMENT_NAME.len())
            == Some(TEXT_SEGMENT_NAME)
    {
        let count = u32::from_be_bytes([
            data[CONTENT_UNIT_COUNT_OFFSET],
            data[CONTENT_UNIT_COUNT_OFFSET + 1],
            data[CONTENT_UNIT_COUNT_OFFSET + 2],
            data[CONTENT_UNIT_COUNT_OFFSET + 3],
        ]) as usize;
        Some(TEXT_CONTENT_HEADER_WORDS.saturating_add(count))
    } else {
        None
    }
}

pub fn parse_document_text(data: &[u8]) -> ParsedDocumentText {
    // SsmgV.01 w[9]=0x0001: single raw-text segment (no 0x001f paragraph markers).
    // Layout: SsmgV.01 header (10 words) + TextV.01 name (4 words) + length (2 words) + text.
    // 実原本（官公庁テンプレート）には w[9]=0x0003..=0x000b のマーカーレス raw 型も存在する
    // （needs-doc-pdf-conversion 調査、REPORT.md 案1）。
    if data.starts_with(DOCUMENT_TEXT_MAGIC) {
        let units: Vec<u16> = data
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect();
        // 既存ゲート（w[9]==0x0001 と TextV.01 プレフィックス）を維持しつつ、
        // 実原本に存在するマーカーレス raw 型（w[9] が 0x0003 以降）も通す。
        // マーカー型（通常）ファイルは本文領域に 0x001c/0x001d/0x001f を含むため
        // is_markerless_raw_text_span() に落ちず、通常パスの挙動は不変（リグレッションガード）。
        if data
            .get(SSMG_HEADER_WORDS * 2..)
            .is_some_and(|rest| rest.starts_with(TEXT_SEGMENT_NAME))
            && (units.get(9) == Some(&SSMG_RAW_TEXT_SEGMENT_COUNT)
                || is_markerless_raw_text_span(&units))
        {
            return parse_raw_text_segment(&units);
        }
    }

    let units = data
        .chunks_exact(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    let unit_limit = document_text_unit_limit(data)
        .unwrap_or(units.len())
        .min(units.len());
    let mut elements = Vec::new();
    let mut run = String::new();
    let mut reading_text = false;
    let mut has_prose = false;
    let mut index = 0;
    let mut numbering_state = NumberingState::default();

    while index < unit_limit {
        let code = units[index];
        if code == TEXT_RUN_MARKER {
            if !is_document_text_run_start(&units, index, has_prose) {
                index += 1;
                continue;
            }
            push_run(&mut elements, &mut run);
            if let Some(prefix) = paragraph_header_prefix(&units, index, &mut numbering_state) {
                run.push_str(&prefix);
            }
            reading_text = true;
            has_prose = true;
            index += 1;
            continue;
        }

        if code == INLINE_TEXT_START {
            push_run(&mut elements, &mut run);
            reading_text = false;
            if let Some(selector) = inline_text_selector(&units, index) {
                index = push_inline_segment(&mut elements, &units, index, selector);
                has_prose = true;
            } else if skipped_inline_selector(&units, index).is_some()
                && let Some((segment, next_index)) = read_skipped_inline_segment(&units, index)
            {
                elements.push(DocumentTextElement::SkippedInlineText(segment));
                index = next_index;
            } else {
                elements.push(DocumentTextElement::ControlBoundary(
                    DocumentTextControl::new(code),
                ));
                index += 1;
            }
            continue;
        }

        if reading_text {
            if is_control_boundary(code) || is_invalid_scalar(code) {
                push_run(&mut elements, &mut run);
                elements.push(DocumentTextElement::ControlBoundary(
                    DocumentTextControl::new(code),
                ));
                reading_text = code == TEXT_ROW_DELIMITER
                    || code == DOCUMENT_TEXT_PAGE_BREAK_CONTROL
                    || code == DOCUMENT_TEXT_INLINE_SPACE_CONTROL;
            } else if let Some(character) = char::from_u32(code as u32) {
                run.push(character);
                has_prose = true;
            }
        }

        index += 1;
    }

    push_run(&mut elements, &mut run);
    ParsedDocumentText::new(elements)
}

pub fn map_document_text(data: &[u8]) -> DocumentTextMap {
    let units = data
        .chunks_exact(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    let unit_limit = document_text_unit_limit(data)
        .unwrap_or(units.len())
        .min(units.len());
    let mut entries = Vec::new();
    let mut run = String::new();
    let mut run_start = 0usize;
    let mut reading_text = false;
    let mut has_prose = false;
    let mut index = 0;

    while index < unit_limit {
        let code = units[index];
        if code == TEXT_RUN_MARKER {
            if !is_document_text_run_start(&units, index, has_prose) {
                index += 1;
                continue;
            }
            push_map_run(&mut entries, &mut run, run_start, index);
            reading_text = true;
            has_prose = true;
            index += 1;
            continue;
        }

        if code == INLINE_TEXT_START {
            push_map_run(&mut entries, &mut run, run_start, index);
            reading_text = false;
            if let Some(selector) = inline_text_selector(&units, index) {
                index = push_mapped_inline_segment(&mut entries, &units, index, selector);
                has_prose = true;
            } else if skipped_inline_selector(&units, index).is_some()
                && let Some((segment, next_index)) = read_skipped_inline_segment(&units, index)
            {
                entries.push(DocumentTextMapEntry::new(
                    index,
                    next_index,
                    DocumentTextMapKind::SkippedInlineText,
                    segment.selector(),
                    None,
                    segment.text().to_string(),
                ));
                index = next_index;
            } else {
                push_map_control(&mut entries, index, code);
                index += 1;
            }
            continue;
        }

        if reading_text {
            if is_control_boundary(code) || is_invalid_scalar(code) {
                push_map_run(&mut entries, &mut run, run_start, index);
                push_map_control(&mut entries, index, code);
                reading_text = code == TEXT_ROW_DELIMITER
                    || code == DOCUMENT_TEXT_PAGE_BREAK_CONTROL
                    || code == DOCUMENT_TEXT_INLINE_SPACE_CONTROL;
                run_start = index + 1;
            } else if let Some(character) = char::from_u32(code as u32) {
                if run.is_empty() {
                    run_start = index;
                }
                run.push(character);
                has_prose = true;
            }
        }

        index += 1;
    }

    push_map_run(&mut entries, &mut run, run_start, unit_limit);
    DocumentTextMap::new(entries)
}

// マーカーレス raw テキスト型（w[9] が 0x0001 より大きい実原本）の識別。
// TextV.01 セグメント内の word 15 が本文長（word 数）であり、本文領域
// units[16..16+len] に RFC 0009 のマーカー 0x001c/0x001d/0x001f が一切含まれない
// ときのみ raw デコードする。マーカー型ファイルは本文にマーカーを含むため
// ここの条件を満たさず、通常パスの挙動は不変となる（リグレッションガード）。
fn is_markerless_raw_text_span(units: &[u16]) -> bool {
    units.len() >= 16
        && units[14] == 0x0000
        && 0 < units[15] as usize
            && units[15] as usize <= units.len() - 16
        && units[16..16 + units[15] as usize]
            .iter()
            .all(|&code| {
                code != RECORD_START_MARKER
                    && code != INLINE_TEXT_START
                    && code != TEXT_RUN_MARKER
            })
}

// Parse a SsmgV.01 w[9]=0x0001 raw-text segment: TextV.01 header (14..16) gives
// the word count, then the text follows as plain UTF-16BE with no 0x001f markers.
// w[9]>1 のマーカーレス raw 型（is_markerless_raw_text_span で識別）も同一レイアウトで通す。
fn parse_raw_text_segment(units: &[u16]) -> ParsedDocumentText {
    // Layout: SSMG_HEADER_WORDS=10 + TextV.01 name (4) + length field (2) = 16 words header
    const HEADER_WORDS: usize = SSMG_HEADER_WORDS + 4 + 2;
    let length = match units.get(SSMG_HEADER_WORDS + 4 + 1) {
        Some(&len) => len as usize,
        None => return ParsedDocumentText::default(),
    };
    let text_start = HEADER_WORDS;
    let text_end = text_start.saturating_add(length).min(units.len());
    let mut run = String::new();
    for &code in &units[text_start..text_end] {
        if code == 0x0000 {
            break;
        }
        if !is_invalid_scalar(code)
            && let Some(character) = char::from_u32(code as u32)
        {
            run.push(character);
        }
    }
    if run.is_empty() {
        ParsedDocumentText::default()
    } else {
        ParsedDocumentText::new(vec![DocumentTextElement::TextRun(run)])
    }
}

fn push_run(elements: &mut Vec<DocumentTextElement>, run: &mut String) {
    if !run.is_empty() {
        elements.push(DocumentTextElement::TextRun(std::mem::take(run)));
        run.clear();
    }
}

#[derive(Debug, Default, Clone)]
struct NumberingState {
    chapter: usize,
    section: usize,
    subsection: usize,
    list_style: Option<u16>,
    list_counter: usize,
    style8_level1_counter: usize,
    style8_level2_counter: usize,
}

fn circled_number(n: usize) -> String {
    const CIRCLED: &[char] = &[
        '①', '②', '③', '④', '⑤', '⑥', '⑦', '⑧', '⑨', '⑩',
        '⑪', '⑫', '⑬', '⑭', '⑮', '⑯', '⑰', '⑱', '⑲', '⑳',
    ];
    if (1..=CIRCLED.len()).contains(&n) {
        CIRCLED[n - 1].to_string()
    } else {
        format!("({})", n)
    }
}

fn alpha_number(n: usize) -> String {
    if (1..=26).contains(&n) {
        let c = (b'a' + (n - 1) as u8) as char;
        format!("({})", c)
    } else {
        format!("({})", n)
    }
}

fn paragraph_header_prefix(
    units: &[u16],
    marker_index: usize,
    state: &mut NumberingState,
) -> Option<String> {
    if marker_index < 3 || units[marker_index - 2] != 0x0000 || units[marker_index - 1] != 0x0010 {
        return None;
    }
    let total_len = units[marker_index - 3] as usize;
    if total_len < 4 || marker_index + 1 < total_len {
        return None;
    }
    let start = (marker_index + 1) - total_len;
    if units[start] != 0x001c || units[start + 1] != 0x0010 || units[start + 2] != total_len as u16 {
        return None;
    }
    let header = &units[start..=marker_index];

    let a3_opt = header.windows(3).find_map(|w| {
        if w[0] == 0x00a3 && w[1] == 0x0002 {
            Some(w[2])
        } else {
            None
        }
    });
    let level_50 = header.windows(3).find_map(|w| {
        if w[0] == 0x0050 && w[1] == 0x0002 {
            Some(w[2])
        } else {
            None
        }
    });

    if let Some(style) = a3_opt {
        if style == 1 && level_50.is_some() {
            state.list_style = None;
            state.list_counter = 0;
            let level = level_50.unwrap();
            let prefix = match level {
                1 => {
                    state.chapter += 1;
                    state.section = 0;
                    state.subsection = 0;
                    format!("第{}章 ", state.chapter)
                }
                2 => {
                    state.section += 1;
                    state.subsection = 0;
                    format!("{}.{} ", state.chapter, state.section)
                }
                3 => {
                    state.subsection += 1;
                    format!("{}.{}.{} ", state.chapter, state.section, state.subsection)
                }
                _ => format!("(level {}) ", level),
            };
            return Some(prefix);
        } else if style == 8 {
            let level = level_50.unwrap_or(1);
            if level == 1 {
                state.style8_level1_counter += 1;
                state.style8_level2_counter = 0;
                return Some(format!("{} ", circled_number(state.style8_level1_counter)));
            } else {
                state.style8_level2_counter += 1;
                let sym = circled_number(state.style8_level1_counter);
                return Some(format!("{}-{} ", sym, state.style8_level2_counter));
            }
        } else if [2, 3, 4].contains(&style) {
            let is_restart = header.windows(4).any(|w| {
                w[0] == 0x00a3 && w[1] == 0x0002 && w[2] == style && w[3] == 0x7fff
            });
            if state.list_style != Some(style) || is_restart {
                state.list_style = Some(style);
                state.list_counter = 1;
            } else {
                state.list_counter += 1;
            }
            let prefix = match style {
                2 => format!("({}) ", state.list_counter),
                3 => format!("{} ", alpha_number(state.list_counter)),
                4 => format!("{}. ", state.list_counter),
                _ => unreachable!(),
            };
            return Some(prefix);
        }
    } else {
        state.list_style = None;
        state.list_counter = 0;
    }

    None
}

fn is_control_boundary(code: u16) -> bool {
    (code < 0x20 && !matches!(code, 0x09 | 0x0a | 0x0d)) || (0x7f..=0x9f).contains(&code)
}

fn is_invalid_scalar(code: u16) -> bool {
    (0xd800..=0xdfff).contains(&code) || code == 0xffff
}

// Internal terminal/boundary code points that leak into extracted text as visible
// glyphs even though they are not prose. U+FE14 and U+0490 form the pair appended at
// the end of a DocumentText stream (seen as `︔Ґ`); U+0400 is an unassigned code point
// that only ever appears as leaking record-data.
fn is_exposed_terminal_control(character: char) -> bool {
    matches!(character as u32, 0x0400 | 0x0490 | 0xfe14)
}

/// Removes a trailing run of exposed terminal control characters (and any surrounding
/// whitespace) from `text`. The trailing region is the maximal suffix made up of
/// whitespace and exposed controls; it is removed only when it contains at least one
/// exposed control, so a trailing run of whitespace alone (a legitimate line terminator)
/// is left intact. Controls appearing anywhere except this trailing region are preserved.
pub fn trim_trailing_exposed_controls(text: &str) -> &str {
    let mut boundary = usize::MAX;
    let mut seen_control = false;
    for (index, character) in text.char_indices().rev() {
        if is_exposed_terminal_control(character) {
            seen_control = true;
            boundary = index;
        } else if character.is_whitespace() {
            boundary = index.min(boundary);
        } else {
            break;
        }
    }
    if seen_control {
        &text[..boundary]
    } else {
        text
    }
}

fn inline_text_selector(units: &[u16], index: usize) -> Option<u16> {
    if index < 6 {
        return None;
    }

    let context = &units[index - 6..index];
    if context[..5] == [0x001c, 0x0001, 0x0007, 0x0000, 0x0000]
        && matches!(context[5], 0x0001 | 0x0003 | 0x0013)
    {
        Some(context[5])
    } else {
        None
    }
}

fn skipped_inline_selector(units: &[u16], index: usize) -> Option<u16> {
    if index == 0 {
        return None;
    }

    if units[index - 1] == 0x001c {
        return Some(0x001c);
    }

    if index < 6 {
        return None;
    }

    let context = &units[index - 6..index];
    if context[..5] == [0x001c, 0x0001, 0x0007, 0x0000, 0x0001] {
        Some(context[5])
    } else {
        None
    }
}

fn push_inline_segment(
    elements: &mut Vec<DocumentTextElement>,
    units: &[u16],
    start: usize,
    selector: u16,
) -> usize {
    let mut index = start + 1;
    let mut text = String::new();
    while index < units.len() {
        let code = units[index];
        if code == INLINE_TEXT_END {
            if !text.is_empty() {
                elements.push(DocumentTextElement::InlineText(InlineTextSegment::new(
                    selector, text,
                )));
            }
            return index + 1;
        }

        if !is_control_boundary(code)
            && !is_invalid_scalar(code)
            && let Some(character) = char::from_u32(code as u32)
        {
            text.push(character);
        }
        index += 1;
    }

    if !text.is_empty() {
        elements.push(DocumentTextElement::InlineText(InlineTextSegment::new(
            selector, text,
        )));
    }
    index
}

fn push_mapped_inline_segment(
    entries: &mut Vec<DocumentTextMapEntry>,
    units: &[u16],
    start: usize,
    selector: u16,
) -> usize {
    let mut index = start + 1;
    let mut text = String::new();
    while index < units.len() {
        let code = units[index];
        if code == INLINE_TEXT_END {
            entries.push(DocumentTextMapEntry::new(
                start,
                index + 1,
                DocumentTextMapKind::InlineText,
                Some(selector),
                None,
                text,
            ));
            return index + 1;
        }

        if !is_control_boundary(code)
            && !is_invalid_scalar(code)
            && let Some(character) = char::from_u32(code as u32)
        {
            text.push(character);
        }
        index += 1;
    }

    entries.push(DocumentTextMapEntry::new(
        start,
        index,
        DocumentTextMapKind::InlineText,
        Some(selector),
        None,
        text,
    ));
    index
}

fn push_map_run(
    entries: &mut Vec<DocumentTextMapEntry>,
    run: &mut String,
    run_start: usize,
    run_end: usize,
) {
    if !run.is_empty() {
        entries.push(DocumentTextMapEntry::new(
            run_start,
            run_end,
            DocumentTextMapKind::TextRun,
            None,
            None,
            std::mem::take(run),
        ));
        run.clear();
    }
}

fn push_map_control(entries: &mut Vec<DocumentTextMapEntry>, index: usize, code: u16) {
    entries.push(DocumentTextMapEntry::new(
        index,
        index + 1,
        DocumentTextMapKind::ControlBoundary,
        None,
        Some(code),
        String::new(),
    ));
}

fn read_skipped_inline_segment(
    units: &[u16],
    start: usize,
) -> Option<(SkippedInlineTextSegment, usize)> {
    if start >= units.len() || units[start] != INLINE_TEXT_START {
        return None;
    }

    let context_start = start.saturating_sub(6);
    let context = units[context_start..start].to_vec();
    let mut text = String::new();
    let mut index = start + 1;

    while index < units.len() {
        if index - start > SKIPPED_INLINE_MAX_UNITS {
            return None;
        }

        let code = units[index];
        if code == INLINE_TEXT_END {
            let raw_bytes = units_to_be_bytes(&units[context_start..=index]);
            return Some((
                SkippedInlineTextSegment::new(context, text, raw_bytes),
                index + 1,
            ));
        }

        if !is_control_boundary(code)
            && !is_invalid_scalar(code)
            && let Some(character) = char::from_u32(code as u32)
        {
            text.push(character);
        }
        index += 1;
    }

    None
}

fn units_to_be_bytes(units: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(units.len() * 2);
    for unit in units {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    bytes
}

// Structural check for an incoming TEXT_RUN_MARKER (0x001f). A marker starts a text run only
// when ONE of the following holds:
//  1. No prose has been read yet (stream head) — synthetic head markers and the first content
//     run are always accepted.
//  2. It terminates a well-formed RFC 0009 `0x001c` record header
//     (`[..] [len](=i-3) 0x0000 [class](=i-1) 0x001f` where the opener/class/len triple at
//     `start = i + 1 - len` re-echoes consistently) — the record terminator doubles as the start
//     of the following run.
//  3. The immediately preceding unit is `0x001e` (inline text terminator) — the 0x001d inline
//     form is consumed by the inline reader, so a marker right after 0x001e is the following run.
//  4. A `0x001c` record opener appears within the previous 12 units — a record body can carry a
//     short (`len < 4`) or malformed footer; the run still starts once the record opened.
//  5. The stream is still within its first 16 units and no `0x001c`/`0x001f` precedes it.
//
// Any other `0x001f` is an isolated binary `31` value in the trailing style-table/metadata area
// (observed at word 76819 of 計画交通課パブコメ2025.jtd) and must not start a text run.
fn is_document_text_run_start(units: &[u16], index: usize, has_prose: bool) -> bool {
    if !has_prose {
        return true;
    }
    if rfc0009_record_footer_at(units, index) {
        return true;
    }
    if index > 0
        && (units[index - 1] == INLINE_TEXT_END || units[index - 1] == INLINE_TEXT_START)
    {
        return true;
    }
    if index >= 4 && units[index - 4] == INLINE_TEXT_END {
        return true;
    }
    if units[index.saturating_sub(12)..index].contains(&0x001c) {
        return true;
    }
    index <= 16 && !units[..index].iter().any(|&unit| unit == 0x001c || unit == 0x001f)
}

/// Returns true when the unit at `index` is a `0x001f` that forms a self-consistent RFC 0009
/// record footer terminator:
/// ```text
/// units[ index - 3 ] = len      (total record length in words, includes opener + footer + marker)
/// units[ index - 2 ] = 0x0000
/// units[ index - 1 ] = class    (a known RFC 0009 class code)
/// units[ index ]     = 0x001f   (marker under check)
/// units[ start ]     = 0x001c   (opener, start = index + 1 - len)
/// units[ start + 1 ] = class    (echo)
/// units[ start + 2 ] = len      (echo)
/// ```
/// The opener/class/len re-echo check is the structural proof that this `0x001f` is a record
/// terminator (hence the following run's start) rather than an isolated binary `31` value.
fn rfc0009_record_footer_at(units: &[u16], index: usize) -> bool {
    const KNOWN_CLASSES: &[u16] = &[
        RECORD_CLASS_INLINE_CONTEXT,
        RECORD_CLASS_PARAGRAPH_LINE,
        RECORD_CLASS_TABLE_SECTION_TRANSITION,
        RECORD_CLASS_TABLE_CELL,
    ];
    if index < 4 || units[index - 2] != 0x0000 {
        return false;
    }
    let class = units[index - 1];
    if !KNOWN_CLASSES.contains(&class) {
        return false;
    }
    let total_len = units[index - 3] as usize;
    if total_len < 4 || total_len > index + 1 {
        return false;
    }
    let start = index + 1 - total_len;
    units.get(start) == Some(&0x001c)
        && units.get(start + 1) == Some(&class)
        && units.get(start + 2) == Some(&(total_len as u16))
}

#[cfg(test)]
mod tests {
    use super::{
        DocumentTextElement, DocumentTextMapKind, DocumentTextRowHeaderPair,
        DocumentTextRowHeaderPairClassification, EMBEDDED_DOCUMENT_TEXT_PATH,
        SKIPPED_INLINE_MAX_UNITS, TEXT_RUN_MARKER, extract_document_text, map_document_text,
        parse_document_text, parse_document_text_row_headers, read_document_text_payload,
        read_document_text_stream, trim_trailing_exposed_controls, units_to_be_bytes,
    };
    use crate::compressed_document::is_just_compressed_document;
    use std::io::{Cursor, Write};
    use std::path::PathBuf;

    #[test]
    fn extracts_utf16be_runs_after_text_marker() {
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

        assert_eq!(extract_document_text(&bytes), "銀河鉄道\n");
    }

    #[test]
    fn ignores_bytes_before_text_marker() {
        let mut bytes = vec![0x53, 0x73, 0x6d, 0x67, 0x00, 0x10, 0x00, 0x20];
        bytes.extend_from_slice(&[0x00, 0x1f]);
        for unit in "本文".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }

        assert_eq!(extract_document_text(&bytes), "本文");
    }

    #[test]
    fn treats_c1_control_codes_as_boundaries() {
        let mut bytes = vec![0x00, 0x1f];
        for unit in "目次".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        bytes.extend_from_slice(&[0x00, 0x90]);
        for unit in "ignored".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }

        assert_eq!(extract_document_text(&bytes), "目次");
    }

    #[test]
    fn continues_text_after_row_delimiter_inside_text_run() {
        let mut bytes = vec![0x00, 0x1f, 0x00, 0x0e];
        for unit in "１，次の計算をしなさい\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(&mut bytes, &[0x001c]);

        assert_eq!(extract_document_text(&bytes), "１，次の計算をしなさい\n");

        let map = map_document_text(&bytes);
        assert_eq!(
            map.entries()[0].kind(),
            DocumentTextMapKind::ControlBoundary
        );
        assert_eq!(map.entries()[0].code(), Some(0x000e));
        assert_eq!(map.entries()[1].kind(), DocumentTextMapKind::TextRun);
        assert_eq!(map.entries()[1].unit_start(), 2);
        assert_eq!(map.entries()[1].text(), "１，次の計算をしなさい\n");
    }

    #[test]
    fn continues_text_after_page_break_control_inside_text_run() {
        let mut bytes = vec![0x00, 0x1f];
        for unit in "一、午后の授業".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(&mut bytes, &[super::DOCUMENT_TEXT_PAGE_BREAK_CONTROL]);
        for unit in "二、活版所\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(&mut bytes, &[0x001c]);

        assert_eq!(extract_document_text(&bytes), "一、午后の授業二、活版所\n");

        let map = map_document_text(&bytes);
        assert_eq!(map.entries()[0].kind(), DocumentTextMapKind::TextRun);
        assert_eq!(map.entries()[0].text(), "一、午后の授業");
        assert_eq!(
            map.entries()[1].kind(),
            DocumentTextMapKind::ControlBoundary
        );
        assert_eq!(
            map.entries()[1].code(),
            Some(super::DOCUMENT_TEXT_PAGE_BREAK_CONTROL)
        );
        assert_eq!(map.entries()[2].kind(), DocumentTextMapKind::TextRun);
        assert_eq!(map.entries()[2].text(), "二、活版所\n");
    }

    #[test]
    fn continues_text_after_inline_space_control_inside_text_run() {
        let mut bytes = vec![0x00, 0x1f];
        for unit in "ジョバンニは学校の門を".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(&mut bytes, &[super::DOCUMENT_TEXT_INLINE_SPACE_CONTROL]);
        for unit in "出るとき\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(&mut bytes, &[0x001c]);

        assert_eq!(
            extract_document_text(&bytes),
            "ジョバンニは学校の門を出るとき\n"
        );

        let map = map_document_text(&bytes);
        assert_eq!(map.entries()[0].kind(), DocumentTextMapKind::TextRun);
        assert_eq!(map.entries()[0].text(), "ジョバンニは学校の門を");
        assert_eq!(
            map.entries()[1].kind(),
            DocumentTextMapKind::ControlBoundary
        );
        assert_eq!(
            map.entries()[1].code(),
            Some(super::DOCUMENT_TEXT_INLINE_SPACE_CONTROL)
        );
        assert_eq!(map.entries()[2].kind(), DocumentTextMapKind::TextRun);
        assert_eq!(map.entries()[2].text(), "出るとき\n");
    }

    #[test]
    fn extracts_display_inline_segments_without_phonetic_annotations() {
        let mut bytes = vec![0x00, 0x1f];
        for unit in "一、".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(
            &mut bytes,
            &[0x001c, 0x0001, 0x0007, 0x0000, 0x0000, 0x0003, 0x001d],
        );
        for unit in "午后".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(&mut bytes, &[0x001e, 0x0005, 0x0000, 0x0001, 0x001f]);
        extend_units(
            &mut bytes,
            &[0x001c, 0x0001, 0x0007, 0x0000, 0x0001, 0x0082, 0x001d],
        );
        for unit in "ごご".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(&mut bytes, &[0x001e, 0x0005, 0x0000, 0x0001, 0x001f]);
        for unit in "の授業".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }

        assert_eq!(extract_document_text(&bytes), "一、午后の授業");

        let parsed = parse_document_text(&bytes);
        let skipped = parsed
            .elements()
            .iter()
            .find_map(|element| match element {
                DocumentTextElement::SkippedInlineText(segment) => Some(segment),
                _ => None,
            })
            .expect("phonetic annotation should be preserved as skipped inline text");
        assert_eq!(skipped.selector(), Some(0x0082));
        assert_eq!(skipped.text(), "ごご");
        assert!(!skipped.raw_bytes().is_empty());
    }

    #[test]
    fn parses_document_text_into_structured_elements() {
        let mut bytes = vec![0x00, 0x1f];
        for unit in "一、".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(
            &mut bytes,
            &[0x001c, 0x0001, 0x0007, 0x0000, 0x0000, 0x0003, 0x001d],
        );
        for unit in "午后".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(&mut bytes, &[0x001e, 0x0005, 0x0000, 0x0001, 0x001f]);
        for unit in "の授業".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }

        let parsed = parse_document_text(&bytes);

        assert_eq!(parsed.plain_text(), "一、午后の授業");
        assert_eq!(parsed.elements().len(), 4);
        assert_eq!(
            parsed.elements()[0],
            DocumentTextElement::TextRun("一、".into())
        );
        match &parsed.elements()[1] {
            DocumentTextElement::ControlBoundary(control) => {
                assert_eq!(control.code(), 0x001c);
            }
            other => panic!("expected control boundary, got {other:?}"),
        }
        match &parsed.elements()[2] {
            DocumentTextElement::InlineText(segment) => {
                assert_eq!(segment.selector(), 0x0003);
                assert_eq!(segment.text(), "午后");
            }
            other => panic!("expected inline text segment, got {other:?}"),
        }
        assert_eq!(
            parsed.elements()[3],
            DocumentTextElement::TextRun("の授業".into())
        );
    }

    #[test]
    fn maps_document_text_elements_to_byte_and_unit_ranges() {
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

        let map = map_document_text(&bytes);

        assert_eq!(map.entries().len(), 3);
        assert_eq!(map.entries()[0].kind(), DocumentTextMapKind::TextRun);
        assert_eq!(map.entries()[0].byte_start(), 10);
        assert_eq!(map.entries()[0].byte_end(), 14);
        assert_eq!(map.entries()[0].unit_start(), 5);
        assert_eq!(map.entries()[0].unit_end(), 7);
        assert_eq!(map.entries()[0].text(), "銀河");
        assert!(map.entries()[0].contains_byte_offset(10));
        assert!(map.entries()[0].contains_unit_offset(5));
        assert_eq!(
            map.entries()[1].kind(),
            DocumentTextMapKind::ControlBoundary
        );
        assert_eq!(map.entries()[1].code(), Some(0x001c));
        assert_eq!(map.entries()[2].text(), "鉄道\n");
    }

    #[test]
    fn extracts_template_placeholder_inline_segments() {
        let mut bytes = Vec::new();
        extend_units(
            &mut bytes,
            &[0x001c, 0x0001, 0x0007, 0x0000, 0x0000, 0x0001, 0x001d],
        );
        for unit in "○○○".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(&mut bytes, &[0x001e, 0x001f]);
        for unit in "賞".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }

        assert_eq!(extract_document_text(&bytes), "○○○賞");
    }

    #[test]
    fn skips_template_instruction_inline_segments() {
        let mut bytes = Vec::new();
        extend_units(
            &mut bytes,
            &[0x001c, 0x0001, 0x0007, 0x0000, 0x0001, 0x0000, 0x001d],
        );
        for unit in "名前を入力してください。".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(&mut bytes, &[0x001e, 0x001f]);
        for unit in "本文".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }

        assert_eq!(extract_document_text(&bytes), "本文");

        let parsed = parse_document_text(&bytes);
        let skipped = parsed
            .elements()
            .iter()
            .find_map(|element| match element {
                DocumentTextElement::SkippedInlineText(segment) => Some(segment),
                _ => None,
            })
            .expect("template instruction should be preserved as skipped inline text");
        assert_eq!(skipped.selector(), Some(0x0000));
        assert_eq!(skipped.text(), "名前を入力してください。");
    }

    #[test]
    fn does_not_consume_unbounded_skipped_inline_segments() {
        let mut bytes = Vec::new();
        extend_units(
            &mut bytes,
            &[0x001c, 0x0001, 0x0007, 0x0000, 0x0001, 0x0082, 0x001d],
        );
        for _ in 0..(SKIPPED_INLINE_MAX_UNITS + 2) {
            bytes.extend_from_slice(&0x3042_u16.to_be_bytes());
        }
        bytes.extend_from_slice(&[0x00, 0x1e]);

        let parsed = parse_document_text(&bytes);

        assert!(
            parsed
                .elements()
                .iter()
                .all(|element| { !matches!(element, DocumentTextElement::SkippedInlineText(_)) })
        );
        assert!(parsed.elements().iter().any(|element| matches!(
            element,
            DocumentTextElement::ControlBoundary(control) if control.code() == 0x001d
        )));
    }

    #[test]
    fn extracts_styled_body_text_inside_inline_start_end_boundaries() {
        // 宮沢賢治「銀河鉄道の夜」より:
        // 文字装飾コード（0x00a3...0x00a4...0x001d...0x001f）で囲まれた本文が
        // 誤ってスキップされず正常に抽出されることを検証
        let mut bytes = Vec::new();
        for unit in "先生は、黒板に吊した大きな黒い星座の図の、上から下へ白くけぶった銀河帯のようなところを指しながら、みんなに問をかけました。\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(
            &mut bytes,
            &[
                0x001c, 0x0010, 0x0011, 0x0000, 0x00a3, 0x0002, 0x0002, 0xffff, 0x00a4,
                0x0001, 0x001d, 0xffff, 0x0000, 0x0011, 0x0000, 0x0010, 0x001f,
            ],
        );
        for unit in "「ではみなさんは、そういうふうに川だと云われたり、乳の流れたあとだと云われたりしていたこのぼんやりと白いものがほんとうは何かご承知ですか。」".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        extend_units(
            &mut bytes,
            &[
                0x000a, 0x001c, 0x0010, 0x0011, 0x0000, 0x00a3, 0x0002, 0x0002, 0xffff,
                0x00a4, 0x0001, 0x001e,
            ],
        );
        extend_units(&mut bytes, &[0x001f]);
        for unit in "カムパネルラが手をあげました。".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }

        let extracted = extract_document_text(&bytes);
        assert!(extracted.contains("「ではみなさんは、そういうふうに川だと云われたり、乳の流れたあとだと云われたりしていたこのぼんやりと白いものがほんとうは何かご承知ですか。」"));
        assert!(extracted.contains("カムパネルラが手をあげました。"));
    }

    #[test]
    fn detects_just_compressed_document_payload() {
        assert!(is_just_compressed_document(
            b"\x26\0JustCompressedDocument\0payload"
        ));
        assert!(!is_just_compressed_document(b"DocumentText"));
    }

    #[test]
    fn reports_compressed_document_when_document_text_is_absent() {
        let bytes = cfb_with_stream(
            "/JSCompDocument",
            b"\x26\0JustCompressedDocument\0-lh5-\0payload",
        );
        let error = read_document_text_stream(&bytes).unwrap_err();

        assert!(error.to_string().contains("invalid data"));
    }

    #[test]
    fn reports_missing_document_text_without_known_compressed_payload() {
        let bytes = cfb_with_stream("/Other", b"payload");
        let error = read_document_text_stream(&bytes).unwrap_err();

        assert!(error.to_string().contains("not found"));
    }

    #[test]
    fn reads_embedded_document_text_when_named_stream_is_absent() {
        let mut embedded = b"prefix SsmgV.01".to_vec();
        embedded.extend_from_slice(&[0x00, 0x1f]);
        for unit in "Note".encode_utf16() {
            embedded.extend_from_slice(&unit.to_be_bytes());
        }
        let bytes = cfb_with_stream("/JSSlipObject1", &embedded);

        let payload = read_document_text_payload(&bytes).unwrap();

        assert_eq!(payload.source_name(), EMBEDDED_DOCUMENT_TEXT_PATH);
        assert_eq!(payload.text(), "Note");
        assert!(payload.bytes().starts_with(b"SsmgV.01"));
    }

    #[test]
    fn embedded_document_text_drops_implausible_noise_lines() {
        let mut embedded = b"SsmgV.01".to_vec();
        embedded.extend_from_slice(&[0x00, 0x1f]);
        for unit in "NoteĀ́āāāā蔭\u{f706}\r本文".encode_utf16() {
            embedded.extend_from_slice(&unit.to_be_bytes());
        }
        let bytes = cfb_with_stream("/JSSlipObject1", &embedded);

        let payload = read_document_text_payload(&bytes).unwrap();

        assert_eq!(payload.text(), "本文");
    }

    fn extend_units(bytes: &mut Vec<u8>, units: &[u16]) {
        for unit in units {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
    }

    #[test]
    fn parses_raw_text_segment_format_without_paragraph_markers() {
        // SsmgV.01 w[9]=0x0001 + TextV.01 + length + raw UTF-16BE text (no 0x001f markers)
        // This matches the te.jtd format where text is stored directly in a TextV.01 segment.
        let text_content: Vec<u16> = "te\nsto\nて".encode_utf16().collect();
        let length = text_content.len() as u16;
        let mut payload: Vec<u8> = Vec::new();
        // SsmgV.01 header (10 words): magic + 4 header words + segment-count=1
        let header: &[u16] = &[
            0x5373, 0x6d67, 0x562e, 0x3031, 0x0000, 0x0001, 0x0000, 0x0100, 0x0000, 0x0001,
        ];
        for w in header {
            payload.extend_from_slice(&w.to_be_bytes());
        }
        // TextV.01 segment name: 8 ASCII bytes → 4 big-endian u16 words (0x5465 0x7874 0x562e 0x3031)
        for w in &[0x5465_u16, 0x7874, 0x562e, 0x3031] {
            payload.extend_from_slice(&w.to_be_bytes());
        }
        // Length field: word[14]=0x0000, word[15]=length (matches te.jtd layout)
        payload.extend_from_slice(&0x0000_u16.to_be_bytes());
        payload.extend_from_slice(&length.to_be_bytes());
        // Text content
        for w in &text_content {
            payload.extend_from_slice(&w.to_be_bytes());
        }

        let parsed = parse_document_text(&payload);
        let text = parsed.plain_text();
        assert!(text.contains("te"), "should contain 'te' but got: {text:?}");
        assert!(
            text.contains("sto"),
            "should contain 'sto' but got: {text:?}"
        );
        assert!(text.contains('て'), "should contain 'て' but got: {text:?}");
    }

    // ---- markerless raw body (w[9] > 1): red tests for needs-doc-pdf-conversion ----
    //
    // 2026-09-22 の調査（rjtd-testdata/local-samples/gov-web-jtd/needs-doc-pdf-conversion/
    // REPORT.md）で判明した「マーカーレス raw テキスト型」のサイレント脱落。
    // 官公庁の原本本文は使わず、宮沢賢治「銀河鉄道の夜」（青空文庫、パブリックドメイン、
    // https://www.aozora.gr.jp/cards/000081/files/456_15050.html よりふりがなを除去）
    // を本文として合成する。
    //
    // レイアウト（REPORT.md と同一語義、word 単位）:
    //   w[0..9]   SsmgV.01 + ヘッダ + w[9]=内部セグメント数（本ケースでは >1）
    //   w[10..13] "TextV.01"
    //   w[14]     0x0000
    //   w[15]     本文長（word 数）
    //   w[16..]   本文（UTF-16BE 生・0x001f/0x001d/0x001c マーカー無し）
    //
    // 現行実装は raw パスのゲートを w[9]==0x0001 に限定しているため本文に到達できず、
    // plain_text() が空になる（期待: 本文全文）。修正案は REPORT.md 案1 参照。
    fn markerless_raw_payload(segment_count: u16, text: &str, tail: &[u16]) -> Vec<u8> {
        let text_content: Vec<u16> = text.encode_utf16().collect();
        let length = text_content.len() as u16;
        let mut payload: Vec<u8> = Vec::new();
        // SsmgV.01 header (10 words): magic + 4 header words + segment-count (2 words)
        extend_units(
            &mut payload,
            &[
                0x5373, 0x6d67, 0x562e, 0x3031, 0x0000, 0x0001, 0x0000, 0x0100, 0x0000,
                segment_count,
            ],
        );
        // TextV.01 segment name (4 words)
        extend_units(&mut payload, &[0x5465, 0x7874, 0x562e, 0x3031]);
        // Text span header: word[14]=0x0000, word[15]=本文長
        extend_units(&mut payload, &[0x0000, length]);
        // マーカーレスの UTF-16BE 本文
        extend_units(&mut payload, &text_content);
        extend_units(&mut payload, tail);
        payload
    }

    const GALAXY_P1: &str = "「ではみなさんは、そういうふうに川だと云われたり、乳の流れたあとだと云われたりしていたこのぼんやりと白いものがほんとうは何かご承知ですか。」先生は、黒板に吊した大きな黒い星座の図の、上から下へ白くけぶった銀河帯のようなところを指しながら、みんなに問をかけました。";

    const GALAXY_P2: &str = "カムパネルラが手をあげました。それから四五人手をあげました。ジョバンニも手をあげようとして、急いでそのままやめました。たしかにあれがみんな星だと、いつか雑誌で読んだのでしたが、このごろはジョバンニはまるで毎日教室でもねむく、本を読むひまも読む本もないので、なんだかどんなこともよくわからないという気持ちがするのでした。";

    const GALAXY_P3: &str = "ところが先生は早くもそれを見つけたのでした。";

    const GALAXY_P4: &str = "「ですからもしもこの天の川がほんとうに川だと考えるなら、その一つ一つの小さな星はみんなその川のそこの砂や砂利の粒にもあたるわけです。」";

    #[test]
    fn markerless_raw_body_recovers_text_when_segment_count_exceeds_one() {
        // 官公庁原本4件の直撃型（w[9]=0x0003..0x000b）を模す合成ペイロード。
        // 本文領域にマーカーが一切無く、raw ゲート（w[9]==1）を通過できないため
        // 現行実装では空文字になる。期待: 改行を含む本文全文の完全一致。
        let text = format!("{GALAXY_P1}\n{GALAXY_P2}");
        let payload = markerless_raw_payload(0x0004, &text, &[]);

        let parsed = parse_document_text(&payload);

        assert_eq!(
            parsed.plain_text(),
            text,
            "マーカーレス raw 本文（銀河鉄道の夜）が復元できない"
        );
    }

    #[test]
    fn markerless_raw_body_ignores_stray_run_marker_beyond_text_span() {
        // env-youshi.jtd 型: 本文領域 [16, 16+本文長) にマーカーは無く、
        // span 終端を過ぎた余白領域に孤立した 0x001f と末尾ノイズが存在する。
        // 孤立マーカーに惑わされず、span 内の本文だけを復元すること。
        let text = format!("{GALAXY_P3}\n{GALAXY_P4}");
        let tail: &[u16] = &[0xffff, 0xffff, 0x0000, 0x000a, 0x001f, 0x3000, 0x74b0];
        let payload = markerless_raw_payload(0x000b, &text, tail);

        let parsed = parse_document_text(&payload);

        assert_eq!(
            parsed.plain_text(),
            text,
            "span 外の孤立 0x001f があっても本文全文を復元しなければならない"
        );
    }

    // ---- mixed raw prologue (raw 前置き + マーカー本文): red tests for PARTIAL-LOSS-REPORT.md P1 ----
    //
    // 2026-09-23 の調査（rjtd-testdata/local-samples/gov-web-jtd/PARTIAL-LOSS-REPORT.md P1）で
    // 判明した「混在型」のサイレント脱落。TextV.01 セグメント内で最初のマーカー語
    // （0x001c/0x001d/0x001f）の位置 m より手前に UTF-16BE 生の前置き本文が置かれ、
    // 後続がマーカー型本文になっている文書が存在する（maff-tenpu02-238 等 33件）。
    // 現行実装は通常パスを reading_text=false で走査するため前置き領域の語を丸ごと
    // 捨て、最初の run マーカー以降だけを抽出する。修正案は同レポート P1（プロローグ
    // raw デコード）参照。
    //
    // 本文は官公庁原本の文言を使わず、宮沢賢治「銀河鉄道の夜」（青空文庫、
    // パブリックドメイン、https://www.aozora.gr.jp/cards/000081/files/456_15050.html
    // よりふりがなを除去）を合成する。
    //
    // レイアウト（REPORT.md と同一語義、word 単位）:
    //   w[0..9]   SsmgV.01 + ヘッダ + w[9]=内部セグメント数（本ケースでは >1）
    //   w[10..13] "TextV.01"
    //   w[14]     0x0000
    //   w[15]     本文 span 長（word 数。前置き＋マーカー部を含む）
    //   w[16..m]  前置き（UTF-16BE 生テキスト・マーカー無し）
    //   w[m..]    マーカー型本文（0x001c…0x001f レコード＋run テキスト）
    const TEXTV01_NAME_WORDS: &[u16] = &[0x5465, 0x7874, 0x562e, 0x3031]; // "TextV.01"
    const SSMGV_HEAD_WORDS: &[u16] =
        &[0x5373, 0x6d67, 0x562e, 0x3031, 0x0000, 0x0001, 0x0000, 0x0100, 0x0000];

    // RFC 0009 の自己整合なレコード終端（len=6・class=0x0010）。0x001f の直後に
    // run テキストが続く構造（is_document_text_run_start 条件2・3通过）。
    const RUN_RECORD_FOOTER_WORDS: &[u16] = &[0x001c, 0x0010, 0x0006, 0x0000, 0x0010, 0x001f];

    fn mixed_prologue_payload(segment_count: u16, prologue: &[u16], body: &[u16]) -> Vec<u8> {
        let length = (prologue.len() + body.len()) as u16;
        let mut payload: Vec<u8> = Vec::new();
        extend_units(&mut payload, SSMGV_HEAD_WORDS);
        extend_units(&mut payload, &[segment_count]);
        extend_units(&mut payload, TEXTV01_NAME_WORDS);
        extend_units(&mut payload, &[0x0000, length]);
        extend_units(&mut payload, prologue);
        extend_units(&mut payload, body);
        payload
    }

    fn utf16_units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    const GALAXY_P5: &str = "「ジョバンニさん。あなたはわかっているのでしょう。」";
    const GALAXY_P6: &str =
        "やっぱり星だとジョバンニは思いましたがこんどもすぐに答えることができませんでした。";
    const GALAXY_P7: &str = "「ああきっと一緒だよ。お母さん、窓をしめて置こうか。」";
    const GALAXY_P8: &str = "「ああ行っておいで。川へははいらないでね。」";
    const GALAXY_P9: &str = "ジョバンニは窓をあけました。";
    const GALAXY_P10_NOISE: &str = "銀河ステーションで、もらったんだ。";
    const GALAXY_P11: &str = "「大きな望遠鏡で銀河をよっく調べると銀河は大体何でしょう。」";
    const GALAXY_P12: &str =
        "ジョバンニは、ばっと胸がつめたくなり、そこら中きぃんと鳴るように思いました。";
    const GALAXY_P13: &str = "「ああ、お前さきにおあがり。あたしはまだほしくないんだから。」";

    #[test]
    fn mixed_raw_prologue_recovers_text_before_first_run_marker() {
        // 混在型: 前置き raw テキスト＋0x001c…0x001f マーカー本文。
        // 現行実装は前置きを捨てて GALAXY_P6+GALAXY_P7 だけになる。
        // 期待: 前置き込みの全文（maff-tenpu02-238 の宛先ブロック脱落の再現型）。
        let prologue = utf16_units(&format!("{GALAXY_P5}\n"));
        let mut body: Vec<u16> = Vec::new();
        body.extend_from_slice(RUN_RECORD_FOOTER_WORDS);
        body.extend(utf16_units(GALAXY_P6));
        body.extend_from_slice(RUN_RECORD_FOOTER_WORDS);
        body.extend(utf16_units(GALAXY_P7));
        let payload = mixed_prologue_payload(0x0003, &prologue, &body);

        let parsed = parse_document_text(&payload);
        let expected = format!("{GALAXY_P5}\n{GALAXY_P6}{GALAXY_P7}");

        assert_eq!(
            parsed.plain_text(),
            expected,
            "最初の run マーカーより手前の raw 前置き（銀河鉄道の夜）が復元できない"
        );
        assert_eq!(
            parsed.elements().first(),
            Some(&DocumentTextElement::TextRun(format!("{GALAXY_P5}\n"))),
            "前置きは後続マーカー本文より前の TextRun として emit しなければならない"
        );

        // map_document_text にも同一レイアウトをミラーすること（レポート P1 方針2）。
        let map = map_document_text(&payload);
        assert_eq!(map.entries().first().map(|e| e.unit_start()), Some(16));
        assert_eq!(map.entries().first().map(|e| e.byte_start()), Some(32));
        assert_eq!(
            map.entries().first().map(|e| e.text()),
            Some(format!("{GALAXY_P5}\n").as_str()),
            "プロローグは unit 16（byte 32）から始まるエントリとしてマップされるべき"
        );
        let mapped: String = map.entries().iter().map(|e| e.text()).collect();
        assert_eq!(mapped, expected);
    }

    #[test]
    fn mixed_raw_prologue_keeps_line_breaks_and_stops_at_control_boundary() {
        // 前置きデコードの意味論: CR/LF は行区切りとして保持、0x0000 は連続
        // パディングとしてスキップ、それ以外の制御境界（0x0019）で読みを打ち切る。
        // 打ち切り以降のノイズ語（銀河ステーションで、もらったんだ。）は出力しない。
        let mut prologue: Vec<u16> = utf16_units(&format!("{GALAXY_P8}\n"));
        prologue.extend_from_slice(&[0x0000, 0x0000]);
        prologue.extend(utf16_units(GALAXY_P9));
        prologue.push(0x0019);
        prologue.extend(utf16_units(GALAXY_P10_NOISE));
        let mut body: Vec<u16> = Vec::new();
        body.extend_from_slice(RUN_RECORD_FOOTER_WORDS);
        body.extend(utf16_units(GALAXY_P11));
        let payload = mixed_prologue_payload(0x0003, &prologue, &body);

        let parsed = parse_document_text(&payload);
        let expected = format!("{GALAXY_P8}\n{GALAXY_P9}{GALAXY_P11}");

        assert_eq!(
            parsed.plain_text(),
            expected,
            "改行保持・ゼロパディングスキップ・制御境界打ち切りが守られていない"
        );
        assert!(
            !parsed.plain_text().contains(GALAXY_P10_NOISE),
            "制御境界 0x0019 より後ろのノイズ語を出力してはならない"
        );
    }

    #[test]
    fn mixed_raw_prologue_empty_span_keeps_output_unchanged() {
        // リグレッションガード: 正常マーカー型（span が語 16 からちょうど 0x001c で
        // 始まり前置き領域が空）では出力を一切変えないこと。
        // コーパス 236件中 174件がこれに相当（empty_prologue=174）。
        let mut body: Vec<u16> = Vec::new();
        body.extend_from_slice(RUN_RECORD_FOOTER_WORDS);
        body.extend(utf16_units(GALAXY_P12));
        body.extend_from_slice(RUN_RECORD_FOOTER_WORDS);
        body.extend(utf16_units(GALAXY_P13));
        let payload = mixed_prologue_payload(0x0003, &[], &body);

        let parsed = parse_document_text(&payload);
        let expected = format!("{GALAXY_P12}{GALAXY_P13}");

        assert_eq!(
            parsed.plain_text(),
            expected,
            "前置き空のマーカー型で出力が変化してはならない（リグレッション）"
        );
        let map = map_document_text(&payload);
        assert_eq!(
            map.entries().first().map(|e| e.unit_start()),
            Some(22),
            "前置き空では最初のテキストエントリが語 22（最初の run テキスト）であること"
        );
        let mapped: String = map.entries().iter().map(|e| e.text()).collect();
        assert_eq!(mapped, expected);
    }

    #[test]
    fn parses_row_header_inventory_as_state_run_pairs() {
        let payload = row_header_record_bytes(
            [0x0000, 0x008f, 0x0011, 0x0118, 0x0000, 0x0050],
            &[
                0x0008, 0x0003, 0x0013, 0x0000, 0x0000, 0x0046, 0x0013, 0x0000, 0x0000, 0x0017,
                0x0021, 0x0000, 0x0000, 0x0060, 0xffff, 0x0000,
            ],
        );

        let records = parse_document_text_row_headers(&payload);

        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.source_span().unit_start(), 0);
        assert_eq!(record.source_span().byte_start(), 0);
        assert_eq!(record.total_len_words(), 29);
        assert_eq!(record.fixed_fields().subtype(), 0x008f);
        assert_eq!(record.fixed_fields().grid_extent(), 0x0118);
        assert_eq!(
            record.fixed_fields().raw_words(),
            &[0x0000, 0x008f, 0x0011, 0x0118, 0x0000, 0x0050]
        );
        assert_eq!(
            record.raw_payload_words(),
            &[
                0x0008, 0x0003, 0x0013, 0x0000, 0x0000, 0x0046, 0x0013, 0x0000, 0x0000, 0x0017,
                0x0021, 0x0000, 0x0000, 0x0060, 0xffff, 0x0000,
            ]
        );
        assert_eq!(
            record
                .pairs()
                .iter()
                .map(|pair| (pair.state_code(), pair.run_length()))
                .collect::<Vec<_>>(),
            vec![
                (0x0008, 0x0003),
                (0x0013, 0x0000),
                (0x0000, 0x0046),
                (0x0013, 0x0000),
                (0x0000, 0x0017),
                (0x0021, 0x0000),
                (0x0000, 0x0060),
            ]
        );
        assert_eq!(record.raw_tail_words(), &[0xffff, 0x0000]);
        assert!(!record.tail_truncated());
        assert!(record.geometry_complete());
        assert_eq!(record.pairs()[0].start_unit(), 81);
        assert_eq!(record.pairs()[0].end_unit(), 84);
        assert_eq!(
            record.pairs()[0].classification(),
            DocumentTextRowHeaderPairClassification::NonBlankRun
        );
        assert_eq!(record.pairs()[1].start_unit(), 85);
        assert_eq!(record.pairs()[1].end_unit(), 85);
        assert_eq!(
            record.pairs()[1].classification(),
            DocumentTextRowHeaderPairClassification::Junction
        );
        assert_eq!(record.pairs()[2].start_unit(), 86);
        assert_eq!(record.pairs()[2].end_unit(), 156);
        assert_eq!(
            record.pairs()[2].classification(),
            DocumentTextRowHeaderPairClassification::BlankRun
        );
        assert!(
            record
                .pairs()
                .iter()
                .all(DocumentTextRowHeaderPair::geometry_complete)
        );
    }

    #[test]
    fn preserves_odd_row_header_tail_words_losslessly() {
        let odd_payload = row_header_record_bytes(
            [0x0000, 0x008f, 0x000f, 0x0118, 0x0000, 0x0020],
            &[0x0023, 0x0000, 0x0000, 0x0020, 0x7777],
        );

        let records = parse_document_text_row_headers(&odd_payload);

        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(
            record
                .pairs()
                .iter()
                .map(|pair| (pair.state_code(), pair.run_length()))
                .collect::<Vec<_>>(),
            vec![(0x0023, 0x0000), (0x0000, 0x0020)]
        );
        assert_eq!(record.raw_tail_words(), &[0x7777]);
        assert!(record.tail_truncated());
        assert!(!record.geometry_complete());
    }

    #[test]
    #[ignore = "requires local shanai_lan sample"]
    fn inventories_shanai_lan_row_headers_from_local_sample() {
        let path = repo_root()
            .join("rjtd-testdata/local-samples")
            .join("ichitaro-20030315134715-success-001-success_data-shanai_lan.jtd");
        let bytes = std::fs::read(&path).expect("read local shanai_lan sample");
        let payload = read_document_text_payload(&bytes).expect("read DocumentText payload");

        let records = parse_document_text_row_headers(payload.bytes());

        let g21 = records
            .iter()
            .find(|record| record.source_span().unit_start() == 3989)
            .expect("g21 row header at unit 3989");
        assert_eq!(g21.fixed_fields().subtype(), 0x008f);
        assert_eq!(g21.fixed_fields().grid_extent(), 280);
        assert_eq!(
            g21.pairs()
                .iter()
                .map(|pair| (pair.state_code(), pair.run_length()))
                .collect::<Vec<_>>(),
            vec![
                (0x0023, 0x0000),
                (0x0000, 0x0020),
                (0x0023, 0x0000),
                (0x0000, 0x0023),
                (0x0013, 0x0000),
                (0x0000, 0x0003),
                (0x002b, 0x0000),
                (0x0008, 0x001a),
                (0x0000, 0x0026),
                (0x0013, 0x0000),
                (0x0000, 0x0017),
                (0x0023, 0x0000),
                (0x0000, 0x0060),
            ]
        );
        assert_eq!(g21.raw_tail_words(), &[0xffff, 0x0000]);
        assert!(g21.geometry_complete());
        assert_eq!(g21.pairs()[6].start_unit(), 90);
        assert_eq!(g21.pairs()[6].end_unit(), 90);
        assert_eq!(g21.pairs()[7].start_unit(), 91);
        assert_eq!(g21.pairs()[7].end_unit(), 117);
        assert_eq!(
            g21.pairs()[7].classification(),
            DocumentTextRowHeaderPairClassification::NonBlankRun
        );

        let g31 = records
            .iter()
            .find(|record| record.source_span().unit_start() == 5537)
            .expect("g31 row header at unit 5537");
        assert_eq!(g31.fixed_fields().subtype(), 0x008f);
        assert_eq!(g31.fixed_fields().w8(), 80);
        assert_eq!(
            g31.pairs()
                .iter()
                .map(|pair| (pair.state_code(), pair.run_length()))
                .collect::<Vec<_>>(),
            vec![
                (0x0008, 0x0003),
                (0x0013, 0x0000),
                (0x0000, 0x0046),
                (0x0013, 0x0000),
                (0x0000, 0x0017),
                (0x0021, 0x0000),
                (0x0000, 0x0060),
            ]
        );
        assert_eq!(g31.raw_tail_words(), &[0xffff, 0x0000]);
        assert!(g31.geometry_complete());
        assert_eq!(g31.pairs()[0].start_unit(), 81);
        assert_eq!(g31.pairs()[0].end_unit(), 84);
        assert_eq!(g31.pairs()[1].start_unit(), 85);
        assert_eq!(g31.pairs()[1].end_unit(), 85);
        assert_eq!(g31.pairs()[2].start_unit(), 86);
        assert_eq!(g31.pairs()[2].end_unit(), 156);
    }

    fn row_header_record_bytes(fixed_words: [u16; 6], payload_words: &[u16]) -> Vec<u8> {
        let total_len_words = (3 + fixed_words.len() + payload_words.len() + 4) as u16;
        let mut words = vec![0x001c, 0x0010, total_len_words];
        words.extend_from_slice(&fixed_words);
        words.extend_from_slice(payload_words);
        words.extend_from_slice(&[total_len_words, 0x0000, 0x0010, 0x001f]);
        units_to_be_bytes(&words)
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .and_then(|path| path.parent())
            .expect("repo root")
            .to_path_buf()
    }

    fn cfb_with_stream(path: &str, payload: &[u8]) -> Vec<u8> {
        let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        compound
            .create_stream(path)
            .unwrap()
            .write_all(payload)
            .unwrap();
        compound.into_inner().into_inner()
    }

    #[test]
    fn bounds_document_text_parsing_to_content_unit_count() {
        // Night on the Galactic Railroad (Miyazawa Kenji) via Aozora Bunko
        let body_text = "一、午后の授業\n「ではみなさんは、そういうふうに川だと云われたり、乳の流れたあとだと云われたりしていたこのぼんやりと白いものがほんとうは何かご承知ですか。」";
        let body_units: Vec<u16> = body_text.encode_utf16().collect();
        // 1 unit for TEXT_RUN_MARKER (0x001f) + body_units.len()
        let content_unit_count = (1 + body_units.len()) as u32;

        let mut data = Vec::new();
        // SsmgV.01 header (10 words)
        data.extend_from_slice(b"SsmgV.01");
        extend_units(&mut data, &[0x0000, 0x0001, 0x0000, 0x0100, 0x0000, 0x0027]);
        // TextV.01 segment name (4 words)
        data.extend_from_slice(b"TextV.01");
        // Content unit count (2 words / u32-be)
        data.extend_from_slice(&content_unit_count.to_be_bytes());

        // Body text run: marker + units
        extend_units(&mut data, &[super::TEXT_RUN_MARKER]);
        extend_units(&mut data, &body_units);

        // Trailing garbage beyond content_unit_count (e.g. style section or edit residue)
        // containing a duplicate/extra text run marker and different text.
        let trailing_garbage = "「大きな望遠鏡で銀河をよっく調べると銀河は大体何でしょう。」";
        let garbage_units: Vec<u16> = trailing_garbage.encode_utf16().collect();
        extend_units(&mut data, &[0x0000, 0x001c, super::TEXT_RUN_MARKER]);
        extend_units(&mut data, &garbage_units);

        let parsed = parse_document_text(&data);
        let plain = parsed.plain_text();
        assert!(
            plain.contains("一、午后の授業"),
            "expected body text, got: {plain}"
        );
        assert!(
            !plain.contains("大きな望遠鏡で銀河をよっく調べると"),
            "trailing garbage beyond content_unit_count should not be parsed as body text, got: {plain}"
        );

        let map = map_document_text(&data);
        for entry in map.entries() {
            assert!(
                entry.unit_end() <= super::TEXT_CONTENT_HEADER_WORDS + content_unit_count as usize,
                "map entry end {} exceeds unit limit {}",
                entry.unit_end(),
                super::TEXT_CONTENT_HEADER_WORDS + content_unit_count as usize
            );
        }
    }

    #[test]
    fn trims_trailing_exposed_terminal_controls_from_plain_text() {
        // Unit tests must not embed thesis text; using an Aozora Bunko "Night on the
        // Galactic Railroad" excerpt instead (per plan). 夜 is a real prose character.
        let body = "ではみなさんは、そういうふうに川だと云われたり、乳の流れたあとだと云われたりしていたこのぼんやりと白いものがほんとうは何かご承知ですか。";
        let mut content_units: Vec<u16> = body.encode_utf16().collect();
        // Terminal exposed controls appended at the end of the DocumentText stream:
        // 0xFE14 (︔) and 0x0490 (Ґ), plus 0x0400 (unassigned) as leaking record data.
        for unit in [0xFE14u16, 0x0400, 0x0490] {
            content_units.push(unit);
        }

        assert_eq!(
            trim_trailing_exposed_controls("ジョバンニ\u{FE14}\u{0400}\u{0490}"),
            "ジョバンニ"
        );
        assert_eq!(trim_trailing_exposed_controls("ジョバンニ\u{FE14}"), "ジョバンニ");
        assert_eq!(trim_trailing_exposed_controls("ジョバンニ\u{0490}"), "ジョバンニ");
        // Whitespace immediately before the controls (as in `0.475 ︔`) is trimmed too.
        assert_eq!(trim_trailing_exposed_controls("0.475 \u{FE14}\u{0490}"), "0.475");
        // Whitespace after the controls is part of the trailing region.
        assert_eq!(trim_trailing_exposed_controls("ジョバンニ\u{FE14}  "), "ジョバンニ");
        // A trailing run of whitespace alone is a legitimate line terminator: kept.
        assert_eq!(trim_trailing_exposed_controls("ジョバンニ\n\n"), "ジョバンニ\n\n");
        // No exposed controls at all: text is unchanged.
        assert_eq!(trim_trailing_exposed_controls(body), body);

        let mut data = b"SsmgV.01".to_vec();
        extend_units(&mut data, &[0x0000, 0x0001, 0x0000, 0x0100, 0x0000, 0x0027]);
        data.extend_from_slice(b"TextV.01");
        data.extend_from_slice(&((1 + content_units.len()) as u32).to_be_bytes());
        extend_units(&mut data, &[TEXT_RUN_MARKER]);
        extend_units(&mut data, &content_units);

        let parsed = parse_document_text(&data);
        let plain = parsed.plain_text();
        assert!(
            plain.ends_with(body),
            "expected clean trailing body text, got: {plain:?}"
        );
        assert!(
            !plain.contains('\u{FE14}') && !plain.contains('\u{0490}'),
            "exposed terminal controls should be trimmed, got: {plain:?}"
        );
    }

    #[test]
    fn keeps_mid_text_exposed_records_but_only_trims_the_final_sequence() {
        // 0x0400 / 0xFE14 appearing mid-stream (inside table/formula data) are out of
        // scope for the trailing trim; only the terminal sequence at the very end is removed.
        let mut bytes = b"SsmgV.01".to_vec();
        bytes.extend_from_slice(&[0x00, 0x1f]);
        for unit in "銀河".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        bytes.extend_from_slice(&0x0400u16.to_be_bytes());
        for unit in "鉄道".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        // Final exposed terminal sequence.
        for unit in [0xFE14u16, 0x0490] {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }

        let parsed = parse_document_text(&bytes);
        let plain = parsed.plain_text();
        assert_eq!(plain, "銀河\u{0400}鉄道");
    }

    #[test]
    fn parses_document_text_ignores_isolated_text_marker_in_trailing_metadata() {
        // A trailing 0x001f appearing in metadata/footer without an RFC 0009 record header
        // should not start a text run, structurally preventing trailing binary data from leaking as prose.
        let mut bytes = b"SsmgV.01".to_vec();
        bytes.extend_from_slice(&[0x00, 0x1f]);
        for unit in "ジョバンニ".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        // Terminal row delimiter / end of text run
        bytes.extend_from_slice(&0x000eu16.to_be_bytes());
        bytes.extend_from_slice(&0x0000u16.to_be_bytes());
        // Trailing metadata with non-record padding followed by an isolated 0x001f marker
        bytes.extend_from_slice(&[0x12, 0x34, 0x56, 0x78]);
        bytes.extend_from_slice(&0x001fu16.to_be_bytes());
        for unit in [0xfe01u16, 0x0200, 0x0302, 0x0200, 0x03ff, 0x0000] {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }

        let parsed = parse_document_text(&bytes);
        let plain = parsed.plain_text();
        assert_eq!(plain.trim(), "ジョバンニ");
        assert!(
            !plain.contains('\u{FE01}')
                && !plain.contains('\u{0200}')
                && !plain.contains('\u{0302}')
                && !plain.contains('\u{03FF}'),
            "isolated marker in trailing metadata should not produce text elements, got: {plain:?}"
        );
    }

    #[test]
    fn restores_heading_and_itemization_numbering_from_paragraph_headers() {
        let mut content = Vec::new();

        // Heading Level 1 (Chapter): 午后の授業
        content.extend_from_slice(&[
            0x001c, 0x0010, 0x0011, 0x0000, 0x00a3, 0x0002, 0x0001, 0xffff, 0x0050, 0x0002,
            0x0001, 0xffff, 0x0000, 0x0011, 0x0000, 0x0010, 0x001f,
        ]);
        content.extend("午后の授業\n".encode_utf16());

        // Heading Level 2 (Section): 星座の図
        content.extend_from_slice(&[
            0x001c, 0x0010, 0x0011, 0x0000, 0x00a3, 0x0002, 0x0001, 0xffff, 0x0050, 0x0002,
            0x0002, 0xffff, 0x0000, 0x0011, 0x0000, 0x0010, 0x001f,
        ]);
        content.extend("星座の図\n".encode_utf16());

        // Heading Level 3 (Subsection): 銀河の巨きな星
        content.extend_from_slice(&[
            0x001c, 0x0010, 0x0011, 0x0000, 0x00a3, 0x0002, 0x0001, 0xffff, 0x0050, 0x0002,
            0x0003, 0xffff, 0x0000, 0x0011, 0x0000, 0x0010, 0x001f,
        ]);
        content.extend("銀河の巨きな星\n".encode_utf16());

        // Itemization Style 2, item 1 (restart)
        content.extend_from_slice(&[
            0x001c, 0x0010, 0x0011, 0x0000, 0x00a3, 0x0002, 0x0002, 0x7fff, 0x00a4, 0x0001,
            0x001d, 0xffff, 0x0000, 0x0011, 0x0000, 0x0010, 0x001f,
        ]);
        content.extend("ジョバンニは手をあげようとして、急いでそれをやめました。\n".encode_utf16());

        // Itemization Style 2, item 2
        content.extend_from_slice(&[
            0x001c, 0x0010, 0x0011, 0x0000, 0x00a3, 0x0002, 0x0002, 0xffff, 0x00a4, 0x0001,
            0x001d, 0xffff, 0x0000, 0x0011, 0x0000, 0x0010, 0x001f,
        ]);
        content.extend("カムパネルラが手をあげました。\n".encode_utf16());

        // Circled numbers Style 8: level 1
        content.extend_from_slice(&[
            0x001c, 0x0010, 0x0011, 0x0000, 0x00a3, 0x0002, 0x0008, 0xffff, 0x0050, 0x0002,
            0x0001, 0xffff, 0x0000, 0x0011, 0x0000, 0x0010, 0x001f,
        ]);
        content.extend("カムパネルラ\n".encode_utf16());

        // Circled numbers Style 8: level 2
        content.extend_from_slice(&[
            0x001c, 0x0010, 0x0011, 0x0000, 0x00a3, 0x0002, 0x0008, 0xffff, 0x0050, 0x0002,
            0x0002, 0xffff, 0x0000, 0x0011, 0x0000, 0x0010, 0x001f,
        ]);
        content.extend("ジョバンニ\n".encode_utf16());

        let content_unit_count = content.len() as u32;
        let mut data = Vec::new();
        // SsmgV.01 header
        data.extend_from_slice(b"SsmgV.01");
        extend_units(&mut data, &[0x0000, 0x0001, 0x0000, 0x0100, 0x0000, 0x0027]);
        data.extend_from_slice(b"TextV.01");
        data.extend_from_slice(&content_unit_count.to_be_bytes());
        extend_units(&mut data, &content);

        let parsed = parse_document_text(&data);
        let plain = parsed.plain_text();

        assert!(
            plain.contains("第1章 午后の授業"),
            "expected chapter 1 prefix, got: {plain}"
        );
        assert!(
            plain.contains("1.1 星座の図"),
            "expected section 1.1 prefix, got: {plain}"
        );
        assert!(
            plain.contains("1.1.1 銀河の巨きな星"),
            "expected subsection 1.1.1 prefix, got: {plain}"
        );
        assert!(
            plain.contains("(1) ジョバンニは手をあげようとして、急いでそれをやめました。"),
            "expected itemization (1) prefix, got: {plain}"
        );
        assert!(
            plain.contains("(2) カムパネルラが手をあげました。"),
            "expected itemization (2) prefix, got: {plain}"
        );
        assert!(
            plain.contains("① カムパネルラ"),
            "expected circled number ① prefix, got: {plain}"
        );
        assert!(
            plain.contains("①-1 ジョバンニ"),
            "expected circled number ①-1 prefix, got: {plain}"
        );
    }
}
