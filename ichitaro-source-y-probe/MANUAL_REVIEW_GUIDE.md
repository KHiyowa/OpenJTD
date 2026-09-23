# 手動レビューガイド

生成日時: 2026-07-05T02:15:05+09:00

このフォルダの `files/` 以下には、現在 Ichitaro の `.jtd` ファイルと Ichitaro から書き出された `.pdf` ファイルが対になった 39 組のペアが含まれています。最新のパスでは、Computer Use で生成された RTF 原本を Ichitaro で開いた後、PDF 書き出しを有効にした状態で `.jtd` として保存し、サンプルを 28 個追加しました。

## 出典

- ネイティブ（Ichitaro UI での直接作成）サンプル: `000_base_a`, `001_base_b_resave`, `010_table_moved_down_small`, `011_table_moved_down_large`, `020_row1_height_plus`, `021_row2_height_plus`, `022_row3_height_plus`, `040_top_margin_plus`, `050_wrapped_one_cell`, `070_two_tables_vertical`, `080_plain_paragraph_lines_only`.
- RTF インポートに基づく代替サンプル: `010a_table_after_1_paragraph`, `011a_table_after_4_paragraphs`, `012a_table_after_8_paragraphs`, `013_table_moved_right`, `030_col1_width_plus`, `031_col2_width_plus`, `032_table_width_plus_both_cols`, `040a_top_margin_20mm`, `040b_top_margin_30mm_baseline`, `040c_top_margin_40mm`, `040d_top_margin_50mm`, `040e_top_margin_60mm`, `053_font_size_table_plus`, `054_font_size_paragraph_plus`, `055_table_cell_line_spacing_plus`, `056_paragraph_line_spacing_plus`, `060_table_3x3`, `061_table_2x5`, `062_table_1x3`, `063_table_4x2`, `064_merged_header`, `066_empty_cells`, `074a_many_paragraphs_then_small_table_page2`, `074b_table_near_page_bottom_no_split`, `074c_table_crosses_page_boundary`, `074d_many_row_table_2col_simple`, `081_plain_paragraph_line_spacing_plus`, `082_plain_paragraph_font_size_plus`.
- 失敗したネイティブサンプル: `074_multi_page_table`。この ID について、汚染されたプレースホルダーのアーティファクトは保存していません。

RTF インポートのサンプルは、広範なパーサー検証や source-y の探索には有用ですが、ネイティブな Ichitaro 作成のセマンティクスを厳密に証明するものではありません。視覚レビューが完了するまでは、生成された代替サンプルとして扱ってください。

## 必ずレビューするか、ネイティブで作り直す項目

- `010a_table_after_1_paragraph`, `011a_table_after_4_paragraphs`, `012a_table_after_8_paragraphs`: table-y 代替サンプルとしてのみ使用してください。正確な Ichitaro オブジェクト移動が必要な場合は、ネイティブな段落挿入と表配置で作り直してください。
- `040a_top_margin_20mm` から `040e_top_margin_60mm` まで: 正確な差分（デルタ）として使う前に、インポートされた余白をネイティブの `040_top_margin_plus` サンプルと対照して検証してください。
- `053_font_size_table_plus`, `054_font_size_paragraph_plus`, `055_table_cell_line_spacing_plus`, `056_paragraph_line_spacing_plus`, `081_plain_paragraph_line_spacing_plus`, `082_plain_paragraph_font_size_plus`: レイアウト基準値として使う前に、インポートされたフォントおよび行間メトリクスを検証してください。
- `064_merged_header`: 現在のサンプルは近似値です。RTF 原本は実際に結合された 1 つのヘッダーセルではなく、2 つの表ブロックを使用しています。
- `074a_many_paragraphs_then_small_table_page2`, `074b_table_near_page_bottom_no_split`, `074c_table_crosses_page_boundary`, `074d_many_row_table_2col_simple`: ページ境界の挙動はレイアウトに敏感です。正確な行の継続、または分割・非分割のセマンティクスが重要な場合は、ネイティブで作り直してください。

## 手動作成手順

1. Ichitaro を開き、新規文書を作成します。
2. OpenJTD/PDF の抽出結果でテキストを安定して比較できるよう、`P01`, `R01C01`, `R02C02` のような単純な ASCII ラベルを使用します。
3. 厳密なセマンティクスが必要な場合は、RTF をインポートせず、Ichitaro の表/罫線ツールで意図した表をネイティブに作成します。
4. 列幅サンプルは、対象の列境界を直接ドラッグし、他の列は固定したままにします。
5. 行高さサンプルは、対象の行境界を直接ドラッグし、前段の行の上面位置は固定したままにします。
6. 余白サンプルは、文書スタイル/ページ設定ダイアログを使用し、正確なミリメートル値を `manifest.csv` に記録します。
7. ページ境界サンプルは、表の前に十分な段落を挿入してから保存し、PDF に書き出した後、表が意図したページで開始するか、継続するか、分割されるかを視覚的に確認します。
8. `ichitaro-source-y-probe/files/` に `.jtd` で保存し、対応する `.pdf` が隣に生成されるよう、Ichitaro の PDF 同時書き出しを有効にしておきます。
9. 空白ページまたは内容の少ないページが意図された場合にのみ、Ichitaro の空白ページ警告を許可します。
10. `*.jtd.$$$` ファイルを削除する前に Ichitaro を閉じます。これらのファイルはコーパスのアーティファクトではなく、ロック/バックアップファイルです。

## UI 補足事項

- アクセシビリティ階層の影響で、メニューラベルが日本語で表示されたり文字化けして見える場合があります。レビュー作業では許容範囲なので、正確なメニューテキストよりも安定した位置とダイアログ構造に頼ってください。
- この自動化ワークフローを再現する場合、RTF インポートの表変換ダイアログでは Word 互換フレームオプションではなく、罫線/表変換オプションを使用する必要があります。
- 体験版またはバックアップの通知が表示された場合は、文書が正しく保存されたかを判定する前に、まず通知を閉じてください。
