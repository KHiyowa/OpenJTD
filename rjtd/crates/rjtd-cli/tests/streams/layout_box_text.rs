use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::process::Command;

// 囲み枠テキスト（/LayoutBoxText）のコンテナレベル復元テスト（PARTIAL-LOSS-REPORT.md P2）。
//
// 実原本 houmukyoku-content-001188695.jtd（再使用証明申出書）では、領収欄ブロック6行が
// /LayoutBoxText（TextV.01 マジック＋約256 byte 固定長ブロック、UTF-16BE テキスト語＋
// 0x0000 パディング、末尾に位置/所有テーブル列）に実在するのに `cat` が枠内を一切
// 出力しない。修正方針は同レポート P2（`cat` が本文 /DocumentText の後に
// 「※枠内テキスト」注記を添えて枠テキストを連結）。
//
// テスト本文は官公庁原本の文言を使わず、宮沢賢治「銀河鉄道の夜」（青空文庫、
// パブリックドメイン、https://www.aozora.gr.jp/cards/000081/files/456_15050.html
// よりふりがなを除去）を合成する。

const DOC_PROLOGUE: &str = "ジョバンニは窓をあけました。";
const DOC_RUN1: &str = "「ジョバンニさん。あなたはわかっているのでしょう。」";
const DOC_RUN2: &str = "「大きな望遠鏡で銀河をよっく調べると銀河は大体何でしょう。」";
const BOX_LABEL1: &str = "「お母さんの牛乳は来ていないんだろうか。」";
const BOX_INLINE1: &str = "「ああ、お前さきにおあがり。あたしはまだほしくないんだから。」";
const BOX_RAW2: &str = "「うん。ぼく牛乳をとりながら見てくるよ。」";

const BOX_TEXT_NOTE: &str = "※枠内テキスト";

fn run_cat(path: &Path) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rjtd"))
        .arg("cat")
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "rjtd cat {} が失敗しました (exit={}). stderr: {}",
        path.display(),
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn extend_units(bytes: &mut Vec<u8>, units: &[u16]) {
    for unit in units {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
}

fn utf16_units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// /DocumentText ペイロード: SsmgV.01 + TextV.01 + 長さ + raw 前置き + マーカー本文。
/// 前置き部分は P1 型（maff-tenpu02-238 と同じ混在型）を兼ねる。
fn document_text_payload() -> Vec<u8> {
    let mut content: Vec<u16> = utf16_units(&format!("{DOC_PROLOGUE}\n"));
    content.extend_from_slice(&[0x001c, 0x0010, 0x0006, 0x0000, 0x0010, 0x001f]);
    content.extend(utf16_units(DOC_RUN1));
    content.push(0x000a);
    content.extend_from_slice(&[0x001c, 0x0010, 0x0006, 0x0000, 0x0010, 0x001f]);
    content.extend(utf16_units(DOC_RUN2));

    let length = content.len() as u16;
    let mut bytes: Vec<u8> = Vec::new();
    extend_units(
        &mut bytes,
        &[0x5373, 0x6d67, 0x562e, 0x3031, 0x0000, 0x0001, 0x0000, 0x0100, 0x0000, 0x0002],
    );
    extend_units(&mut bytes, &[0x5465, 0x7874, 0x562e, 0x3031]); // "TextV.01"
    extend_units(&mut bytes, &[0x0000, length]);
    extend_units(&mut bytes, &content);
    bytes
}

const LAYOUT_BOX_BLOCK_PITCH_WORDS: usize = 128;

/// 実測レイアウト（houmukyoku-content-001188695.jtd）を模した /LayoutBoxText ペイロード。
/// SsmgV.01 ヘッダ（w[9]=ブロック数）の後に、語ピッチ 128（256 byte）で
/// "TextV.01" + [0x0000, span 長] + span 内容（raw 前置き / 0x001c…0x001f レコード /
/// 0x001d…0x001e インラインテキスト）が並び、ブロック末尾は 0x0000 パディング。
/// 末尾には位置/所有テーブル列（ printable な語を含み、出力へ漏れてはならない）が続く。
fn layout_box_text_payload() -> Vec<u8> {
    // ブロック1: 領収欄ラベル（raw 前置き + 改行）＋ 実験で観測した記録本体の
    // 座標ジャンク（0x00be/0x020d 等 printable な二進語）を含む 0x001c レコードを
    // スキップして、インライン(run)テキストのみを拾う。
    let mut block1: Vec<u16> = Vec::new();
    block1.extend(utf16_units(&format!("{BOX_LABEL1}\n")));
    block1.extend_from_slice(&[
        0x001c, 0x0000, 0x000c, 0x0000, 0x0007, 0x00be, 0x020d, 0x0000, 0x000c, 0x0000, 0x0000,
        0x001f,
    ]);
    block1.extend_from_slice(&[0x001c, 0x0001, 0x0007, 0x0000, 0x0000, 0x0003, 0x001d]);
    block1.extend(utf16_units(BOX_INLINE1));
    block1.push(0x001e);

    // ブロック2: マーカーの無い生テキストのみ（実データの「御　中」ブロックと同型）。
    let block2: Vec<u16> = utf16_units(BOX_RAW2);

    let mut bytes: Vec<u8> = Vec::new();
    extend_units(
        &mut bytes,
        &[0x5373, 0x6d67, 0x562e, 0x3031, 0x0000, 0x0002, 0x0000, 0x0100, 0x0000, 0x0002],
    );
    for block in [&block1, &block2] {
        extend_units(&mut bytes, &[0x5465, 0x7874, 0x562e, 0x3031]); // "TextV.01"
        extend_units(&mut bytes, &[0x0000, block.len() as u16]);
        extend_units(&mut bytes, block);
        let used = 6 + block.len();
        extend_units(
            &mut bytes,
            &vec![0x0000u16; LAYOUT_BOX_BLOCK_PITCH_WORDS - used],
        );
    }
    // 末尾の位置/所有テーブル列（実データ同様の dword 列。'c' 'I' 等の printable
    // な語を含むため、枠領域の境界を誤ると出力へ漏れる）。
    extend_units(
        &mut bytes,
        &[
            0x0000, 0x0063, 0x0000, 0x0001, 0x0000, 0x0001, 0x0000, 0x0000, 0x0000, 0x0049,
            0x0000, 0x0001,
        ],
    );
    bytes
}

fn cfb_with_streams(streams: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    for (path, payload) in streams {
        compound
            .create_stream(path)
            .unwrap()
            .write_all(payload)
            .unwrap();
    }
    compound.into_inner().into_inner()
}

fn write_temp_jtd(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rjtd-layout-box-{}-{}.jtd",
        std::process::id(),
        name
    ));
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn cat_appends_layout_box_text_after_body() {
    // /DocumentText（前置き＋マーカー本文）と /LayoutBoxText（枠2ブロック）を
    // 併せ持つ CFB 合成フィクスチャ。現行実装は /LayoutBoxText を一切読まない
    // ため、枠内テキストが欠けて RED になる。
    // 期待出力: 本文 + 「※枠内テキスト」注記 + ブロックを改行で連結した枠テキスト。
    let bytes = cfb_with_streams(&[
        ("/DocumentText", document_text_payload()),
        ("/LayoutBoxText", layout_box_text_payload()),
    ]);
    let path = write_temp_jtd("with-box", &bytes);

    let stdout = run_cat(&path);
    let stdout = String::from_utf8(stdout).unwrap();
    fs::remove_file(&path).ok();

    let body = format!("{DOC_PROLOGUE}\n{DOC_RUN1}\n{DOC_RUN2}");
    let box_text = format!("{BOX_LABEL1}\n{BOX_INLINE1}\n{BOX_RAW2}");
    assert_eq!(
        stdout,
        format!("{body}\n{BOX_TEXT_NOTE}\n{box_text}"),
        "cat が本文 /DocumentText の後に枠内テキストを連結していない"
    );
    assert!(
        !stdout.contains('¾') && !stdout.contains('ȍ'),
        "枠レコード本体の座標ジャンク（0x00be/0x020d）が出力へ漏れている"
    );
    assert!(
        !stdout.contains('c') && !stdout.contains('I'),
        "末尾の位置/所有テーブル列の二進語（'c' 'I'）が出力へ漏れている"
    );
}

#[test]
fn cat_output_unchanged_without_layout_box_text() {
    // 対照: /LayoutBoxText を持たない正常ファイルでは `cat` 出力が不変であること。
    let bytes = cfb_with_streams(&[("/DocumentText", document_text_payload())]);
    let path = write_temp_jtd("no-box", &bytes);

    let stdout = run_cat(&path);
    let stdout = String::from_utf8(stdout).unwrap();
    fs::remove_file(&path).ok();

    let body = format!("{DOC_PROLOGUE}\n{DOC_RUN1}\n{DOC_RUN2}");
    assert!(
        !stdout.contains(BOX_TEXT_NOTE),
        "/LayoutBoxText が無いファイルに枠内テキスト注記を出してはならない"
    );
    assert_eq!(stdout, format!("{DOC_PROLOGUE}\n{DOC_RUN1}\n{DOC_RUN2}"));
}
