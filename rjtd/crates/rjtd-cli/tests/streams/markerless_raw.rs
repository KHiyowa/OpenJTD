use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// このテスト群のローカル・フィクスチャディレクトリ。
///
/// DOC(CFB) に /BodyText がなく /DocumentText 直ストリーム
/// (SsmgV.01 + TextV.01、本文に 0x001c/0x001d/0x001f マーカーなし)
/// のみで本文が格納されている、政府系ウェブ JTD。
/// `golden/` サブディレクトリには macOS textutil で抽出した正解テキスト
/// (`<name>.doc.txt`) を置く。
fn gov_web_fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("rjtd-testdata/local-samples")
        .join("gov-web-jtd")
        .join("needs-doc-pdf-conversion")
}

/// `rjtd cat <path>` を実行し stdout を返す。失敗時は stderr を含めて panick する。
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

#[test]
fn cat_restores_markerless_raw_bodies_via_cfb_document_text() {
    let dir = gov_web_fixture_dir();
    // ローカル・フィクスチャが存在しないときはスキップする
    // (page_mark の local-samples テストと同様の手順)。
    if !dir.exists() {
        return;
    }

    // (フィクスチャ名, 本文の期待文字数)
    let cases = [
        ("env-youshi", 1037usize),
        ("env-youshi-2", 1027usize),
        ("maff-tenpu05-601", 377usize),
        ("maff-tenpu05-602", 297usize),
    ];

    for (name, expected_chars) in cases {
        let fixture = dir.join(format!("{name}.jtd"));
        let golden = dir.join("golden").join(format!("{name}.doc.txt"));
        let golden_bytes = fs::read(&golden).unwrap();

        // golden は macOS textutil 抽出の正解テキストであり、textutil が末尾に
        // 1 文字 (0x0a 1 バイト) だけ余分に付与した終端改行を含む。rjtd cat は
        // write_stdout で末尾改行なしに本文を出力するため、末尾の 0x0a を
        // ちょうど 1 バイトだけ除いてから完全一致を照合する。
        assert!(
            golden_bytes.ends_with(b"\n"),
            "{name}: golden ファイルの末尾に textutil 由来の終端改行がない"
        );
        let expected = golden_bytes[..golden_bytes.len() - 1].to_vec();

        // マーカーレス raw 本文 (SsmgV.01 + TextV.01、本文長 word[15]、
        // 0x001c/0x001d/0x001f なし) のコンテナレベル
        // (CFB /DocumentText 経由) での復元テスト。
        let stdout = run_cat(&fixture);
        assert_eq!(
            stdout, expected,
            "{name}: rjtd cat の出力が golden 本文と完全一致しない"
        );
        assert_eq!(
            String::from_utf8(stdout)
                .unwrap()
                .chars()
                .count(),
            expected_chars,
            "{name}: 本文の文字数が期待値と合わない"
        );
    }
}

#[test]
fn cat_keeps_marker_controlled_body_on_normal_path() {
    // 対照: env-b-2_0.jtd (needs-doc-pdf-conversion の親ディレクトリ) は
    // マーカー型 (0x001f などを含む) のドキュメントであり、通常の復元パス
    // を通るはず。本テストは、新設のマーカーレス raw ゲート
    // (is_markerless_raw_text_span) がマーカー型ドキュメントに誤発火せず
    // 通常パスが不変であることを、コンテナレベル (CFB /DocumentText 経由)
    // で保証するために存在する。
    let path = gov_web_fixture_dir()
        .parent()
        .unwrap()
        .join("env-b-2_0.jtd");
    // ローカル・フィクスチャが存在しないときはスキップする
    // (page_mark の local-samples テストと同様の手順)。
    if !path.exists() {
        return;
    }

    let stdout = run_cat(&path);
    assert_eq!(
        stdout.len(),
        1091,
        "env-b-2_0: rjtd cat 出力のバイト長が通常パスのまま (1091) であること"
    );
    let stdout_text = String::from_utf8(stdout).unwrap();
    // 0x001f マーカー型として復元されている証に stdout が空でないこと。
    assert!(
        !stdout_text.is_empty(),
        "env-b-2_0: rjtd cat の出力は空であってはならない"
    );
    let lines: Vec<&str> = stdout_text.split('\n').collect();
    assert_eq!(
        lines.first().copied(),
        Some("（様式）"),
        "env-b-2_0: 本文の 1 行目"
    );
    assert_eq!(
        lines.get(1).copied(),
        Some("平成17年度　国指定鳥獣保護区の指定等への意見提出用紙"),
        "env-b-2_0: 本文の 2 行目"
    );
}
