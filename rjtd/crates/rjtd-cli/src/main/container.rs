use rjtd_core::container::{
    EntryKind, inspect_cfb_directory, inspect_cfb_entries, inspect_cfb_overview, read_cfb_stream,
};
use rjtd_core::document_text::{
    COMPRESSED_DOCUMENT_PATH, DOCUMENT_TEXT_PATH, EMBEDDED_DOCUMENT_TEXT_PATH,
    read_document_text_payload,
};
use rjtd_core::format::detect_format;

use crate::input::read_file;

use super::container_support::{
    format_cfb_id, format_sector_ids, print_entry_size, write_cfb_chain,
};
use super::support::{
    escaped_path, escaped_text, required_path, unescaped_path, write_stdout_bytes,
    write_stdout_line,
};

pub(crate) fn run_streams(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let path = required_path(args.next(), "streams")?;
    let bytes = read_file(&path)?;
    let entries = inspect_cfb_entries(&bytes).map_err(|error| error.to_string())?;
    for entry in entries {
        write_stdout_line(&format!(
            "{}\t{}\t{}",
            entry.kind().as_str(),
            entry.size(),
            escaped_path(entry.path())
        ))?;
    }
    Ok(())
}

pub(crate) fn run_info(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let path = required_path(args.next(), "info")?;
    let bytes = read_file(&path)?;
    let format = detect_format(&bytes);
    write_stdout_line(&format!("format\t{}", format.as_str()))?;

    if format.as_str() == "unknown" {
        return Ok(());
    }

    let entries = inspect_cfb_entries(&bytes).map_err(|error| error.to_string())?;
    let stream_count = entries
        .iter()
        .filter(|entry| entry.kind() == EntryKind::Stream)
        .count();
    let storage_count = entries
        .iter()
        .filter(|entry| entry.kind() == EntryKind::Storage)
        .count();
    write_stdout_line(&format!("streams\t{stream_count}"))?;
    write_stdout_line(&format!("storages\t{storage_count}"))?;
    print_entry_size(&entries, DOCUMENT_TEXT_PATH, "document_text_bytes")?;
    print_entry_size(
        &entries,
        COMPRESSED_DOCUMENT_PATH,
        "compressed_document_bytes",
    )?;
    if !entries
        .iter()
        .any(|entry| entry.path() == DOCUMENT_TEXT_PATH)
        && read_document_text_payload(&bytes)
            .is_ok_and(|payload| payload.source_name() == EMBEDDED_DOCUMENT_TEXT_PATH)
    {
        write_stdout_line("embedded_document_text\tpresent")?;
    }
    Ok(())
}

pub(crate) fn run_dump_stream(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let path = required_path(args.next(), "dump-stream")?;
    let stream_path = required_path(args.next(), "dump-stream")?;
    let stream_path = unescaped_path(&stream_path)?;
    let bytes = read_file(path)?;
    let stream = read_cfb_stream(&bytes, &stream_path).map_err(|error| error.to_string())?;
    write_stdout_bytes(&stream)?;
    Ok(())
}

pub(crate) fn run_cfb_map(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let path = required_path(args.next(), "cfb-map")?;
    let bytes = read_file(path)?;
    let overview = inspect_cfb_overview(&bytes).map_err(|error| error.to_string())?;
    write_stdout_line(&format!("sector_size\t{}", overview.sector_size()))?;
    write_stdout_line(&format!(
        "mini_stream_cutoff\t{}",
        overview.mini_stream_cutoff()
    ))?;
    write_stdout_line(&format!(
        "fat_sectors\t{}\t{}",
        overview.fat_sector_ids().len(),
        format_sector_ids(overview.fat_sector_ids())
    ))?;
    write_cfb_chain("directory_chain", overview.directory_chain())?;
    write_cfb_chain("mini_fat_chain", overview.mini_fat_chain())?;
    write_stdout_line(&format!(
        "root_mini_stream\t{}\t{}",
        overview.root_start_sector(),
        overview.root_size()
    ))?;
    write_cfb_chain("mini_stream_chain", overview.mini_stream_chain())?;
    Ok(())
}

pub(crate) fn run_cfb_dir(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let path = required_path(args.next(), "cfb-dir")?;
    let bytes = read_file(path)?;
    let entries = inspect_cfb_directory(&bytes).map_err(|error| error.to_string())?;
    for entry in entries {
        write_stdout_line(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            entry.id(),
            entry.kind().as_str(),
            entry.size(),
            entry.start_sector(),
            format_cfb_id(entry.left_id()),
            format_cfb_id(entry.right_id()),
            format_cfb_id(entry.child_id()),
            escaped_path(entry.path().unwrap_or("-")),
            escaped_text(entry.name()),
            entry.name().encode_utf16().count()
        ))?;
    }
    Ok(())
}

