use rjtd_model::parse_document;

use crate::input::read_file;

use super::support::*;

pub(crate) fn run_sheets(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let path = required_path(args.next(), "sheets")?;
    let bytes = read_file(&path)?;
    let document = parse_document(&bytes).map_err(|error| error.to_string())?;
    write_stdout_line(&format!("sheets\t{}", document.sheets().len()))?;
    for sheet in document.sheets() {
        let orig = sheet.original_path().unwrap_or("-");
        write_stdout_line(&format!(
            "sheet\t{}\t{}\t{}\t{}",
            sheet.index(),
            sheet.name(),
            sheet.storage_path(),
            orig
        ))?;
    }
    Ok(())
}
