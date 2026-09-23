#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExportOptions {
    pub(crate) format: String,
    pub(crate) sheet: Option<String>,
}

pub(crate) fn export_options(args: impl Iterator<Item = String>) -> Result<ExportOptions, String> {
    let mut format = None;
    let mut sheet = None;
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--format" | "-f" => {
                let Some(value) = args.next() else {
                    return Err(format!("missing value for `{arg}`"));
                };
                format = Some(value);
            }
            "--sheet" | "-s" => {
                let Some(value) = args.next() else {
                    return Err(format!("missing value for `{arg}`"));
                };
                sheet = Some(value);
            }
            other => {
                return Err(format!(
                    "unexpected export argument `{other}`; usage: rjtd export <file> [-f|--format <txt|text>] [-s|--sheet <name|index>]"
                ));
            }
        }
    }

    Ok(ExportOptions {
        format: format.ok_or_else(|| {
            "usage: rjtd export <file> [-f|--format <txt|text>] [-s|--sheet <name|index>]".to_string()
        })?,
        sheet,
    })
}
