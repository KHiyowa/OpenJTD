use std::io::{self, Write};

use crate::BROKEN_PIPE_EXIT;

pub(crate) fn required_path(path: Option<String>, command: &str) -> Result<String, String> {
    path.ok_or_else(|| format!("missing path for `{command}`"))
}

pub(crate) fn write_stdout(text: &str) -> Result<(), String> {
    write_stdout_bytes(text.as_bytes())
}

pub(crate) fn write_stdout_line(line: &str) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(line.as_bytes()).map_err(stdout_error)?;
    stdout.write_all(b"\n").map_err(stdout_error)
}

pub(crate) fn write_stdout_bytes(bytes: &[u8]) -> Result<(), String> {
    io::stdout().write_all(bytes).map_err(stdout_error)
}

pub(crate) fn stdout_error(error: io::Error) -> String {
    if error.kind() == io::ErrorKind::BrokenPipe {
        BROKEN_PIPE_EXIT.to_string()
    } else {
        format!("cannot write to stdout: {error}")
    }
}

pub(crate) fn escaped_path(path: &str) -> String {
    let mut escaped = String::new();
    for character in path.chars() {
        if character.is_ascii_control() {
            escaped.push_str(&format!("\\x{:02X}", character as u32));
        } else {
            escaped.push(character);
        }
    }
    escaped
}

pub(crate) fn escaped_text(text: &str) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        match character {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_ascii_control() => {
                escaped.push_str(&format!("\\u{:04X}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

pub(crate) fn unescaped_path(path: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut chars = path.chars().peekable();

    while let Some(character) = chars.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }

        match chars.next() {
            Some('x') => {
                let high = chars
                    .next()
                    .ok_or_else(|| "incomplete \\x escape in stream path".to_string())?;
                let low = chars
                    .next()
                    .ok_or_else(|| "incomplete \\x escape in stream path".to_string())?;
                let byte = hex_pair(high, low)?;
                output.push(byte as char);
            }
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
            None => output.push('\\'),
        }
    }

    Ok(output)
}

pub(crate) fn hex_pair(high: char, low: char) -> Result<u8, String> {
    let high = high
        .to_digit(16)
        .ok_or_else(|| format!("invalid hex escape digit: {high}"))?;
    let low = low
        .to_digit(16)
        .ok_or_else(|| format!("invalid hex escape digit: {low}"))?;
    Ok(((high << 4) | low) as u8)
}
