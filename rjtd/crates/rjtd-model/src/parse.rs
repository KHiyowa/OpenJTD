use rjtd_core::{ParseLimits, ResourceBudget, Result};

use crate::{Document, IchitaroParser};

pub fn parse_document(data: &[u8]) -> Result<Document> {
    parse_document_with_limits(data, ParseLimits::DEFAULT)
}

/// Parses an already allocated document with explicit resource limits.
///
/// `max_decompressed_bytes` applies to each LH5 member, while the budget created here applies
/// `max_total_decompressed_bytes` across all members reached during this parse. Input limits
/// validate `data` after the caller has allocated it and therefore cannot reclaim that memory.
pub fn parse_document_with_limits(data: &[u8], limits: ParseLimits) -> Result<Document> {
    let mut budget = limits.resource_budget();
    parse_document_with_budget(data, &mut budget)
}

fn parse_document_with_budget(data: &[u8], budget: &mut ResourceBudget) -> Result<Document> {
    budget.check_input_size(data.len())?;
    IchitaroParser.parse_with_budget(data, budget)
}
