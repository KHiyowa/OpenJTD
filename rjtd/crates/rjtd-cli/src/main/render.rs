use rjtd_export::to_plain_text;
use rjtd_model::parse_document;

use crate::input::read_file;

use super::render_support::*;
use super::support::*;

pub(crate) fn run_export(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let path = required_path(args.next(), "export")?;
    let options = export_options(args)?;
    let bytes = read_file(&path)?;
    let document = parse_document(&bytes).map_err(|error| error.to_string())?;

    if let Some(ref sheet_spec) = options.sheet {
        let sheet = if let Ok(idx) = sheet_spec.parse::<usize>() {
            document.sheets().get(idx)
        } else {
            document.sheet_by_name(sheet_spec)
        };
        let Some(sheet) = sheet else {
            return Err(format!("sheet `{sheet_spec}` not found"));
        };
        match options.format.as_str() {
            "txt" | "text" => {
                let text = if let Some(fn_text) = sheet.footnote_text() {
                    format!("{}\n\n{}", sheet.text().trim_end(), fn_text.trim())
                } else {
                    sheet.text().to_string()
                };
                write_stdout(&text)?;
            }
            other => {
                return Err(format!(
                    "exporting specific sheet with format `{other}` is not supported yet (use text)"
                ));
            }
        }
        return Ok(());
    }

    match options.format.as_str() {
        "txt" | "text" => write_stdout(&to_plain_text(&document))?,
        other => return Err(format!("unsupported export format: {other}")),
    }
    Ok(())
}
