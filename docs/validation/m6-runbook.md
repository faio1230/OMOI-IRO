# M6 手順書（実機と長時間）

設計担当が実装担当の案（`apps/desktop/docs/m6-runbook-draft.md`、2026-10-28）を正式版として採用。設計判断（同日）：(1) Launchpad へのLED出力は端末設定 `devices.allow_launchpad_led`（既定false）で許可し、画面に「Launchpad X へ直接LEDを送ります。Live の Control Surface に Launchpad を割り当てないでください」と表示する。(2) 2時間試験のメモリー判定は、開始30分後を基準に2時間後までの増加が10%以内かつ絶対値で+50 MB以内、単調増加の傾向なし。(3) Device Inquiry の family と firmware 表記は S01 で実機確認し、違えば codec を直す。記録は `artifacts/m6/`。


2026-10-28、実装担当が作成。設計担当が docs/validation に正式版として転記・修正する前提の案。M6 は利用者の立ち会いが必要なので、ここでは「当日やること」と「事前に揃えておくもの」を分ける。既存の p0-runbook（B4、C、D）と Q01（`apps/desktop/docs/q01-report.md`）を下敷きにしている。

## 0. 共通の準備（当日の前に）

| 項目 | 内容 | 状態 |
| --- | --- | --- |
| ビルド | release：`apps/desktop` で `npx tauri build`（NSIS インストーラー、署名なし）と `cargo build --release -p omoiiro-probe`。debug の Link 付き probe も用意 | 手順は BUILD.md |
| 仮想バス | loopMIDI：`OMOI-IRO Control` / `OMOI-IRO Timecode` / `OMOI-IRO Clock`。起動順は loopMIDI → Live。Live を起動する前に `omoiiro-probe gen-mtc` → `monitor` の折り返しで3ポートが通ることを確認する。通らないとき（loopMIDI の Total data が 0 のまま）は、Live を閉じ、管理者権限で `sc stop midisrv` / `sc start midisrv`（Windows MIDI Services）→ loopMIDI を起動し直す → 折り返しを再確認（2026-10-28 に実績あり。environment.md 参照）。それでも通らなければ PC 再起動 | 作成済み（2026-10-28） |
| Remote Script | `live-bridge/OmoiIroBridge` を User Library の `Remote Scripts\OmoiIroBridge\` へコピー、Live の Control Surface で選ぶ（Input / Output None）。旧 DJSCBridge は None に戻す | 利用者の作業 |
| Live の MIDI 設定 | `OMOI-IRO Clock` の Output：Sync ON。`OMOI-IRO Control` の Input：Remote / Track ON。Link OFF（拍源を Link にする場合も Start Stop Sync は使わない） | 利用者の作業 |
| Launchpad X を出力に使う許可 | アプリは今、Launchpad らしい LED 出力を拒否している（端末設定の確認と Runtime の両方）。M6 の開始時に「端末設定で機器出力を許可する」項目を足して開ける（設計承認のうえ実装担当が入れる）。CLI は `follow --allow-device` で既に可能 | 未（M6 開始時） |
| 機器の確認（Identifying / Configuring） | LED 出力が機器のとき、接続のたびに Device Inquiry → Launchpad X の応答 → Programmer Mode 選択と照会 → Ready まで LED を送らない。ブートローダー、別機種、応答なし（1 s × 3 回）は理由つきの fault。応答は同じ機器の操作入力から受けるので、操作入力と LED 出力に同じ LPX を指定する | 実装済み（fake で検証、2026-10-28） |
| 記録 | `artifacts/m6/` に JSONL と画面。`omoiiro-probe follow --out` で状態記録、アプリは診断表示をスクリーンショット | — |

## 1. Launchpad X（S01 / S02 / S05 / C8）

### S01 ポート・モード・RGB・入力・復帰

1. `omoiiro-probe ports` で LPX の2系統を確認（Windows では DAW 用と MIDI 用で表示名が違う。MIDI 用を使う）。同名のポートが2つあれば `#番号` で選ぶ（アプリは同名を Fault にする）。
2. `omoiiro-probe lpx-check --in "<LPX MIDI 入力>" --out "<LPX MIDI 出力>" --duration 30 --run-id win-s01-lpx --record artifacts\m6\win-s01-lpx.jsonl`（P0 の手順。identity、mode、RGB 1パッド、押下と解放、Live Mode への復帰）。
3. 本番アプリ：接続設定で 操作入力 = LPX、LED 出力 = LPX（M6 で許可を開けた後）。開始すると LED タイルが identifying → configuring → 接続 になることを確認。firmware 版は Device Inquiry の応答から記録（`identity.version_text`）。
4. 停止とウィンドウを閉じる操作の両方で、全消灯と Live Mode 復帰（`F0 00 20 29 02 0C 0E 00 F7`）を確認。Ctrl+C（CLI）でも同じ。
5. 期待：Identifying で別機種やブートローダーなら理由つき fault で LED を送らない。応答がなければ 3 回で fault。

### S02 Control バスから Live の MIDI Map へ

1. Live の MIDI Map モードでパラメーターを1つ選び、LPX のパッドを押す。`OMOI-IRO Control` 経由で割り当たることを確認。
2. 押下・解放を10回、別のパッドで同時押し。Live 側に Note On / Off が重複なく届くこと（アプリの診断：Live へ転送の件数、受信 callback → Live へ送信 p95）。
3. Blackout 中もパッドの操作が Live に届くこと（Blackout は LED のみ）。
4. Live の Control Surface に Launchpad X を割り当てていないこと（アプリと二重にならない）。

### S05 LED 30 fps と入力の併用、send 時間

1. 動きの多いショー（Q01 のサンプル、または Chase を多用したもの）で追従し、パッドを連打しながら 5 分。
2. 記録：LED 送信 p95 / max、render → 送信完了 p95、飛ばした frame、mailbox 上書き、受信 callback → Live 送信 p95、プレビュー間引き。合否の目安：render p95 ≤ 5 ms（NFR03）、LED 送信が 1 frame（33 ms）に収まる、操作の転送が LED に引きずられない（callback → 送信 p95 が 1 ms 程度で変わらない）。
3. 60 fps 比較は show の `render.fps` を変えて同じ記録を取る（任意）。

### C8 USB 抜き差し

1. 追従中に LPX の USB を抜く → LED タイルが「再接続待ち」、操作経路はパッドの入力が消えたので鳴っている note に Note Off を送る（Live に stuck note が残らない）。
2. 挿し直す → 自動で再接続し、Identifying → Configuring → Ready の後に全 LED を送り直す（差分の基準をリセット）。
3. 抜いている間も、時刻（ブリッジ）と MTC 出力には影響がないこと。
4. 「全ポートを再接続」ボタンでも同じ流れになること。

## 2. Live：B4 相当（C1〜C7）を本番アプリで

P0 の B4（DJSCBridge + `mtc-out`）と同じケースを、OmoiIroBridge と本番アプリ（`omoiiro.exe`）で再実施する。Set は `artifacts/p0/sets/djsc-tempo-automation-test.als`（テンポ自動化入り）。

準備：loopMIDI → Live。Control Surface = OmoiIroBridge。`OMOI-IRO Clock` Sync ON。Link OFF。アプリの端末設定：ブリッジ 127.0.0.1:9007、タイムコード出力 = `OMOI-IRO Timecode`（show の `sync.mtc_output.enabled` = true）、MIDI Clock 入力 = `OMOI-IRO Clock`（show の拍源が midi_clock のとき）。

比較用に CLI で受け直す：`omoiiro-probe monitor --mtc-in "OMOI-IRO Timecode" --clock-in "OMOI-IRO Clock" --run-id win-m6-c<N>-mon --out artifacts\m6\win-m6-c<N>-mon.jsonl`。WinMM で同じ入力を2つ開けないときは、アプリの MIDI Clock 入力を外して行う。

| ケース | 操作 | 期待 |
| --- | --- | --- |
| C1 Play / Stop / 再開 | Live で再生・停止・再開 | 時刻源 locked → 停止で hold（show の stop_behavior）→ 再開で follow。MTC は FF(start)→QF、受信 lock 500 ms 以内 |
| C2 seek | 停止中と再生中 | 位置の飛びとして扱い、再生中は FF(jump) → 再 lock 500 ms 以内。LED は新しい位置をすぐ描く |
| C3 短い loop | 4小節 loop | 折返しごとに FF → 再 lock 500 ms 以内 |
| C4 テンポ自動化 | 120→140→90→120 | 再生中の不連続 0、受信 MTC とブリッジ位置の差 p95 10 ms 以内、resync 0 |
| C5 Session clip / ビュー切替 | — | 不連続なし |
| C6 ブリッジ停止 | Control Surface を None → OmoiIroBridge | lost → holdover の後に消灯（show の loss）→ 戻すと再 lock、回復フェード |
| C7 Clock 停止 | `OMOI-IRO Clock` Sync OFF → ON | 時刻・LED は影響なし、拍源のタイルは holdover → lost、Pulse は透明 |

合否：B4 と同じ（再 lock 500 ms 以内、残差 p95 10 ms 以内、QF 送信遅れ p95 1 ms 以内）。加えて、アプリの診断（jitter / render / LED 送信）と安全タイルを各ケースで記録。

## 3. 2時間試験（NFR07）

- 構成：Live でループ再生（テンポ自動化を含む8〜16小節）、OmoiIroBridge、本番アプリ（release）、実機 LPX に LED 出力、操作入力も LPX、タイムコード出力あり。fake は使わない。
- 途中の操作：15分ごとにパッドを数回押す（Live の MIDI Map の対象を動かす）、30分ごとに Blackout のオン・オフ。
- 記録（5分ごと）：
  - working set：`omoiiro.exe` と、WebView2 を含む**子孫プロセスすべて**（`Get-CimInstance Win32_Process` で ParentProcessId を辿る）。Q01 では直接の子までしか数えていないので、ここで孫まで含める。
  - アプリの診断（frame、飛ばした frame、jitter、render、LED 送信、Live 転送、MTC 送信と失敗、破棄件数）。
  - `omoiiro-probe monitor` で受けた MTC とブリッジ位置の差（累積ドリフトの確認）。
- 判定：
  1. メモリー：開始30分後を基準に、2時間後までの増加が 10% 以内、かつ単調増加の傾向がない（基準値は要合意）。
  2. stuck note：終了時に Live 側で鳴りっぱなしの note がない。アプリの「保留中の Note Off」0。
  3. 累積ドリフト：受信 MTC とブリッジ位置の差の p95 が開始時と2時間後で変わらない（10 ms 以内のまま）、resync 0。
  4. 飛ばした frame の割合 0.1% 以下、send_errors 0、fault 0。
- Mac は放置不可のため、2時間連続は Windows のみ。Mac は短時間の立ち会い枠で同じ項目を分割して記録し、「連続安定性は未検証」とする（docs/01）。

## 4. Mac（立ち会い枠）

2026-10-28：本番（2026-10-29）はMacで行うと決まった（A19）。当日までの順序と本番の起動手順は [m6-mac-showday.md](m6-mac-showday.md) にまとめた。この節はMacでの差分の一覧として残す。

| 項目 | Mac での違い |
| --- | --- |
| Remote Script の配置 | `~/Music/Ableton/User Library/Remote Scripts/OmoiIroBridge/`（Live の Preferences > Library の場所を確認） |
| 仮想バス | IAC Driver を有効化し、バス名を `OMOI-IRO Control` / `OMOI-IRO Timecode` / `OMOI-IRO Clock` にする。CoreMIDI のポート名は「IAC Driver OMOI-IRO Control」のように機器名が前に付くことがあるので、`omoiiro-probe ports` で完全一致名を確認して端末設定に入れる |
| LPX のポート名 | CoreMIDI では「Launchpad X LPX MIDI」等（2系統）。MIDI 用を使う |
| ビルド | Xcode Command Line Tools（clang、libclang）、Rust（aarch64-apple-darwin）、Node 24。Link を使うなら cmake（Homebrew）と `cargo build --features link`（`LIBCLANG_PATH` は通常不要）。Tauri は `npx tauri build`（署名・公証なし、Gatekeeper の警告は手動で許可） |
| GUI の自動化 | `q01-gui.ps1` は Windows 専用。Mac は `omoiiro-probe q01`（headless）を実行し、GUI は手動で同じ表の項目を確認 |
| 枠の割り方 | M0 環境とビルド → M1 LPX（S01）→ M2 Live（C1〜C4）→ M3 併用と抜き差し（S05、C8）。各15〜20分、放置しない |

## 5. 実装側で M6 前に残っていること

1. 端末設定で機器（Launchpad）への LED 出力を許可する項目と、その確認画面の文言（M6 開始時、設計承認後）。
2. 機器の確認は実装済み。実機で Device Inquiry の family（03 01 / 03 11）と firmware 表記が想定どおりかを S01 で確かめ、違えば `omoiiro-devices::launchpad_x` を直す。
3. 2時間試験用に、子孫プロセス込みの working set を5分ごとに記録するスクリプト（`q01-gui.ps1 -Scenario soak` の拡張）。
