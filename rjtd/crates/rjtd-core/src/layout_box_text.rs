// /LayoutBoxText（囲み枠テキスト）ストリームのレコード reader（PARTIAL-LOSS-REPORT.md P2）。
//
// 観測レイアウト（houmukyoku-content-001188695.jtd の実測値・tests/streams/layout_box_text.rs の
// 合成フィクスチャと同一構造、word 単位）:
//   w[0..9]      SsmgV.01 ヘッダ（w[9] = ブロック数）
//   w[10 + 128*k]（byte 20 + 256*k）に各ブロック:
//     "TextV.01"（4語） + [0x0000, span 長]（2語） + span 内容 + 0x0000 パディング
//     （ブロック語ピッチ 128 word = 256 byte）
//   ストリーム末尾: 位置/所有テーブル列（dword 群、'c'(0x0063)・'I'(0x0049) 等の
//     printable な語を含む）。テキスト復元には絶対に読めない。
//
// span 内容の種別:
//   - 最初のマーカー（0x001c/0x001d/0x001f）より手前の raw 前置きテキスト
//   - 0x001c…0x001f レコード（本体に座標等の printable バイナリ語 0x00be/0x020d を含み、
//     必ずスキップする）
//   - 0x001d…0x001e インラインテキスト（テキストのみ抽出する）
//   - マーカー無しブロックは全文 raw（0x000a 改行保持・末尾 0x0000 パディング除去）
//
// 復元テキストの cat 出力契約（crates/rjtd-cli run_cat）:
//   ブロック毎に（前置き text ＋ インライン/run text を出現順で連結、前置き内の
//   CR/LF は保持）、ブロック間を '\n' で連結。
//
// 読取領域はブロック数（w[9]）とピッチで厳密に区切り、末尾テーブル列は読まない。
// span 復元は P1 のプロローグ raw デコード機構（document_text::decode_prologue_units /
// first_text_marker ＋ 通常のマーカー走査）を再利用する。

mod span_decode;

use crate::Error;
use crate::Result;
use crate::container::read_cfb_stream;

use span_decode::decode_layout_box_span;

pub const LAYOUT_BOX_TEXT_PATH: &str = "/LayoutBoxText";
const LAYOUT_BOX_MAGIC: &[u8; 8] = b"SsmgV.01";
const LAYOUT_BOX_TEXT_SEGMENT_NAME: &[u8; 8] = b"TextV.01";
const LAYOUT_BOX_HEADER_WORDS: usize = 10; // SsmgV.01 (4) + header (4) + block-count (2)
const LAYOUT_BOX_TEXT_SEGMENT_NAME_WORDS: usize = 4;
const LAYOUT_BOX_SPAN_HEADER_WORDS: usize = 2; // [0x0000, span 長]
const LAYOUT_BOX_BLOCK_PITCH_WORDS: usize = 128; // 256 byte
const LAYOUT_BOX_MAX_SPAN_WORDS: usize = LAYOUT_BOX_BLOCK_PITCH_WORDS
    - LAYOUT_BOX_TEXT_SEGMENT_NAME_WORDS
    - LAYOUT_BOX_SPAN_HEADER_WORDS;

/// /LayoutBoxText 内の復元テキスト（ブロックを '\n' で連結して取得する）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutBoxText {
    blocks: Vec<String>,
}

impl LayoutBoxText {
    fn new(blocks: Vec<String>) -> Self {
        Self { blocks }
    }

    /// ブロック単位の復元テキスト（ブロック内は前置き＋インライン/run の出現順連結）。
    pub fn blocks(&self) -> &[String] {
        &self.blocks
    }

    /// 復元テキスト全体をブロック間に改行を挟んで返す。
    pub fn text(&self) -> String {
        self.blocks.join("\n")
    }
}

/// /LayoutBoxText ストリームから囲み枠テキストを復元する。
/// ストリームが無い場合は `Ok(None)`、レイアウトが観測構造と合わない場合は `Ok(None)` を返す
/// （呼び出し側は本文のみの従来出力を維持する）。
pub fn read_layout_box_text(data: &[u8]) -> Result<Option<LayoutBoxText>> {
    let stream = match read_cfb_stream(data, LAYOUT_BOX_TEXT_PATH) {
        Ok(stream) => stream,
        Err(Error::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error),
    };

    Ok(parse_layout_box_text(&stream))
}

/// /LayoutBoxText ペイロード（ストリーム全体）を解析する。
/// レイアウト（SsmgV.01 ヘッダ + TextV.01 ブロック列 + 末尾テーブル）の観測構造に
/// 合わない入力は `None`（cat は本文のみを出力し続ける）。
pub fn parse_layout_box_text(data: &[u8]) -> Option<LayoutBoxText> {
    if !data.starts_with(LAYOUT_BOX_MAGIC) || data.len() < LAYOUT_BOX_HEADER_WORDS * 2 {
        return None;
    }
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect();
    let block_count = *units.get(LAYOUT_BOX_HEADER_WORDS - 1)? as usize;
    if block_count == 0 {
        return Some(LayoutBoxText::default());
    }

    // ブロック領域は w[9]（ブロック数）とピッチで厳密に区切る（末尾テーブルは読まない）。
    // 宣言分が欠ける（切り詰め等）レイアウトは復元不能とみなす。
    let block_area_end =
        LAYOUT_BOX_HEADER_WORDS.saturating_add(block_count * LAYOUT_BOX_BLOCK_PITCH_WORDS);
    if units.len() < block_area_end {
        return None;
    }

    let mut blocks = Vec::new();
    for k in 0..block_count {
        let block_unit_start = LAYOUT_BOX_HEADER_WORDS + k * LAYOUT_BOX_BLOCK_PITCH_WORDS;
        if !is_textv01_segment_name(&units, block_unit_start) {
            return None;
        }
        let span_start =
            block_unit_start + LAYOUT_BOX_TEXT_SEGMENT_NAME_WORDS + LAYOUT_BOX_SPAN_HEADER_WORDS;
        let span_len = units.get(span_start - 1).copied().unwrap_or(0) as usize;
        if span_len > LAYOUT_BOX_MAX_SPAN_WORDS {
            return None;
        }
        blocks.push(decode_layout_box_span(
            &units,
            span_start,
            span_start + span_len,
        ));
    }

    Some(LayoutBoxText::new(blocks))
}

/// ブロック先頭に "TextV.01" セグメント名があるか（4語 = 8 byte の完全一致）。
fn is_textv01_segment_name(units: &[u16], offset: usize) -> bool {
    units
        .get(offset..offset + LAYOUT_BOX_TEXT_SEGMENT_NAME_WORDS)
        .is_some_and(|words| words_to_be_bytes(words) == LAYOUT_BOX_TEXT_SEGMENT_NAME)
}

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
    use crate::document_text::{decode_prologue_units, first_text_marker};
    use std::io::{Cursor, Write};

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

    // 実測レイアウト（houmukyoku-content-001188695 同型）を模した /LayoutBoxText ペイロード。
    // 本文は官公庁原本の実文言を使わず、宮沢賢治「銀河鉄道の夜」（青空文庫、パブリック
    // ドメイン）から合成する。末尾テーブルは observable の dword 列（'c' 'I' 等の
    // printable な語）を模す。
    fn fixture_block1() -> Vec<u16> {
        let prologue = "「ジョバンニさん。あなたはわかっているのでしょう。」";
        let inline = "「やっぱり星だとジョバンニは思いましたが」";
        let mut block: Vec<u16> = Vec::new();
        block.extend(utf16_units(&format!("{prologue}\n")));
        block.extend_from_slice(&[
            0x001c, 0x0000, 0x000c, 0x0000, 0x0007, 0x00be, 0x020d, 0x0000, 0x000c, 0x0000, 0x0000,
            0x001f,
        ]);
        block.extend_from_slice(&[0x001c, 0x0001, 0x0007, 0x0000, 0x0000, 0x0003, 0x001d]);
        block.extend(utf16_units(inline));
        block.push(0x001e);
        block
    }

    fn fixture_raw_block() -> Vec<u16> {
        utf16_units("「うん。ぼく牛乳をとりながら見てくるよ。」")
    }

    fn tail_table_words() -> &'static [u16] {
        &[
            0x0000, 0x0063, 0x0000, 0x0001, 0x0000, 0x0001, 0x0000, 0x0000, 0x0000, 0x0049, 0x0000,
            0x0001,
        ]
    }

    /// 2 ブロック＋末尾テーブル列のフルペイロード（CLI テストと同一構造）。
    fn layout_box_text_payload(block_count: u16) -> Vec<u8> {
        let block1 = fixture_block1();
        let block2 = fixture_raw_block();
        let mut bytes: Vec<u8> = Vec::new();
        extend_units(
            &mut bytes,
            &[
                0x5373,
                0x6d67,
                0x562e,
                0x3031,
                0x0000,
                0x0002,
                0x0000,
                0x0100,
                0x0000,
                block_count,
            ],
        );
        for block in [&block1, &block2] {
            extend_units(&mut bytes, &[0x5465, 0x7874, 0x562e, 0x3031]); // "TextV.01"
            extend_units(&mut bytes, &[0x0000, block.len() as u16]);
            extend_units(&mut bytes, block);
            let used =
                LAYOUT_BOX_TEXT_SEGMENT_NAME_WORDS + LAYOUT_BOX_SPAN_HEADER_WORDS + block.len();
            extend_units(
                &mut bytes,
                &vec![0x0000u16; LAYOUT_BOX_BLOCK_PITCH_WORDS - used],
            );
        }
        // 末尾の位置/所有テーブル列（dword 列、'c' 'I' 等の printable 語を含む）。
        extend_units(&mut bytes, tail_table_words());
        bytes
    }

    #[test]
    fn decodes_mixed_block_as_prologue_then_inline_text() {
        // raw 前置き（末尾改行保持）＋ 0x001c…0x001f レコード（座標ジャンク 0x00be/0x020d
        // を必ずスキップ）＋ 0x001d…0x001e インライン（抽出）。
        let block = fixture_block1();
        let decoded = decode_layout_box_span(&block, 0, block.len());
        assert_eq!(
            decoded,
            "「ジョバンニさん。あなたはわかっているのでしょう。」\n「やっぱり星だとジョバンニは思いましたが」"
        );
        assert!(!decoded.contains('\u{00be}'));
        assert!(!decoded.contains('\u{020d}'));
    }

    #[test]
    fn decodes_markerless_block_as_full_raw_text() {
        let block = fixture_raw_block();
        assert_eq!(
            decode_layout_box_span(&block, 0, block.len()),
            "「うん。ぼく牛乳をとりながら見てくるよ。」"
        );
    }

    #[test]
    fn parses_blocks_joins_with_newlines_and_ignores_tail_table() {
        let payload = layout_box_text_payload(2);
        let text = parse_layout_box_text(&payload).expect("valid layout should parse");

        assert_eq!(text.blocks().len(), 2);
        assert_eq!(
            text.blocks()[0],
            "「ジョバンニさん。あなたはわかっているのでしょう。」\n「やっぱり星だとジョバンニは思いましたが」"
        );
        assert_eq!(
            text.blocks()[1],
            "「うん。ぼく牛乳をとりながら見てくるよ。」"
        );
        // ブロック間は改行、末尾テーブル列の 'c'/'I' と座標ジャンクは絶対に出ないこと。
        let joined = text.text();
        assert_eq!(
            joined,
            "「ジョバンニさん。あなたはわかっているのでしょう。」\n「やっぱり星だとジョバンニは思いましたが」\n「うん。ぼく牛乳をとりながら見てくるよ。」"
        );
        assert!(!joined.contains('c'));
        assert!(!joined.contains('I'));
        assert!(!joined.contains('¾'));
        assert!(!joined.contains('\u{020d}'));
    }

    #[test]
    fn block_count_bounds_reading_to_declared_area_only() {
        // w[9]=1 の宣言で 2 ブロック分の実データが並ぶ場合、1 ブロック目だけ復元する
        // （ブロック領域は w[9] とピッチで厳密に区切ること）。
        let mut payload = layout_box_text_payload(2);
        // ヘッダのブロック数（w[9]）を 1 に書き換える。
        let header_count_offset = (LAYOUT_BOX_HEADER_WORDS - 1) * 2;
        payload[header_count_offset..header_count_offset + 2].copy_from_slice(&1u16.to_be_bytes());

        let text = parse_layout_box_text(&payload).expect("single declared block should parse");
        assert_eq!(text.blocks().len(), 1);
        assert!(
            text.text()
                .starts_with("「ジョバンニさん。あなたはわかっているのでしょう。」")
        );
        assert!(
            !text
                .text()
                .contains("「うん。ぼく牛乳をとりながら見てくるよ。」")
        );
    }

    #[test]
    fn read_layout_box_text_returns_none_when_stream_missing() {
        let bytes = cfb_with_stream("/Unused", b"payload");
        assert!(read_layout_box_text(&bytes).unwrap().is_none());
    }

    #[test]
    fn read_layout_box_text_extracts_from_cfb_stream() {
        let payload = layout_box_text_payload(2);
        let bytes = cfb_with_stream(LAYOUT_BOX_TEXT_PATH, &payload);
        let text = read_layout_box_text(&bytes)
            .unwrap()
            .expect("should extract box text");
        assert_eq!(text.blocks().len(), 2);
        assert!(
            text.text()
                .starts_with("「ジョバンニさん。あなたはわかっているのでしょう。」")
        );
    }

    #[test]
    fn rejects_malformed_layouts() {
        // マジック不一致。
        assert!(parse_layout_box_text(b"TextV.01........").is_none());
        // ヘッダのみ（ブロック数 0）は空テキスト（ブロック皆無）として成立。
        let mut header_only: Vec<u8> = Vec::new();
        extend_units(
            &mut header_only,
            &[
                0x5373, 0x6d67, 0x562e, 0x3031, 0x0000, 0x0002, 0x0000, 0x0100, 0x0000, 0x0000,
            ],
        );
        let text = parse_layout_box_text(&header_only).expect("header-only block=0 should parse");
        assert!(text.blocks().is_empty());

        // ブロック宣言があるがデータが切り詰められている（領域不足）場合は復元不能。
        let mut truncated: Vec<u8> = Vec::new();
        extend_units(
            &mut truncated,
            &[
                0x5373, 0x6d67, 0x562e, 0x3031, 0x0000, 0x0002, 0x0000, 0x0100, 0x0000, 0x0004,
                0x5465, 0x7874,
            ],
        );
        assert!(parse_layout_box_text(&truncated).is_none());

        // 領域内だが TextV.01 セグメント名が無い（構造崩壊）場合も復元不能。
        let mut corrupted = layout_box_text_payload(1);
        // ブロック1 の "TextV.01" を破壊する。
        corrupted[20..28].copy_from_slice(b"XXXXXXXX");
        assert!(parse_layout_box_text(&corrupted).is_none());
    }

    #[test]
    fn shared_p1_helpers_keep_marker_and_boundary_semantics() {
        // P1 共用ヘルパー（first_text_marker / decode_prologue_units）の復元規約確認。
        let block = fixture_block1();
        let prose = "「ジョバンニさん。あなたはわかっているのでしょう。」"
            .encode_utf16()
            .count();
        assert_eq!(first_text_marker(&block, 0, block.len()), Some(prose + 1));

        let mut raw: Vec<u16> = Vec::new();
        raw.extend(utf16_units("前置きA\n"));
        raw.extend_from_slice(&[0x0000, 0x0000]);
        raw.extend(utf16_units("前置きB"));
        raw.push(0x0019); // 制御境界: 打ち切り
        raw.extend(utf16_units("ノイズ"));
        let (text, boundary) = decode_prologue_units(&raw, 0, raw.len()).unwrap();
        assert_eq!(text, "前置きA\n前置きB");
        assert_eq!(boundary, raw.len() - 4);
    }
}
