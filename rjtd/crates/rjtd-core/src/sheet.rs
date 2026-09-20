use crate::container::read_cfb_stream;
use crate::{Error, Result};

pub const OBJECT_SHEETS_SHEET_INFO_PATH: &str = "/ObjectSheets/SheetInfo";
pub const DOC_SHEET_DOC_ITEM_INFO_PATH: &str = "/ObjectSheets/DocSheet/DocItemInfo";
pub const DOC_SHEET_DOC_ITEM_INFO2_PATH: &str = "/ObjectSheets/DocSheet/DocItemInfo2";
pub const DOC_SHEET_STORAGE_PREFIX: &str = "/ObjectSheets/DocSheet/DOCS_";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetItem {
    pub index: usize,
    pub name: String,
    pub storage_path: String,
    pub original_path: Option<String>,
}

pub type DocumentSheetInfo = SheetItem;

impl SheetItem {
    pub fn new(
        index: usize,
        name: impl Into<String>,
        storage_path: impl Into<String>,
        original_path: Option<String>,
    ) -> Self {
        Self {
            index,
            name: name.into(),
            storage_path: storage_path.into(),
            original_path,
        }
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn storage_path(&self) -> &str {
        &self.storage_path
    }

    pub fn original_path(&self) -> Option<&str> {
        self.original_path.as_deref()
    }

    pub fn document_text_path(&self) -> String {
        if self.storage_path.is_empty() || self.storage_path == "/" {
            "/DocumentText".to_string()
        } else {
            format!("{}/DocumentText", self.storage_path.trim_end_matches('/'))
        }
    }

    pub fn footnote_path(&self) -> String {
        if self.storage_path.is_empty() || self.storage_path == "/" {
            "/Footnote".to_string()
        } else {
            format!("{}/Footnote", self.storage_path.trim_end_matches('/'))
        }
    }
}

pub fn has_multiple_sheets(data: &[u8]) -> bool {
    read_cfb_stream(data, OBJECT_SHEETS_SHEET_INFO_PATH).is_ok()
        || read_cfb_stream(data, DOC_SHEET_DOC_ITEM_INFO_PATH).is_ok()
}

pub fn read_document_sheets(data: &[u8]) -> Result<Vec<SheetItem>> {
    let sheet_info_bytes = match read_cfb_stream(data, OBJECT_SHEETS_SHEET_INFO_PATH) {
        Ok(bytes) => Some(bytes),
        Err(Error::NotFound(_)) => None,
        Err(err) => return Err(err),
    };

    let doc_item_info_bytes = match read_cfb_stream(data, DOC_SHEET_DOC_ITEM_INFO_PATH) {
        Ok(bytes) => Some(bytes),
        Err(Error::NotFound(_)) => None,
        Err(err) => return Err(err),
    };

    if sheet_info_bytes.is_none() && doc_item_info_bytes.is_none() {
        return Ok(Vec::new());
    }

    let original_paths = match read_cfb_stream(data, DOC_SHEET_DOC_ITEM_INFO2_PATH) {
        Ok(bytes) => parse_doc_item_info2(&bytes),
        Err(_) => Vec::new(),
    };

    let root_name = sheet_info_bytes
        .as_deref()
        .and_then(parse_sheet_info_root_name)
        .unwrap_or_else(|| "タイトル".to_string());

    let mut sheets = Vec::new();
    sheets.push(SheetItem::new(0, root_name, "", None));

    if let Some(bytes) = doc_item_info_bytes.as_deref() {
        let sub_sheets = parse_doc_item_info(bytes)?;
        for (i, (name, docs_id)) in sub_sheets.into_iter().enumerate() {
            let original_path = original_paths.get(i).cloned();
            let storage_path = format!("{DOC_SHEET_STORAGE_PREFIX}{docs_id:04}");
            sheets.push(SheetItem::new(
                sheets.len(),
                name,
                storage_path,
                original_path,
            ));
        }
    }

    Ok(sheets)
}

fn parse_sheet_info_root_name(data: &[u8]) -> Option<String> {
    if data.len() < 12 {
        return None;
    }
    let mut offset = data.len().saturating_sub(8);
    while offset >= 4 {
        if offset + 4 <= data.len() {
            let len = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            if (1..=64).contains(&len) && offset + 4 + len * 2 <= data.len() {
                let slice = &data[offset + 4..offset + 4 + len * 2];
                let u16_units: Vec<u16> = slice
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                if u16_units.iter().all(|&u| u >= 0x20 && u < 0xd800) {
                    if let Ok(s) = String::from_utf16(&u16_units) {
                        return Some(s);
                    }
                }
            }
        }
        offset = match offset.checked_sub(2) {
            Some(o) => o,
            None => break,
        };
    }
    None
}

fn parse_doc_item_info(data: &[u8]) -> Result<Vec<(String, u32)>> {
    if data.len() < 4 {
        return Err(Error::InvalidData("DocItemInfo stream too short".into()));
    }
    let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let mut offset = 4;
    let mut results = Vec::with_capacity(count);

    for _ in 0..count {
        if offset + 8 > data.len() {
            return Err(Error::InvalidData("DocItemInfo entry header truncated".into()));
        }
        let name_len = u32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]) as usize;
        offset += 8;

        let name_bytes_len = name_len * 2;
        if offset + name_bytes_len + 4 > data.len() {
            return Err(Error::InvalidData("DocItemInfo name truncated".into()));
        }
        let u16_units: Vec<u16> = data[offset..offset + name_bytes_len]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let name = String::from_utf16(&u16_units)
            .map_err(|_| Error::InvalidData("DocItemInfo name invalid UTF-16".into()))?;
        offset += name_bytes_len;

        let docs_id = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        offset += 4;

        const TRAILER_LEN: usize = 26;
        if offset + TRAILER_LEN > data.len() && results.len() + 1 < count {
            return Err(Error::InvalidData("DocItemInfo trailer truncated".into()));
        }
        offset = (offset + TRAILER_LEN).min(data.len());

        results.push((name, docs_id));
    }

    Ok(results)
}

fn parse_doc_item_info2(data: &[u8]) -> Vec<String> {
    let mut paths = Vec::new();
    let mut offset = 0;
    while offset + 4 <= data.len() {
        let len = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        if (1..=512).contains(&len) && offset + 4 + len * 2 <= data.len() {
            let slice = &data[offset + 4..offset + 4 + len * 2];
            let u16_units: Vec<u16> = slice
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            if let Ok(s) = String::from_utf16(&u16_units) {
                if s.contains('\\') || s.contains('/') || s.contains(".jtd") {
                    paths.push(s);
                    offset += 4 + len * 2;
                    continue;
                }
            }
        }
        offset += 2;
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_doc_item_info_synthetic() {
        let mut data = Vec::new();
        data.extend_from_slice(&2u32.to_le_bytes());

        data.extend_from_slice(&1u32.to_le_bytes());
        let name1 = "午後の授業".encode_utf16().collect::<Vec<_>>();
        data.extend_from_slice(&(name1.len() as u32).to_le_bytes());
        for u in name1 {
            data.extend_from_slice(&u.to_le_bytes());
        }
        data.extend_from_slice(&3u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 26]);

        data.extend_from_slice(&1u32.to_le_bytes());
        let name2 = "活版所".encode_utf16().collect::<Vec<_>>();
        data.extend_from_slice(&(name2.len() as u32).to_le_bytes());
        for u in name2 {
            data.extend_from_slice(&u.to_le_bytes());
        }
        data.extend_from_slice(&4u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 26]);

        let res = parse_doc_item_info(&data).unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0], ("午後の授業".to_string(), 3));
        assert_eq!(res[1], ("活版所".to_string(), 4));
    }
}
