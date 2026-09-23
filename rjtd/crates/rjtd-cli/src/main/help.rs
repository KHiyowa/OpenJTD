use super::support::write_stdout;

pub(crate) fn print_help() -> Result<(), String> {
    write_stdout(
        "\
rjtd

Rust-based Ichitaro (JTD) Document Engine

Usage:
  rjtd streams <file.jtd>
  rjtd info <file.jtd>
  rjtd dump-stream <file.jtd> <stream-path>
  rjtd cfb-map <file.jtd>
  rjtd cfb-dir <file.jtd>
  rjtd cat <file.jtd>
  rjtd sheets <file.jtd>
  rjtd export <file.jtd> [-f|--format <txt|text>] [-s|--sheet <name|index>]
",
    )
}
