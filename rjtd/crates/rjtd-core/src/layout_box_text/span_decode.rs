// /LayoutBoxText（P2）の单一ブロック span 復元。
//
// P1 の「プロローグ raw デコード」機構（document_text::decode_prologue_units /
// first_text_marker / parse_document_text）を再利用し、parse_document_text を合成
// ヘッダでラップする二重実装を避ける（PARTIAL-LOSS-REPORT.md P2 修正方針 2・スパゲッティ
// 防止指示）。
//
// span 復元規則（cat 出力契約。tests/streams/layout_box_text.rs が完全一致で pins）:
//   - 最初のマーカー（0x001c/0x001d/0x001f）位置 m より手前の raw 前置き:
//     decode_prologue_units で復元（CR/LF 保持・0x0000 パディング除外・制御境界で打ち切り）。
//   - m 以降: 0x001c…0x001f レコード（本体の座標ジャンク 0x00be/0x020d はスキップ）、
//     0x001d…0x001e インラインテキスト（抽出）は parse_document_text が通常の
//     マーカー走査で処理する（reading_text=false で開始するため前置きより後だけ読まれ、
//     レコード本体の printable バイナリ語は読み込まれない）。
//   - マーカー無しブロックは全文 raw（decode_prologue_units 全文）。
//
// ブロック毎に（前置き text ＋ インライン/run text を出現順で連結）、ブロック間改行は
// LayoutBoxText::text() が行う。

use crate::document_text::parse_document_text;
use crate::document_text::{decode_prologue_units, first_text_marker};

pub(crate) fn decode_layout_box_span(units: &[u16], start: usize, end: usize) -> String {
    match first_text_marker(units, start, end) {
        Some(marker_index) => {
            let prologue = decode_prologue_units(units, start, marker_index)
                .map(|(text, _)| text)
                .unwrap_or_default();
            let marker_text = parse_marker_part(units, marker_index, end);
            prologue + &marker_text
        }
        None => decode_prologue_units(units, start, end)
            .map(|(text, _)| text)
            .unwrap_or_default(),
    }
}

// m..end を通常のマーカー走査で plain-text 化する。parse_document_text は
// reading_text=false で開始するため、前置き（マーカーより手前）は読まれず m 以降の
// インライン/run テキストのみを取得する。
fn parse_marker_part(units: &[u16], marker_index: usize, end: usize) -> String {
    let bytes = super::words_to_be_bytes(&units[marker_index..end]);
    parse_document_text(&bytes).plain_text()
}
