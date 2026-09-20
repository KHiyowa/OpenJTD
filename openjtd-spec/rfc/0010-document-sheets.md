# RFC 0010: Document Sheets (Multi-Sheet Structure)

Status: accepted

Observed: 2026-09-20

## Summary

Ichitaro (Justsystem Ichitaro) document files (JTD / JTT) support multi-sheet documents where multiple documents, tables, or notes are stored within a single file in a tabbed interface.

While single-sheet documents store their main body text stream under the root CFB path `/DocumentText`, multi-sheet documents organize content as follows:
- The root `/DocItemInfo` stream contains metadata for each sheet (sheet name, original file path, internal storage identifiers, GUIDs, etc.).
- The text stream and style/layout data for each secondary sheet are stored inside sub-storages under `/ObjectSheets/DocSheet/DOCS_XXXX/` (e.g., `/ObjectSheets/DocSheet/DOCS_0000/DocumentText`).
- The root `/DocumentText` stream typically corresponds to the first (title or primary) sheet.

## CFB Storage Structure

A typical multi-sheet JTD compound file layout:

```text
/DocumentText                          (Primary sheet body text)
/DocItemInfo                           (Sheet metadata list)
/ObjectSheets                          (Storage)
/ObjectSheets/DocSheet                 (Storage)
/ObjectSheets/DocSheet/DOCS_0000       (Storage: Sheet 1)
/ObjectSheets/DocSheet/DOCS_0000/DocumentText
/ObjectSheets/DocSheet/DOCS_0000/DocumentTextPositionTables
/ObjectSheets/DocSheet/DOCS_0000/LineMark
/ObjectSheets/DocSheet/DOCS_0000/PageMark
/ObjectSheets/DocSheet/DOCS_0000/PaperMark
/ObjectSheets/DocSheet/DOCS_0001       (Storage: Sheet 2)
/ObjectSheets/DocSheet/DOCS_0001/DocumentText
...
```

## `/DocItemInfo` Stream Parsing

The `/DocItemInfo` stream sequentially stores sheet item records. Each entry exposes:
1. **Sheet Name**: Display name of the sheet presented to users (UTF-16LE).
2. **Storage Name**: Internal CFB storage identifier (e.g., `DOCS_0000`), resolving to `/ObjectSheets/DocSheet/DOCS_XXXX`.
3. **Original File Path**: Source file path if linked or imported (UNC path or drive path, UTF-16LE).

## CLI (`rjtd`) Support

### `rjtd sheets <file.jtd>`
List sheets present in the document in a tab-separated format.

```sh
rjtd sheets sample.jtd
```

Example output:
```text
sheets	4
sheet	0	タイトル	/	-
sheet	1	一、午後の授業	/ObjectSheets/DocSheet/DOCS_0000	\\server\share\01_gogo.jtd
sheet	2	二、活版所	/ObjectSheets/DocSheet/DOCS_0001	\\server\share\02_kappan.jtd
sheet	3	三、家	/ObjectSheets/DocSheet/DOCS_0002	\\server\share\03_ie.jtd
```

### `rjtd export <file.jtd> --format <text|md> [--sheet <name|index>]`
- When `--sheet` is omitted on a multi-sheet file: concatenates text across all sheets with sheet headers.
- When `--sheet <name|index>` is provided: exports text specifically for that sheet.

```sh
# Export a specific sheet
rjtd export sample.jtd --format text --sheet 一、午後の授業
rjtd export sample.jtd --format text --sheet 1
```

## Web / WASM API

WASM bindings (`HwpDocument`) expose:
- `doc.getSheetCount(): number`
- `doc.getSheets(): string` (JSON array)
  ```json
  [
    {"index": 0, "name": "タイトル", "storagePath": "", "originalPath": null},
    {"index": 1, "name": "一、午後の授業", "storagePath": "/ObjectSheets/DocSheet/DOCS_0000", "originalPath": "\\\\server\\share\\01_gogo.jtd"}
  ]
  ```
- `doc.getSheetPlainText(index: number): string | undefined`
