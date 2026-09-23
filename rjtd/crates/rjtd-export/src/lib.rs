#![doc = include_str!("../README.md")]

mod text;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub use text::to_plain_text;
