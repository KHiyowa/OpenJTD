// /Header（ヘッダ・フッタ本文）ストリームのレコード reader（PARTIAL-LOSS-REPORT.md 付録2
// 実装方針 P-H＋F）。
//
// 観測レイアウト（実データ 243 件の機械走査で確定・UTF-16BE word 単位・
// tests/streams/header_text.rs の合成フィクスチャと同一構造）:
//   w[0..9]      SsmgV.01 ヘッダ（w[5] = 0x0001 等・w[9] = セグメント数）
//   w[10] 以降   セグメントスロット列（語ピッチ 128 word = 256 byte）:
//     "TextV.01"（4語） + [0x0000, span 長（語数）]（2語） + span 内容 + 0x0000 パディング
//     または空の "TCntV.01"（[0x5443, 0x6e74, 0x562e, 0x3031] + 0x0000, 0x0000、
//     自動ページ番号連番キャッシュ・常に空 → 読まない）
//   ストリーム末尾: 位置/所有テーブル列（0x001b・0x005d 等の数値語）。
//     テキスト復元には絶対に読めない。
//
// span 長はスロットピッチ 128 を超えて後続スロットへオーバーフローしうる
//（実測 maff-tenpu02-200: span 長 211 語）ため、スロット番号の算術では走査せず、
// w[10] 以降を "TextV.01" マジック 4 語出現位置で走査する。
//
// span 内容の種別:
//   - raw（0x001c/0x001d/0x001f マーカー無し）: 全文 UTF-16BE 生
//     （0x0000 パディングスキップ・CR/LF 保持・制御境界で打ち切り）
//   - マーカー型: 先頭語 0x001c（直後にスタイル語ジャンク 0x0010/0x0014/0x0024/
//     0x0025/0xffff 等）+ 0x001f run マーカーの後ろに run 本文（末尾に 0x000a が典型的）
//
// 復元テキストの cat / export-txt 出力契約（crates/rjtd-cli run_cat・別タスク）:
//   span 1 個 = 1 行（末尾 CR/LF 除去済み）、ストリーム出現順で各行を先頭行として
//   本文の前に置き、本文と空行 1 行で区切る（Tika 準拠）。
//   F 裁定（slim/text-only）: raw 経路のデコード結果に限り `?` をページ番号
//   "1"（先頭ページ）に解決する。プレースホルダ `?` を含む raw span の
//   デコード結果は観測上 `"- ? -"` のみで誤変換リスクはなし。
//
// span 復元は P1 のプロローグ raw デコード機構（document_text::decode_prologue_units /
// first_text_marker）と通常のマーカー走査（document_text::parse_document_text）を
// 再利用する。

use crate::container::read_cfb_stream;
use crate::document_text::{decode_prologue_units, first_text_marker, parse_document_text};
use crate::{Error, Result};

pub const HEADER_TEXT_PATH: &str = "/Header";
const HEADER_MAGIC: &[u8; 8] = b"SsmgV.01";
const TEXT_SEGMENT_NAME: [u16; 4] = [0x5465, 0x7874, 0x562e, 0x3031]; // "TextV.01"
const HEADER_COUNT_WORDS: usize = 10; // SsmgV.01 (4) + header (4) + slot-count (2)
const TEXT_SEGMENT_NAME_WORDS: usize = 4;
const HEADER_SLOT_HEAD_WORDS: usize = 2; // [0x0000, span 長]

/// /Header 内の復元テキスト（span をストリーム順に 1 行ずつ取得する）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderText {
    lines: Vec<String>,
}

impl HeaderText {
    /// 復元行（span 1 個 = 1 行・末尾 CR/LF 除去済み）。
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// 復元テキスト全体を行間に改行を挟んで返す（末尾改行なし）。
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
}

/// /Header 欠落（`Error::NotFound`）→ `Ok(None)`。レイアウト不符・span 長が
/// ストリーム末尾を超える等の読み取り失敗 → `Ok(None)`（呼び出し側は本文のみの
/// 従来出力を維持する）。復元行が 1 行も無い（全 span 空・空白のみ）→ `Ok(None)`。
pub fn read_header_text(data: &[u8]) -> Result<Option<HeaderText>> {
    let stream = match read_cfb_stream(data, HEADER_TEXT_PATH) {
        Ok(stream) => stream,
        Err(Error::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error),
    };

    Ok(parse_header_text(&stream))
}

/// /Header ペイロード（ストリーム全体）を解析する（read_header_text の純粋関数部）。
/// レイアウト（SsmgV.01 ヘッダ + w[10] 以降の TextV.01 スロット列 + 末尾テーブル）の
/// 観測構造に合わない入力・span 長がストリーム末尾を超える読み取り失敗・復元行が
/// 1 行も無い場合は `None`（cat は本文のみを出力し続ける）。
pub fn parse_header_text(data: &[u8]) -> Option<HeaderText> {
    if !data.starts_with(HEADER_MAGIC) || data.len() < HEADER_COUNT_WORDS * 2 {
        return None;
    }
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect();

    // w[10] 以降を "TextV.01" マジック 4 語出現位置で走査する（span 長はスロットピッチ
    // 128 を超えて後続へオーバーフローしうるため、スロット番号の算術は使わない）。
    let mut lines: Vec<String> = Vec::new();
    let mut offset = HEADER_COUNT_WORDS;
    while let Some(hit) = next_textv01(&units, offset) {
        // スロット先行語（hit+4）が 0x0000 で無ければ異常語の混入とみなし、
        // そのヒットだけをスキップして次を探す。
        let Some(&slot_head) = units.get(hit + 4) else {
            return None;
        };
        if slot_head != 0x0000 {
            offset = hit + 1;
            continue;
        }
        let Some(&span_len_word) = units.get(hit + 5) else {
            return None;
        };
        let span_len = span_len_word as usize;
        if span_len == 0 {
            // 空 span（宣言のみ）は復元行にならない。
            offset = hit + 1;
            continue;
        }
        let span_start = hit + TEXT_SEGMENT_NAME_WORDS + HEADER_SLOT_HEAD_WORDS;
        if span_start.saturating_add(span_len) > units.len() {
            // 宣言分が欠ける（切詰め等）span は復元不能。
            return None;
        }
        if let Some(line) = decode_header_span(&units, span_start, span_start + span_len) {
            lines.push(line);
        }
        offset = span_start + span_len;
    }

    if lines.is_empty() {
        None
    } else {
        Some(HeaderText { lines })
    }
}

// 1 span の復元: raw 経路（P1 のプロローグ raw デコード。この経路の出力に限り
// `?` をページ番号 "1" に解決）かマーカー経路（0x1f run マーカー以降の本文のみ）
// で取得し、末尾 CR/LF を除去して 1 行化する（末尾の空白・全角スペースは保持）。
// 除去後に空・空白のみなら None（行として丢弃する）。
fn decode_header_span(units: &[u16], start: usize, end: usize) -> Option<String> {
    let text = match first_text_marker(units, start, end) {
        None => {
            // raw span: 0x0000 パディングスキップ・CR/LF 保持・制御境界で打ち切り。
            let (raw, _boundary) = decode_prologue_units(units, start, end)?;
            // F 裁定: プレースホルダ `?` はプレースホルダ `?` を含む span の
            // デコード結果が `"- ? -"` のみと観測されるため、先頭ページ "1" に解決する。
            raw.replace('?', "1")
        }
        Some(marker) => {
            // マーカー型 span: 0x1f run マーカー以降の本文のみを共用の通常マーカー
            // 走査で取得（マーカーより手前のスタイル語ジャンクは絶対読まない）。
            parse_document_text(&words_to_be_bytes(&units[marker..end])).plain_text()
        }
    };

    to_header_line(&text)
}

// 末尾 CR/LF を繰り返し除去して 1 行化する（span 1 個 = 1 行）。
// 除去後に trim が空なら None を返す。
fn to_header_line(text: &str) -> Option<String> {
    let mut line = text;
    while let Some(stripped) = line.strip_suffix(['\r', '\n']) {
        line = stripped;
    }
    if line.trim().is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

/// `from` 以降で "TextV.01" マジック 4 語が初めて出現する位置。
fn next_textv01(units: &[u16], from: usize) -> Option<usize> {
    units
        .windows(TEXT_SEGMENT_NAME_WORDS)
        .enumerate()
        .filter_map(|(hit, words)| (words == &TEXT_SEGMENT_NAME && hit >= from).then_some(hit))
        .next()
}

// words の BE バイナリ化（layout_box_text::words_to_be_bytes と同一の 5 語変換ヘルパー。
// 同側で private なため、ヘッダ側でローカルに持つのは許容される重複）。
fn words_to_be_bytes(words: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(words.len() * 2);
    for word in words {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    const TCNT_SEGMENT_NAME: [u16; 4] = [0x5443, 0x6e74, 0x562e, 0x3031]; // "TCntV.01"
    /// セグメントスロットの語ピッチ（256 byte）。フィクスチャ合成専用のローカル定数。
    const HEADER_SLOT_PITCH_WORDS: usize = 128; // 256 byte
    /// 末尾の位置/所有テーブル列を模した数値語（0x001b・0x005d・0x00a9 等）。
    /// 復元行に漏れてはならない。
    const TAIL_TABLE_WORDS: &[u16] = &[
        0x001b, 0x0000, 0x0001, 0x0000, 0x0001, 0x0000, 0x005d, 0x00a9,
    ];

    fn extend_units(bytes: &mut Vec<u8>, units: &[u16]) {
        for unit in units {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
    }

    fn utf16_units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
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

    // 実測レイアウト（実データ 243 件機械走査確定・mlit-ref3-3_ichitarou と同型）を
    // 模した /Header ペイロードを合成する。本文は宮沢賢治「銀河鉄道の夜」（青空文庫、
    // パブリックドメイン）から合成し、官公庁原本の文言は使用しない。

    /// raw span（0x1c/0x1d/0x1f マーカー無し・全文 UTF-16BE 生）。
    fn raw_span(text: &str) -> Vec<u16> {
        utf16_units(text)
    }

    /// マーカー型 span（先頭 0x001c + スタイル語ジャンク（0x0024/0x0025/0xffff 等）+
    /// 0x001f run マーカー + run 本文 + 末尾 0x000a）。
    fn marker_span(body: &str) -> Vec<u16> {
        let mut span = vec![
            0x001c, 0x0010, 0x001a, 0x0000, 0x0000, 0x0001, 0x0002, 0x0024, 0x0001, 0x0002, 0x0025,
            0x0001, 0x0000, 0x0026, 0x0005, 0x0001, 0x0000, 0x0000, 0x0000, 0x0000, 0xffff, 0x0000,
            0x001a, 0x0000, 0x0010, 0x001f,
        ];
        span.extend(utf16_units(body));
        span.push(0x000a);
        span
    }

    /// `TextV.01` スロット 1 個: 名（4語）+ [0x0000, span 長]（2語）+ span 内容 +
    /// 0x0000 パディング（128 語ピッチまで）。
    fn textv_slot(span: &[u16]) -> Vec<u16> {
        let mut slot = Vec::new();
        slot.extend_from_slice(&TEXT_SEGMENT_NAME);
        slot.extend_from_slice(&[0x0000, span.len() as u16]);
        slot.extend_from_slice(span);
        slot.resize(HEADER_SLOT_PITCH_WORDS, 0x0000);
        slot
    }

    /// 空の `TCntV.01`（自動ページ番号連番キャッシュ）スロット（常に空 → 読まない）。
    fn tcnt_slot() -> Vec<u16> {
        let mut slot = Vec::new();
        slot.extend_from_slice(&TCNT_SEGMENT_NAME);
        slot.extend_from_slice(&[0x0000, 0x0000]);
        slot.resize(HEADER_SLOT_PITCH_WORDS, 0x0000);
        slot
    }

    /// `SsmgV.01` ヘッダ + 128 語ピッチのセグメントスロット列 + 末尾テーブル列
    /// （フル /Header ペイロード）。
    fn header_payload(slots: &[Vec<u16>]) -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::new();
        extend_units(
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
            extend_units(&mut bytes, slot);
        }
        // 末尾の位置/所有テーブル列（数値語）。宣言 span 長で区切るため読まない。
        extend_units(&mut bytes, TAIL_TABLE_WORDS);
        bytes
    }

    // ------------------------------------------------------------------
    // F 規則 / raw 経路
    //

    #[test]
    fn resolves_page_number_placeholder_in_raw_span() {
        // F 裁定: プレースホルダ `?` の span のデコード結果は観測上 `"- ? -"` のみ。
        // `?` を先頭ページ番号 "1" に解決して 1 行化する。
        let payload = header_payload(&[textv_slot(&raw_span("- ? -")), tcnt_slot()]);
        let text = parse_header_text(&payload).expect("valid header should parse");
        assert_eq!(text.lines(), ["- 1 -"]);
        assert_eq!(text.text(), "- 1 -");
        assert!(
            !text.text().contains('?'),
            "プレースホルダ `?` が未解決で残っている"
        );
    }

    #[test]
    fn raw_span_strips_trailing_line_break_and_keeps_inner_one() {
        // raw span: span 内の改行は行区切りとして保持、末尾 CR/LF は除去して
        // 1 span = 1 行にする。
        let inner_break = "銀河の夜\n天文台";
        let mut trailing_break = String::from(inner_break);
        trailing_break.push('\n');
        let payload = header_payload(&[
            textv_slot(&raw_span("Ver.1.0")),
            textv_slot(&raw_span(&trailing_break)),
        ]);
        let text = parse_header_text(&payload).expect("valid header should parse");
        assert_eq!(text.lines(), ["Ver.1.0", inner_break]);
        assert_eq!(text.text(), "Ver.1.0\n銀河の夜\n天文台");
    }

    #[test]
    fn shared_p1_helpers_keep_raw_path_conventions() {
        // ヘッダの raw 経路で共用する P1 ヘルパー（first_text_marker /
        // decode_prologue_units）の規約確認: raw span は マーカー無し、`?` は
        // そのまま保持（F 解決はヘッダ側で行う）。
        let raw = raw_span("- ? -");
        assert_eq!(first_text_marker(&raw, 0, raw.len()), None);
        let (decoded, boundary) = decode_prologue_units(&raw, 0, raw.len()).unwrap();
        assert_eq!(decoded, "- ? -");
        assert_eq!(boundary, raw.len());
        assert_eq!(decoded.replace('?', "1"), "- 1 -");
    }

    // ------------------------------------------------------------------
    // マーカー経路
    //

    #[test]
    fn marker_span_yields_only_run_text_after_text_run_marker() {
        // マーカー型 span: 0x001f run マーカー以降の本文のみを復元し、先頭 0x001c と
        // スタイル語ジャンク（0x0024 `$` / 0x0025 `%` / 0xffff 等）は漏らさない。
        let payload = header_payload(&[
            textv_slot(&marker_span("（別添：銀河鉄道様式）")),
            tcnt_slot(),
        ]);
        let text = parse_header_text(&payload).expect("valid header should parse");
        assert_eq!(text.lines(), ["（別添：銀河鉄道様式）"]);
        let joined = text.text();
        assert!(!joined.contains('$'), "0x0024 スタイル語のリーク");
        assert!(!joined.contains('%'), "0x0025 スタイル語のリーク");
        assert!(!joined.contains('\u{ffff}'), "0xffff 無効スカラーのリーク");
        assert!(
            !joined.contains('\u{001c}'),
            "0x001c レコード開始マーカーのリーク"
        );
    }

    // ------------------------------------------------------------------
    // スロット列・並び順・末尾テーブル
    //

    #[test]
    fn multiple_spans_restore_in_stream_order() {
        // 複数 span（間に空 TCntV.01 を挟む）はストリーム出現順に各行として復元する。
        let payload = header_payload(&[
            textv_slot(&raw_span("- ? -")),
            tcnt_slot(),
            textv_slot(&raw_span("Ver.1.0")),
            tcnt_slot(),
        ]);
        let text = parse_header_text(&payload).expect("valid header should parse");
        assert_eq!(text.lines(), ["- 1 -", "Ver.1.0"]);
        assert_eq!(text.text(), "- 1 -\nVer.1.0");
    }

    #[test]
    fn tail_table_and_segment_names_do_not_leak_into_text() {
        // span 末尾の位置/所有テーブル列（0x001b・0x005d・0x00a9 等の数値語）と
        // セグメント名（TCntV.01 / TextV.01）は text() に漏れない。
        let payload = header_payload(&[textv_slot(&raw_span("- ? -")), tcnt_slot()]);
        let joined = parse_header_text(&payload)
            .expect("valid header should parse")
            .text();
        for junk in ['\u{001b}', '\u{005d}', '\u{00a9}', '\u{0001}', '\u{0000}'] {
            assert!(
                !joined.contains(junk),
                "末尾位置/所有テーブル語 `{junk:?}` が出力に漏れている"
            );
        }
        assert!(!joined.contains("CntV"), "セグメント名 `TCntV.01` のリーク");
        assert!(
            !joined.contains("TextV"),
            "セグメント名 `TextV.01` のリーク"
        );
    }

    #[test]
    fn skips_hits_with_nonzero_slot_head_word() {
        // 異常語の混入耐性: ヒット位置の units[p+4] != 0x0000 はそのヒットを
        // スキップし、次の "TextV.01" を探して復元を続ける。
        let mut broken_slot = Vec::new();
        broken_slot.extend_from_slice(&[0x5465, 0x7874, 0x562e, 0x3031, 0x0001, 0x0000]);
        broken_slot.resize(HEADER_SLOT_PITCH_WORDS, 0x0000);
        let payload = header_payload(&[broken_slot, textv_slot(&raw_span("Ver.1.0"))]);
        let text = parse_header_text(&payload).expect("abnormal slot head should be skipped");
        assert_eq!(text.lines(), ["Ver.1.0"]);
    }

    // ------------------------------------------------------------------
    // 戻り値の確定（None / Some）
    //

    #[test]
    fn returns_none_when_all_spans_are_empty() {
        // 全 span 空（span 長 0 のみ・TCntV.01 のみ）→ 復元行 0 行 → None。
        let payload = header_payload(&[textv_slot(&[]), tcnt_slot()]);
        assert!(parse_header_text(&payload).is_none());
    }

    #[test]
    fn returns_none_when_lines_are_blank_only() {
        // 復元行が空白のみ（末尾の空白・全角スペースは保持するが、trim で空なら丢弃）
        // → None。
        let payload = header_payload(&[textv_slot(&raw_span(" \u{3000} "))]);
        assert!(parse_header_text(&payload).is_none());
    }

    #[test]
    fn rejects_malformed_layouts() {
        // マジック不一致。
        assert!(parse_header_text(b"TextV.01........").is_none());
        // 10 語未満（ヘッダ不足）。
        let mut short: Vec<u8> = Vec::new();
        extend_units(&mut short, &[0x5373, 0x6d67, 0x562e, 0x3031, 0x0000]);
        assert!(parse_header_text(&short).is_none());
        // 奇数 byte 数（word 化不能）。
        assert!(parse_header_text(b"SsmgV.01\x00").is_none());

        // span 長がストリーム末尾を超える（切詰め）読み取り失敗 → None。
        let mut truncated: Vec<u8> = Vec::new();
        extend_units(
            &mut truncated,
            &[
                0x5373, 0x6d67, 0x562e, 0x3031, // SsmgV.01
                0x0000, 0x0001, 0x0000, 0x0100, 0x0000, 0x0001, 0x5465, 0x7874, 0x562e,
                0x3031, // TextV.01
                0x0000, 0x7fff,
            ],
        );
        extend_units(&mut truncated, TAIL_TABLE_WORDS);
        assert!(parse_header_text(&truncated).is_none());

        // span 長 0x0186（390 語）はスロットピッチ 128 を超えるオーバーフロー宣言の
        // 形だが、ストリーム末尾を超えるため読み取り失敗 → None。
        let mut overflow: Vec<u8> = Vec::new();
        extend_units(
            &mut overflow,
            &[
                0x5373, 0x6d67, 0x562e, 0x3031, // SsmgV.01
                0x0000, 0x0001, 0x0000, 0x0100, 0x0000, 0x0001, 0x5465, 0x7874, 0x562e,
                0x3031, // TextV.01
                0x0000, 0x0186,
            ],
        );
        extend_units(&mut overflow, TAIL_TABLE_WORDS);
        assert!(parse_header_text(&overflow).is_none());
    }

    #[test]
    fn read_header_text_returns_none_when_stream_missing() {
        let bytes = cfb_with_stream("/Unused", b"payload");
        assert_eq!(read_header_text(&bytes), Ok(None));
    }

    #[test]
    fn read_header_text_extracts_from_cfb_stream() {
        let payload = header_payload(&[
            textv_slot(&raw_span("- ? -")),
            tcnt_slot(),
            textv_slot(&raw_span("Ver.1.0")),
            tcnt_slot(),
        ]);
        let bytes = cfb_with_stream(HEADER_TEXT_PATH, &payload);
        let text = read_header_text(&bytes)
            .unwrap()
            .expect("should extract header text");
        assert_eq!(text.lines(), ["- 1 -", "Ver.1.0"]);
        assert_eq!(text.text(), "- 1 -\nVer.1.0");
    }
}
