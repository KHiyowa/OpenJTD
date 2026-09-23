#![doc = include_str!("../README.md")]

pub mod compressed_document;
pub mod container;
pub mod document_text;
pub mod error;
pub mod format;
pub mod layout_box_text;
pub mod lha;
pub mod limits;
pub mod record;
pub mod sheet;
pub mod stream;

pub use error::{Error, Result};
pub use limits::{DecompressionBudget, ParseLimits, ResourceBudget, ResourceKind};
