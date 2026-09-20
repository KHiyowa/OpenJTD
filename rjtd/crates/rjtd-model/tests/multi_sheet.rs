use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use rjtd_model::parse_document;

fn make_document_text(text: &str) -> Vec<u8> {
    let mut bytes = b"SsmgV.01".to_vec();
    bytes.extend_from_slice(&[0x00, 0x1f]);
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    bytes
}

#[test]
fn synthetic_multi_sheet_parsing() {
    let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
    compound
        .create_stream("/\u{4}JSRV_SegmentInformation")
        .unwrap()
        .write_all(b"segment")
        .unwrap();
    compound
        .create_stream("/DocInfo")
        .unwrap()
        .write_all(b"doc")
        .unwrap();

    // Root document text: 宮沢賢治「銀河鉄道の夜」タイトル
    compound
        .create_stream("/DocumentText")
        .unwrap()
        .write_all(&make_document_text("銀河鉄道の夜\n宮沢賢治"))
        .unwrap();

    // DocItemInfo with 2 sub-sheets
    let mut item_info = Vec::new();
    item_info.extend_from_slice(&2u32.to_le_bytes()); // 2 sheets

    // Sheet 1: "一、午後の授業", DOCS_0000
    item_info.extend_from_slice(&1u32.to_le_bytes());
    let name1: Vec<u16> = "一、午後の授業".encode_utf16().collect();
    item_info.extend_from_slice(&(name1.len() as u32).to_le_bytes());
    for u in name1 {
        item_info.extend_from_slice(&u.to_le_bytes());
    }
    item_info.extend_from_slice(&0u32.to_le_bytes()); // docs_id 0
    item_info.extend_from_slice(&[0u8; 26]);

    // Sheet 2: "二、活版所", DOCS_0001
    item_info.extend_from_slice(&1u32.to_le_bytes());
    let name2: Vec<u16> = "二、活版所".encode_utf16().collect();
    item_info.extend_from_slice(&(name2.len() as u32).to_le_bytes());
    for u in name2 {
        item_info.extend_from_slice(&u.to_le_bytes());
    }
    item_info.extend_from_slice(&1u32.to_le_bytes()); // docs_id 1
    item_info.extend_from_slice(&[0u8; 26]);

    compound.create_storage("/ObjectSheets").unwrap();
    compound.create_storage("/ObjectSheets/DocSheet").unwrap();
    compound
        .create_stream("/ObjectSheets/DocSheet/DocItemInfo")
        .unwrap()
        .write_all(&item_info)
        .unwrap();

    let text1 = "「ではみなさんは、そういうふうに川だと云われたり、乳の流れたあとだと云われたりしていたこのぼんやりと白いものがほんとうは何かご承知ですか。」";
    compound.create_storage("/ObjectSheets/DocSheet/DOCS_0000").unwrap();
    compound
        .create_stream("/ObjectSheets/DocSheet/DOCS_0000/DocumentText")
        .unwrap()
        .write_all(&make_document_text(text1))
        .unwrap();

    let text2 = "ジョバンニは学校の門を出るとき、同じ組の七八人は家へ帰らずカムパネルラをまん中にして校庭の隅の桜の木のところに集まっていました。";
    compound.create_storage("/ObjectSheets/DocSheet/DOCS_0001").unwrap();
    compound
        .create_stream("/ObjectSheets/DocSheet/DOCS_0001/DocumentText")
        .unwrap()
        .write_all(&make_document_text(text2))
        .unwrap();

    let bytes = compound.into_inner().into_inner();
    let doc = parse_document(&bytes).expect("parse synthetic multi-sheet");

    let sheets = doc.sheets();
    assert_eq!(sheets.len(), 3);
    assert_eq!(sheets[0].name(), "タイトル");
    assert_eq!(sheets[1].name(), "一、午後の授業");
    assert_eq!(sheets[2].name(), "二、活版所");

    assert_eq!(sheets[0].text(), "銀河鉄道の夜\n宮沢賢治\n");
    assert_eq!(sheets[1].text(), text1);
    assert_eq!(sheets[2].text(), text2);

    assert_eq!(
        doc.sheet_by_name("一、午後の授業").unwrap().text(),
        text1
    );
    assert_eq!(
        doc.sheet_plain_text(2).unwrap(),
        text2
    );

    let full_text = doc.plain_text();
    assert!(full_text.contains("# タイトル\n\n銀河鉄道の夜"));
    assert!(full_text.contains("# 一、午後の授業\n\n「ではみなさんは"));
    assert!(full_text.contains("# 二、活版所\n\nジョバンニは学校の門を出るとき"));
}

#[test]
fn multi_sheet_document_parsing_and_text_extraction() {
    let jtd_path = "../../../rjtd-testdata/local-samples/修論.jtd";
    if !Path::new(jtd_path).exists() {
        return;
    }
    let bytes = fs::read(jtd_path).expect("read jtd");
    let doc = parse_document(&bytes).expect("parse document");

    let sheets = doc.sheets();
    assert_eq!(sheets.len(), 7, "Expected 7 sheets in sample");

    assert_eq!(sheets[0].name(), "タイトル");
    assert_eq!(sheets[1].name(), "アブストラクト");
    assert_eq!(sheets[2].name(), "研究業績");
    assert_eq!(sheets[3].name(), "目次");
    assert_eq!(sheets[4].name(), "本文");
    assert_eq!(sheets[5].name(), "謝辞");
    assert_eq!(sheets[6].name(), "参考文献");

    assert_eq!(sheets[1].storage_path(), "/ObjectSheets/DocSheet/DOCS_0000");
    assert_eq!(sheets[5].storage_path(), "/ObjectSheets/DocSheet/DOCS_0004");

    // Check text presence of specific sheets without asserting sensitive text
    let abstract_sheet = doc.sheet_by_name("アブストラクト").expect("sheet exists");
    assert!(!abstract_sheet.text().is_empty());

    let acknowledgements_sheet = doc.sheet_by_name("謝辞").expect("sheet exists");
    assert!(!acknowledgements_sheet.text().is_empty());

    // Check full plain text concatenation
    let full_text = doc.plain_text();
    assert!(full_text.contains("# タイトル"));
    assert!(full_text.contains("# アブストラクト"));
    assert!(full_text.contains("# 謝辞"));
}
