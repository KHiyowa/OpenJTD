use rjtd_core::document_text::read_document_text_payload;
use rjtd_core::layout_box_text::read_layout_box_text;

use crate::input::read_file;

use super::support::*;

pub(crate) fn run_cat(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let path = required_path(args.next(), "cat")?;
    let bytes = read_file(path)?;
    let payload = read_document_text_payload(&bytes).map_err(|error| error.to_string())?;
    write_stdout(payload.text())?;

    // P2（PARTIAL-LOSS-REPORT.md）: /LayoutBoxText（囲み枠テキスト）が非空のときのみ、
    // 本文の後に「※枠内テキスト」注記を添えて枠テキストを連結する。/LayoutBoxText 欠落・
    // 抽出空・CFB 読み取り失敗時は cat を成功させ従来出力のまま（枠注記を出さない）。
    if let Ok(Some(box_text)) = read_layout_box_text(&bytes)
        && !box_text.text().trim().is_empty()
    {
        write_stdout("\n")?;
        write_stdout_line("※枠内テキスト")?;
        write_stdout(&box_text.text())?;
    }
    Ok(())
}
