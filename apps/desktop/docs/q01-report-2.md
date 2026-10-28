# Q01 機器なしの通し試験・2回目（2026-10-28、Windows、段階1〜9 の後）

**すべて合成源による確認。** Live、Launchpad X、MIDI ポートは使っていない。手順と項目は [1回目](q01-report.md) と同じで、ここでは差分と結果だけを書く。ここに書く数値は実機・Live の結果ではない。

- 対象：段階1〜9（docs/14 §4）を入れた版（commit 5f43a36 の次）、**release build**（`target\release`）。1回目は debug build。
- 手順の再現：
  - headless：`target\release\omoiiro-probe q01`。結果は `q01-files/q01-headless-result.json`。
  - GUI：`pwsh apps/desktop/scripts/q01-gui.ps1 -Scenario all -Release -BridgePort 9017`。結果は `q01-files/q01-gui-result.jsonl`。
- このPCでは Live（旧ブリッジ）が起動していて 127.0.0.1:9007 へ送り続けていたため、GUI の追従は別ポートで行った（下の「スクリプトの変更」）。

## 結果

| 番号 | 項目 | 結果 |
| --- | --- | --- |
| 1.1〜1.6 | 新規、プリセット、clip 配置、安全警告と解除、名前を付けて保存（headless） | PASS（1.6 は 3,099 bytes） |
| G1.1 | GUI：新規ショーと編集 | PASS |
| G1.2 | GUI：名前を付けて保存（Windows の保存ダイアログ） | **PASS（1回目は未達）**。ファイルができ、`render` で読め、「保存済み」表示 |
| 2.1〜2.3 | 再読込、無変更の保存でバイト一致、Undo 履歴 | PASS |
| G2.1 | GUI：再読込と無変更の保存 | PASS（ハッシュ一致） |
| 3.1 | 任意位置プレビュー = render（6時点） | PASS（最大差 1） |
| G3.1 | GUI：プレビューの画素 = render（12.345 s） | PASS（最大差 0） |
| 4.1〜4.8 | 合成ブリッジで追従、追従中の編集、Blackout、明るさ、輝度の天井、操作経路の fault と reset、全ポート再接続、停止 | PASS（4.8：363 frame、MTC 1,446件） |
| G4.1 | GUI：追従、編集の反映、Blackout、再接続 | PASS（ポート 9017） |
| 5.1〜5.4 | 壊れたファイル、未対応の版 0.0.9 / 0.2.0、大きすぎるファイル | PASS（5.2 / 5.3 の文言は「this build reads 0.1.3」） |
| G5.1 | GUI：未保存で閉じる → キャンセル → 保存せず終了 | PASS |
| 6.1 | headless 追従 12 s（release） | jitter p95 0.538 ms、render p95 0.019 ms、LED 送信 p95 0.006 ms、飛ばした frame 0 |

- headless 23項目、GUI 6項目（G1.1〜G5.1）がすべて PASS。

## G6.1 10分 soak（重いショー、合成源）

- 手順：`pwsh apps/desktop/scripts/q01-gui.ps1 -Scenario soak -SoakSeconds 600 -SampleSeconds 60 -BaselineSeconds 60 -Release -BridgePort 9017 -Meter -Show <repo>\fixtures\shows\soak-heavy-0.1.3.show.json`
- ショー：`fixtures/shows/soak-heavy-0.1.3.show.json`（8レイヤー、15分）。含まれるもの：
  - 粒子2層：常時の噴水（64個/秒・6秒・bounce）と、繰り返す火花
  - 8フレームの輪
  - 繰り返す文字のスクロール
  - 記録の繰り返しで叩く、熱・余熱・星座の3層（記録は2秒で8点）
  - 合成 meter の glow
- ブリッジ：`fake_bridge.py --steady 630 --meter`（合成の meter を約30 Hz）。LED はプロセス内の仮想 Launchpad X。
- 結果：**PASS**。

| 経過 | omoiiro.exe | 子孫プロセス込み（7プロセス） |
| --- | --- | --- |
| 10 s | 43.4 MB | 433.6 MB |
| 61 s（基準） | 43.7 MB | 493.3 MB |
| 300 s | 44.2 MB | 516.3 MB |
| 541 s | 44.2 MB | 529.3 MB |
| 600 s | 44.3 MB | 524.5 MB |

- 子孫プロセス込みの増加は +31.2 MB（6.3%）。判定の条件（10% 以内かつ +50 MB 以内、単調増加なし）を満たした。
- omoiiro.exe 本体は 43.4〜44.3 MB でほぼ横ばいだった。入力履歴と記録の展開には上限（1,024件・60 s）があり、PadLog は録音中か JSONL 出力中にしか溜まらない。そのため本体が増え続けないことと合っている。
- 増加の大半は WebView2 の子孫プロセスで、後半は 516〜529 MB の間で頭打ちになった。
- 標本：`q01-files/soak-samples-20261028-210635.jsonl`。

## スクリプトの変更（`apps/desktop/scripts/q01-gui.ps1`）

- `-Release`：`target\release` のアプリと probe を使う（既定は従来どおり debug）。
- `-BridgePort <port>`：アプリのブリッジ受信アドレス（端末設定 `bridge_bind`）と `fake_bridge.py --port` を、このポートにそろえる。端末設定は従来どおり試験の前に退避し、後で戻す。ほかの送り手（起動中の Live）が 9007 を使っているときに使う。
- G1.2 を無人で通せるようにした。保存ダイアログのファイル名欄は、UI Automation では Edit ではなく Pane（クラス `Edit`、ID 1001）として見える。そこで Win32 の `WM_SETTEXT` で値を入れ、保存ボタン（クラス `Button`、ID 1）に `BM_CLICK` を送る。どちらもキーボードの焦点が要らない。キー入力は、この方法が使えないときだけの予備として残した。1回目に入力が届かなかった原因は、試験のプロセスが前面になれないため、キー入力がほかの窓に行っていたことと考えられる。
- G1.2 の結果に書くパスを `<repo>\...` にした（公開リポジトリに個人の絶対パスを書かない）。

## 画面の変化と試験

段階1〜9 で、タイムラインの「編集対象」、素材パネル、演奏タブの「パッド入力」欄、効果の入力欄などが増えた。G 系が押すボタン名（「タイムライン」「新規ショー」「レイヤーを追加」「常時レイヤーを追加」「新規プリセット」「名前を付けて保存…」「ショーを読み込んで編集」「保存」「ショーを読み込む」「移動」など）は変わっていないので、スクリプトの変更は上の3点だけで済んだ。
