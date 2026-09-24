//! /Header（ヘッダ・フッタ本文）前置きの cat / export-txt ゲート
//! （PARTIAL-LOSS-REPORT.md 付録2 実装方針 P-H＋F・RED テスト）。
//!
//! 裁定（2026-9-24 / slim/text-only）:
//!   - P-H: `/Header` のヘッダ・フッタ本文を Tika 準拠で**本文前**（先頭行）に出力する。
//!   - F: raw span のページ番号プレースホルダ `?` をページ番号に解決し `- 1 -` として出力する。
//!
//! 観測構造（236+7 件全件機械走査で確定・実測 mlit-ref3-3_ichitarou /
//! houmukyoku-content-001188290 と同型）:
//!   w[0..9]  SsmgV.01 ヘッダ（w[9] = セグメント数）
//!   各スロット（語ピッチ 128）: `TextV.01` + [0x0000, span 長] + span 内容 + 0x0000 パディング
//!   空の `TCntV.01` スロット（自動ページ番号連番キャッシュ）は読まない。
//!   末尾に位置/所有テーブル列（0x001b・0x005d 等の数値語）— 絶対に出ないこと。
//!
//! span 種別（Span 内 0x1c/0x1d/0x1f の有無）:
//!   - raw 242: `"- ? -"`（→ `- 1 -`）/ `"Ver.1.0"` 等全文 UTF-16BE 生
//!   - マーカー型 15: `«1c»…«1f»` レコード後の run 本文（mlit「（別添：…）」型）
//!
//! 出力契約: ヘッダ行（span をストリーム順に 1 行ずつ）+ 空行 を本文の前に前置き。
//! `/Header` 欠落・全 span 空・span 長異常（読み取り失敗）は出力完全不変（ゲート）。
//!
//! テスト本文は宮沢賢治「銀河鉄道の夜」（青空文庫、パブリックドメイン）からの合成で、
//! 官公庁原本の文言は使用していない。

use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::process::Command;

use super::support::*;

const HEADER_SEGMENT_NAME: [u16; 4] = [0x5465, 0x7874, 0x562e, 0x3031]; // "TextV.01"
const HEADER_CNT_SEGMENT_NAME: [u16; 4] = [0x5443, 0x6e74, 0x562e, 0x3031]; // "TCntV.01"
const HEADER_SEGMENT_PITCH_WORDS: usize = 128; // 256 byte
/// テスト末尾に置く位置/所有テーブル列を模した数値語（出力に漏れてはならない）。
const HEADER_TAIL_TABLE: [u16; 8] = [
    0x001b, 0x0000, 0x0001, 0x0000, 0x0001, 0x0000, 0x005d, 0x00a9,
];

fn extend_words(bytes: &mut Vec<u8>, words: &[u16]) {
    for word in words {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
}

fn utf16_words(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// raw span（マーカー無し・全文 UTF-16BE 生）。
fn raw_span(text: &str) -> Vec<u16> {
    utf16_words(text)
}

/// マーカー型 span（実測 mlit-ref3-3_ichitarou と同型の `«1c»…«1f»run本文 LF`）。
/// スタイル語（0x0024 `$`・0x0025 `%`・0xffff 等）を含み、run 本文以外は漏れないことを
/// 個別に断言する。
fn marker_span(body: &str) -> Vec<u16> {
    let mut span: Vec<u16> = vec![
        0x001c, 0x0010, 0x001a, 0x0000, 0x0000, 0x0001, 0x0002, 0x0024, 0x0001, 0x0002, 0x0025,
        0x0001, 0x0000, 0x0026, 0x0005, 0x0001, 0x0000, 0x0000, 0x0000, 0x0000, 0xffff, 0x0000,
        0x001a, 0x0000, 0x0010, 0x001f,
    ];
    span.extend(utf16_words(body));
    span.push(0x000a);
    span
}

/// `SsmgV.01` ヘッダ + 128 語ピッチのセグメントスロット列 + 末尾テーブル列。
fn header_payload(segments: &[&[u16]]) -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    extend_words(
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
            segments.len() as u16,
        ],
    );
    for segment in segments {
        // `empty_cnt_slot()`（セグメント名語のみ）を空 `TCntV.01` スロットとして書く。
        let cnt = *segment == HEADER_CNT_SEGMENT_NAME.as_slice();
        let name = if cnt {
            HEADER_CNT_SEGMENT_NAME.as_slice()
        } else {
            HEADER_SEGMENT_NAME.as_slice()
        };
        // span は宣言長どおりに書き、スロット末尾は 128 語ピッチへ 0x0000 パディング
        // （実測 mlit / houmukyoku-content-001188290 と同型）。
        let body_len = if cnt { 0 } else { segment.len() };
        extend_words(&mut bytes, name);
        extend_words(&mut bytes, &[0x0000, body_len as u16]);
        if !cnt {
            extend_words(&mut bytes, segment);
        }
        let pad = HEADER_SEGMENT_PITCH_WORDS.saturating_sub(6 + body_len);
        extend_words(&mut bytes, &vec![0x0000u16; pad]);
    }
    extend_words(&mut bytes, &HEADER_TAIL_TABLE);
    bytes
}

/// `TCntV.01`（空）スロットを表す sentinel（span 内容なし）。
fn empty_cnt_slot() -> Vec<u16> {
    HEADER_CNT_SEGMENT_NAME.to_vec()
}

fn jtd_with_header(header: &[u8]) -> PathBuf {
    let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    compound
        .create_stream("/\u{4}JSRV_SegmentInformation")
        .unwrap()
        .write_all(b"segment")
        .unwrap();
    compound
        .create_stream("/DocInfo")
        .unwrap()
        .write_all(b"doc")
        .unwrap();
    compound.create_storage("/BodyText").unwrap();
    compound
        .create_stream("/BodyText/Section0")
        .unwrap()
        .write_all(b"hello")
        .unwrap();
    compound
        .create_stream("/DocumentText")
        .unwrap()
        .write_all(&document_text_fixture())
        .unwrap();
    compound
        .create_stream("/Header")
        .unwrap()
        .write_all(header)
        .unwrap();
    write_sample(compound.into_inner().into_inner())
}

fn jtd_without_header() -> PathBuf {
    tiny_cfb_path()
}

fn run_cat(path: &PathBuf) -> (i32, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rjtd"))
        .arg("cat")
        .arg(path)
        .output()
        .unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

fn run_export_txt(path: &PathBuf) -> (i32, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_rjtd"))
        .arg("export")
        .arg(path)
        .args(["--format", "txt"])
        .output()
        .unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// F 規則: raw span `"- ? -"` → 先頭ページ番号に解決して `- 1 -` を本文前に前置き。
#[test]
fn cat_prepends_resolved_page_number_from_raw_header_span() {
    let path = jtd_with_header(&header_payload(&[&raw_span("- ? -"), &empty_cnt_slot()]));
    let (code, stdout) = run_cat(&path);
    fs_remove(&path);

    assert_eq!(code, 0);
    assert_eq!(stdout, "- 1 -\n\n銀河鉄道\n");
    assert!(
        !stdout.contains('?'),
        "プレースホルダ `?` が未解決で残っている"
    );
    assert_no_header_junk(&stdout);
}

/// P-H: マーカー型 span（`«1c»…«1f»`）は run 本文のみ復元し、前置きスタイル語
/// （`$` `%` 0xffff 等）は文字化けリークしない。
#[test]
fn cat_prepends_marker_header_span_run_text() {
    let path = jtd_with_header(&header_payload(&[
        &marker_span("（別添：銀河鉄道様式）"),
        &empty_cnt_slot(),
    ]));
    let (code, stdout) = run_cat(&path);
    fs_remove(&path);

    assert_eq!(code, 0);
    assert_eq!(stdout, "（別添：銀河鉄道様式）\n\n銀河鉄道\n");
    assert!(!stdout.contains('$'), "0x0024 スタイル語のリーク");
    assert!(!stdout.contains('%'), "0x0025 スタイル語のリーク");
    assert!(!stdout.contains('\u{ffff}'), "0xffff 無効スカラーのリーク");
    assert_no_header_junk(&stdout);
}

/// 複数 span はストリーム順に各行として前置き（ページ番号 + 版数注記型）。
#[test]
fn cat_prepends_multiple_header_spans_in_stream_order() {
    let path = jtd_with_header(&header_payload(&[
        &raw_span("- ? -"),
        &empty_cnt_slot(),
        &raw_span("Ver.1.0"),
        &empty_cnt_slot(),
    ]));
    let (code, stdout) = run_cat(&path);
    fs_remove(&path);

    assert_eq!(code, 0);
    assert_eq!(stdout, "- 1 -\nVer.1.0\n\n銀河鉄道\n");
    assert_no_header_junk(&stdout);
}

/// raw span 内の改行（CR/LF）は行区切りとして保持する（P1 デコード規則の共用）。
#[test]
fn cat_header_span_keeps_line_breaks_in_raw_text() {
    let path = jtd_with_header(&header_payload(&[
        &raw_span("銀河の夜\n天文台"),
        &empty_cnt_slot(),
    ]));
    let (code, stdout) = run_cat(&path);
    fs_remove(&path);

    assert_eq!(code, 0);
    assert_eq!(stdout, "銀河の夜\n天文台\n\n銀河鉄道\n");
    assert_no_header_junk(&stdout);
}

/// ゲート: `/Header` 欠落は出力完全不変。
#[test]
fn cat_output_unchanged_without_header_stream() {
    let path = jtd_without_header();
    let (code, stdout) = run_cat(&path);
    fs_remove(&path);

    assert_eq!(code, 0);
    assert_eq!(stdout, "銀河鉄道\n");
}

/// ゲート: span 長 0（全 span 空・`TCntV.01` のみ）は出力完全不変。
#[test]
fn cat_output_unchanged_when_header_spans_are_empty() {
    let empty_textv: Vec<u16> = Vec::new();
    let path = jtd_with_header(&header_payload(&[&empty_textv, &empty_cnt_slot()]));
    let (code, stdout) = run_cat(&path);
    fs_remove(&path);

    assert_eq!(code, 0);
    assert_eq!(stdout, "銀河鉄道\n");
}

/// ゲート: span 長がストリーム末尾を超える読み取り失敗は成功のまま出力完全不変。
#[test]
fn cat_output_unchanged_when_header_span_is_truncated() {
    // "TextV.01" + [0x0000, 0x7fff] + テーブル数語だけでは span 宣言を満たさない。
    let mut broken: Vec<u8> = Vec::new();
    extend_words(
        &mut broken,
        &[
            0x5373, 0x6d67, 0x562e, 0x3031, // SsmgV.01
            0x0000, 0x0001, 0x0000, 0x0100, 0x0000, 0x0001, 0x5465, 0x7874, 0x562e,
            0x3031, // TextV.01
            0x0000, 0x7fff,
        ],
    );
    extend_words(&mut broken, &HEADER_TAIL_TABLE);
    let path = jtd_with_header(&broken);
    let (code, stdout) = run_cat(&path);
    fs_remove(&path);

    assert_eq!(code, 0);
    assert_eq!(stdout, "銀河鉄道\n");
}

/// export-txt ミラー: cat と同一規則で `to_plain_text` 先頭に連結される。
#[test]
fn export_txt_prepends_header_before_body() {
    let path = jtd_with_header(&header_payload(&[
        &raw_span("- ? -"),
        &empty_cnt_slot(),
        &raw_span("Ver.1.0"),
        &empty_cnt_slot(),
    ]));
    let (code, stdout) = run_export_txt(&path);
    fs_remove(&path);

    assert_eq!(code, 0);
    assert_eq!(stdout, "- 1 -\nVer.1.0\n\n銀河鉄道\n");
    assert_no_header_junk(&stdout);
}

/// export-txt ゲート: `/Header` 欠落時は従来出力のまま。
#[test]
fn export_txt_unchanged_without_header_stream() {
    let path = jtd_without_header();
    let (code, stdout) = run_export_txt(&path);
    fs_remove(&path);

    assert_eq!(code, 0);
    assert_eq!(stdout, "銀河鉄道\n");
}

fn fs_remove(path: &PathBuf) {
    let _ = std::fs::remove_file(path);
}

/// 末尾位置/所有テーブル列・スロット名・区切り制御語が出力に一切漏れないこと。
fn assert_no_header_junk(stdout: &str) {
    for junk in [
        '\u{001b}', '\u{005d}', '\u{00a9}', '\u{001c}', '\u{001d}', '\u{001e}', '\u{001f}',
    ] {
        assert!(
            !stdout.contains(junk),
            "ヘッダ領域の制御語・テーブル語 `{junk:?}` が出力に漏れている"
        );
    }
    assert!(!stdout.contains("CntV"), "セグメント名 `TCntV.01` のリーク");
}
