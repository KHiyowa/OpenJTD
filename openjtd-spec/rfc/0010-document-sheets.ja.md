# RFC 0010: Document Sheets (マルチシート構造)

Status: accepted

Observed: 2026-09-20

## 概要

一太郎（Justsystem 一太郎）の文書ファイル（JTD / JTT）には、複数の文書や表、メモを1つのファイル内にタブ形式で保持できる「シート機能（マルチシート）」が存在する。

単一シート文書ではルート直下の `/DocumentText` に本文ストリームが配置されるが、マルチシート文書では以下の構造をとる：
- ルートの `/DocItemInfo` ストリームに各シートのメタデータ（シート名、元ファイルパス、内部ストレージ識別子など）が格納される。
- 各シートの本文およびスタイル・レイアウトデータは `/ObjectSheets/DocSheet/DOCS_XXXX/` 以下の各サブストレージに格納される（例: `/ObjectSheets/DocSheet/DOCS_0000/DocumentText`）。
- ルート直下の `/DocumentText` には、通常先頭シート（メインシート / タイトルシート）の内容が格納される。

## CFB ストレージ構造

マルチシート JTD 文書の代表的な CFB エントリ配置例：

```text
/DocumentText                          (メインシートの本文)
/DocItemInfo                           (シート一覧メタ情報)
/ObjectSheets                          (Storage)
/ObjectSheets/DocSheet                 (Storage)
/ObjectSheets/DocSheet/DOCS_0000       (Storage: シート1)
/ObjectSheets/DocSheet/DOCS_0000/DocumentText
/ObjectSheets/DocSheet/DOCS_0000/DocumentTextPositionTables
/ObjectSheets/DocSheet/DOCS_0000/LineMark
/ObjectSheets/DocSheet/DOCS_0000/PageMark
/ObjectSheets/DocSheet/DOCS_0000/PaperMark
/ObjectSheets/DocSheet/DOCS_0001       (Storage: シート2)
/ObjectSheets/DocSheet/DOCS_0001/DocumentText
...
```

## `/DocItemInfo` ストリームの解析

`/DocItemInfo` ストリームには、シート定義レコードが順次格納されている。各エントリには以下の情報が含まれる：
1. **シート名 (Sheet Name)**: ユーザーが表示・編集するシートの表示名（UTF-16LE）。
2. **ストレージ名 (Storage Name)**: 内部CFBストレージの識別子（例: `DOCS_0000`）。フルパスは `/ObjectSheets/DocSheet/DOCS_XXXX` となる。
3. **オリジナルパス (Original File Path)**: 外部ファイルからシートとして追加・リンクされた場合、元のファイルパス（UNCパスまたはWindowsドライブパス、UTF-16LE）。

## CLI ツール (`rjtd`) の対応

### `rjtd sheets <file.jtd>`
文書内に含まれるシート一覧を表示する。

```sh
rjtd sheets sample.jtd
```

出力例（タブ区切り）:
```text
sheets	4
sheet	0	タイトル	/	-
sheet	1	一、午後の授業	/ObjectSheets/DocSheet/DOCS_0000	\\server\share\01_gogo.jtd
sheet	2	二、活版所	/ObjectSheets/DocSheet/DOCS_0001	\\server\share\02_kappan.jtd
sheet	3	三、家	/ObjectSheets/DocSheet/DOCS_0002	\\server\share\03_ie.jtd
```

### `rjtd export <file.jtd> --format <text|md> [--sheet <name|index>]`
- `--sheet` 未指定時: 複数シートが存在する場合、各シートのテキストを順に見出し付きで連結して出力する。
- `--sheet <name|index>` 指定時: 対象のシートのテキストのみを出力する。

```sh
# 特定シートのみ出力
rjtd export sample.jtd --format text --sheet 一、午後の授業
rjtd export sample.jtd --format text --sheet 1
```

## Web / WASM API

WASM バインディング (`HwpDocument`) では以下の API を提供する：
- `doc.getSheetCount(): number`
- `doc.getSheets(): string` (JSON配列)
  ```json
  [
    {"index": 0, "name": "タイトル", "storagePath": "", "originalPath": null},
    {"index": 1, "name": "一、午後の授業", "storagePath": "/ObjectSheets/DocSheet/DOCS_0000", "originalPath": "\\\\server\\share\\01_gogo.jtd"}
  ]
  ```
- `doc.getSheetPlainText(index: number): string | undefined`
