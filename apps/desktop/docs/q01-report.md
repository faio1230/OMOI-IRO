# Q01 機器なしの通し試験（2026-10-28、Windows）

**すべて合成源による確認。** Live、Launchpad X、MIDI ポートは使っていない。ブリッジは `live-bridge/sim/fake_bridge.py`（GUI）またはプロセス内の合成 UDP 送信（headless）、LED はプロセス内の仮想 Launchpad X、操作経路とタイムコード出力は fake MIDI backend（headless）。ここに書く数値は実機・Live の結果ではない。

- 環境：Windows 11 Pro 25H2、Rust 1.94.1、debug build（`target\debug`）、WebView2 155 / 156。
- 手順の再現：
  - headless：`omoiiro-probe q01`（`apps/probe/src/q01_cmd.rs`）。結果は `q01-files/q01-headless-result.json`。
  - GUI：`pwsh apps/desktop/scripts/q01-gui.ps1 -Scenario all`、メモリーは `-Scenario soak -SoakSeconds 600`（UI Automation。端末設定ディレクトリーを退避して戻す）。結果は `q01-files/q01-gui-result.jsonl`（試行ごとに追記。下の表は最終結果）。
- 使ったファイル（`apps/desktop/docs/q01-files/`）：`q01-sample.show.json`（手順1で作ったサンプル）、`q01-broken.show.json`（意味検証で失敗）、`q01-version-0.0.9.show.json` / `q01-version-0.2.0.show.json`（未対応の版）。大きすぎるファイル（16 MB + 1 byte）は試験中に作って消した。

## 1. サンプル作成（空の状態から）と保存

| 番号 | 手順 | 期待値 | 結果 |
| --- | --- | --- | --- |
| 1.1 | 新規ショー | 空のショー（レイヤー0、プリセット1）が検証を通り、未保存 | PASS（headless） |
| G1.1 | GUI：新規ショー → レイヤー追加 → 常時レイヤー追加 → 新規プリセット | 「新規・未保存」「未保存の変更あり」が出る | PASS（UI Automation） |
| 1.2 | プリセット新規（Solid / Chase / Pulse 各1） | 3種が追加される | PASS |
| 1.3 | clip 配置：背景 0〜20 s（フェード 1 s / 2 s）、同じレイヤーに重なる白 10〜14 s、Chase 5〜15 s（add、80%、フェード 0.5 s）、常時レイヤー（Pulse、30%） | 検証を通る | PASS（履歴15件） |
| 1.4 | 安全警告：Pulse を 0.25 拍周期にする | 120 bpm で 8 Hz > 3 Hz、警告1件（理由と値） | PASS「Pulse 0.25 拍周期 × 120.0 bpm = 8.00 Hz（上限 3 Hz）」 |
| 1.5 | 1拍周期に戻す | 警告0件 | PASS |
| 1.6 | 名前を付けて保存（headless：一時ファイル→置換） | 保存され、未保存の印が消える | PASS（3051 bytes） |
| G1.2 | GUI：名前を付けて保存（Windows の保存ダイアログ） | ファイルができ、読み込める | **未達（要手動確認）**。ダイアログは開くが、無人の自動操作ではファイル名欄に入力できなかった（UI Automation で欄が見つからず、キー入力も届かない）。保存処理そのものは 1.6、G2.1、desktop のテスト（`save_is_atomic_stable_and_clears_dirty`、`files::tests`）で確認済み。手動で「名前を付けて保存」→ ファイル名 → 保存、を確認する |

## 2. 再読込

| 番号 | 手順 | 期待値 | 結果 |
| --- | --- | --- | --- |
| 2.1 | 保存したファイルを読み直す | 保存前と同じショー | PASS |
| 2.2 | 無変更で保存 | バイト一致（差分なし） | PASS（3051 bytes 一致） |
| 2.3 | Undo 履歴 | 空から（元に戻す対象なし）、未保存の印なし | PASS |
| G2.1 | GUI：開いて編集用に読み込み → 「保存」 | 「元に戻す」が無効、ファイルのハッシュが不変 | PASS |

## 3. 任意位置プレビュー（GUI 表示 = Rust の Frame）

| 番号 | 手順 | 期待値 | 結果 |
| --- | --- | --- | --- |
| 3.1 | headless：ローカルプレビューを 0 / 0.75 / 5.25 / 10.5 / 12.345678 / 19 s へ seek し、同じ時刻・拍の `omoiiro-probe render` と比べる | 64パッドすべてで差が7bit 量子化の1段（±2）以内 | PASS（最大差 1） |
| G3.1 | GUI：ローカルプレビューを「位置を指定」で 12.345 s へ移動、画面のパッド中央の画素を `render` と比べる | ±3 以内 | PASS（最大差 0） |

プレビューは機器へ送る値（RGB 0〜127）を 0〜255 に戻して表示し、`render` は同じ frame の 8bit 値を出すので、差は量子化の1段までになる。

## 4. 追従（合成ブリッジ）

| 番号 | 手順 | 期待値 | 結果 |
| --- | --- | --- | --- |
| 4.1 | headless：合成 UDP ブリッジで開始 | 3秒以内に locked / follow | PASS |
| 4.2 | 追従中の編集（レイヤーの不透明度）を submit | 次の frame 境界で採用 | PASS（rev 2、1 frame 後） |
| 4.3 | Blackout | LED は黒、Live への転送は続く（Note On が操作出力に届く） | PASS |
| 4.4 | 明るさ 50% | gain 0.5、全チャンネルが 64 以下 | PASS |
| 4.5 | 安全上限（輝度の天井 30%） | 全チャンネルが 38（30%）以下 | PASS（最大 34） |
| 4.6 | 操作経路の fault（大きすぎる入力）→ reset | 原因つきで fault、reset で転送が戻る | PASS（"oversize input"） |
| 4.7 | 全ポート再接続 | タイムコード出力の接続世代が変わり、操作経路は ready に戻る | PASS（世代 1 → 4） |
| 4.8 | 停止 | 鳴っている note の Note Off、要約 | PASS（363 frame、MTC 1444件） |
| G4.1 | GUI：fake_bridge で開始 → タイムラインで編集 → Blackout → 解除 → 全ポート再接続 → 停止 | locked、「Runtime に反映」、暗転の理由 blackout | PASS |

G4.1 の初回は「Runtime に反映」の表示が出ず FAIL だった。原因は画面の不具合：編集直後に状態の取得でビューを読み直すと、`editor_view` が Runtime の revision を返さず表示が消えていた。`editor_view` が追従中の revision を返すよう直し、再試行で PASS（`apps/desktop/src-tauri/src/editor.rs` の `runtime_revision_for`）。

## 5. 失敗系

| 番号 | 手順 | 期待値 | 結果 |
| --- | --- | --- | --- |
| 5.1 | 壊れたファイル（未知のプリセット） | 拒否、JSON Pointer つき | PASS（`/layers/0/content/clips/0/preset_id`） |
| 5.2 | 未対応の版 0.0.9 | 版で拒否 | PASS |
| 5.3 | 未対応の版 0.2.0 | 版で拒否 | PASS |
| 5.4 | 大きすぎるファイル（16 MB + 1） | 拒否 | PASS |
| G5.1 | GUI：編集して閉じる → キャンセル → もう一度閉じる → 保存せずに終了 | 確認が出る、キャンセルで残る、ファイル不変 | PASS |

読込の失敗で今のショーが残ること（GUI）は U07 の確認（`screenshots/u07-load-error.png`）と desktop のテスト（`files::tests::load_errors_carry_pointers_and_keep_the_show`）で見ている。

## 6. 計測（合成源、debug build）

| 番号 | 項目 | 結果 |
| --- | --- | --- |
| 6.1 | headless 追従 12 s（363 frame） | jitter p50 0.30 / p95 0.60 / max 0.87 ms、render p95 0.051 ms（max 0.196）、LED 送信 p95 0.020 ms、render → 送信完了 p95 0.069 ms、飛ばした frame 0、mailbox 上書き 0 |
| G6.1 | GUI：10分間の追従（fake_bridge `--steady`）、メモリー（開始10秒後 → 600秒後の working set） | omoiiro.exe 45.7 → 46.2 MB、直接の子プロセス（WebView2 のブラウザープロセス）込み 172.1 → 184.1 MB。601 s で 18,034 frame（30 fps どおり）、jitter p95 0.52 ms・max 1.60 ms、render p95 0.05 ms（画面の診断、`screenshots/q01-gui-soak.png`）。WebView2 の孫プロセス（レンダラー等）は数えていない。NFR07 の判定には長時間と孫プロセスを含めた計測が要る |

プレビューの間引き：G6.1 の終了時に 18,036 frame を表示して間引き 0（容量1の Channel、画面が ack するまで次を送らない）。

## まとめ

- headless 23項目はすべて PASS。GUI は G1.2 を除く6項目が PASS。G1.2 は手動確認が必要。
- 試験の途中で見つけて直した不具合：`editor_view` が Runtime の revision を返さず、追従中の編集の反映表示が消える（上記 G4.1）。
- 試験のために足したもの：新規ショー（`editor_new`、`omoiiro_core::edit::new_show`）、LEDプレビューの「位置を指定（秒）」、`fake_bridge.py --steady`、`omoiiro-probe q01`、`apps/desktop/scripts/q01-gui.ps1`。
- テスト件数：`cargo test --workspace` 206件 pass、`--features omoiiro-probe/link` 208件 pass。
- スクリーンショット：`apps/desktop/screenshots/q01-gui-new.png`（新規と編集）、`q01-gui-preview.png`（12.345 s のプレビュー）、`q01-gui-follow-edit.png`（追従中の編集）、`q01-gui-follow-blackout.png`、`q01-gui-follow.png`、`q01-gui-close.png`（閉じる確認）、`q01-gui-soak.png`（10分後）。
