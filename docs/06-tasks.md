# OMOI-IRO 実装タスク分解

## 状態と粒度

2026-10-28時点：D01、S02（Live→アプリ方向）、S03、S04、S06（Windows）、S07（Windows）は完了。S01、S05は本番アプリ実装後（A16）。T01〜T04、T08の大半とI04のコーデック部分は `tools/p0-probe` の試作として実装済みで、[11-implementation-plan.md](11-implementation-plan.md) のM1でcrateへ移す。以下の表は元の計画のまま残す。1タスクは個別に成果物と検証結果をレビューできる粒度とする。P0の結果が関連タスクの設計を変える場合は、先に文書を更新する。

## 設計と技術検証

| ID | タスク | 依存 | 成果物と完了条件 |
| --- | --- | --- | --- |
| D01 | 運用条件を固定 | なし | 30 fps暫定・開始時刻未定、Windows 10/11候補、Apple Silicon Macを反映。正確なOS/CPU、Live/Max版、audio driver、試験時間枠を記録 |
| S01 | LPXポート・モード・RGB疎通 | D01 | 両OSでidentity、mode query、RGB7、入力、復帰の記録 |
| S02 | 仮想バスとLive入力を検証 | D01 | Control/Clock/Timecodeの接続表。Note/CCとNote Off、重複・loopなし |
| S03 | MTCブリッジの両ビュー試験 | S02 | Play/Stop/Continue/seek/loop/ビュー切替の送受信ログ |
| S04 | テンポ・位相と秒位置を照合 | S03 | 120→128→90、シーンのtempo変更で共通時計の意味を確認 |
| S05 | LPX送信負荷を測定 | S01,S02 | RGB 30/60 fps比較、入力併用、send時間、queue、切断復帰 |
| S06 | 対応環境・採用方式を確定 | S01〜S05,S07 | 両OSのmatrix、Link/Clock、bridge/backend、目標fpsの判断を文書化 |
| S07 | Linkの最小検証 | D01 | SDK/binding両OSビルド、tempo/phase、Start Stop Sync、途中参加・peer喪失・起動順・追加peer。依存の配布条件も確認 |

P0では完成GUIを作らず、診断目的の最小ツールと記録で検証する。LTC変換が必要になった場合はS03の派生タスクとして見積り直す。

## Coreと保存

| ID | タスク | 依存 | 成果物と完了条件 |
| --- | --- | --- | --- |
| C01 | Cargo workspaceと依存制約 | S06 | 5 crateとdesktopの空境界。CoreにTauri/midir依存なし |
| C02 | Show型とSchemaを確定 | C01 | DTO、schema、unknown fieldエラー、safe integer範囲が一致 |
| C03 | 意味検証 | C02 | 重複ID、参照切れ、fade超過、無効DF、overflowを拒否 |
| C04 | 時間・surface・Frame型 | C01 | 単位の混同を防ぎ、左下原点とLED IDをfixture化 |
| C05 | SolidとChase | C02,C04 | 始点・終点・周期境界と任意位置評価の期待値に一致 |
| C06 | Pulse | C04 | 与えたBeatStateの位相・depth・waveform通りに評価、beatなしは透明 |
| C07 | Timelineと合成 | C03,C05,C06 | 半開区間、重複clip、layer順、alpha、over/add/max、brightnessを確認 |
| C08 | JSON読込と保存 | C03 | 安定整形、元ファイル保全、無変更saveに差分なし |
| C09 | Presetライブラリー | C08 | 単体Schema、import/export、showへのコピー、共有編集時のfork |
| C10 | CLI相当のheadless検証経路 | C07,C08 | ハードウェア・GUIなしで任意時刻のframeを出力可能 |

## MIDIとDevice Adapter

| ID | タスク | 依存 | 成果物と完了条件 |
| --- | --- | --- | --- |
| I01 | midir backendとfake backend | C01,S06 | ポート列挙、役割binding、connection generation、失敗通知 |
| I02 | MIDI parserと入力正規化 | I01 | Note On 0、Note Off、CC、SysExとReal Time混在を処理 |
| I03 | 操作Routerとactive note追跡 | I02 | whitelist、同一経路の重複抑止、overflow時のfaultとnote解放 |
| I04 | LPX Adapter | C04,I02,S01 | mode/inquiry、64パッド座標、RGB7のgolden bytesが仕様一致 |
| I05 | LED writerと差分cache | I01,I04 | 最新frameのみ、最後の成功送信との差分、再接続でfull refresh |
| I06 | 接続管理と復帰 | I03,I05 | 切断、起動順変更、同名ポート、設定変更中のnote解放 |
| I07 | 固定Trigger/Action | I03,C07 | RouteMidiとBlackoutを別作用として処理、seekで外部操作を再送しない |

## 同期とRuntime

| ID | タスク | 依存 | 成果物と完了条件 |
| --- | --- | --- | --- |
| T01 | MTC decodeと時刻換算 | C04,I02 | QF/Full Frame、全fps、DF境界、順序不正、欠落、逆方向検出 |
| T02 | MIDI Clockとtransport | C04,I02 | F8/FA/FB/FC/F2、24 PPQN、SPP、途中参加、tempo変化 |
| T08 | Link Adapterと拍入力の選択 | C04,S07 | clock対応、tempo/phase snapshot、peer喪失、transport有効性。Liveへ変更を送らず、Clockへの切替は明示操作のみ |
| T03 | 単調時計への対応付け | I01,T01,T02 | 接続ごとのtimestampを正規化、再接続で古い基準を破棄 |
| T04 | 時刻補間と状態機械 | T03,T08,S03,S04 | offset、stop/seek/loop、Link途中参加、stopとMTCの到着順差、holdover/lost/relockが規約どおり |
| T05 | Schedulerとmailbox | C07,I05,T04 | 30 fps、frame drop時は現在時刻へ、入力経路をblockしない |
| T06 | Show差替えとoverride | T05,C08,I07 | 検証成功したrevisionだけ反映、Blackout・brightnessが最終合成に作用 |
| T07 | 診断と計測 | T05 | jitter、render/send時間、callback→send遅延、drop件数を取得 |

## GUIと利用フロー

| ID | タスク | 依存 | 成果物と完了条件 |
| --- | --- | --- | --- |
| U01 | Tauri shellとIPC | C01,T06 | Rust Runtimeの起動終了、typed DTO、command検証、最小capability |
| U02 | 接続・同期設定画面 | U01,I06,T04 | port roleを視覚化し疎通可能。誤設定の理由を表示 |
| U03 | LEDプレビューとlocal clock | U01,C10 | Rust Frameのみ表示、ローカルscrubで外部Liveを動かさない |
| U04 | Timeline編集 | U01,C07,C08 | clip追加・移動・長さ・削除・複製、数値入力、Undo/Redo |
| U05 | LayerとPreset編集 | U04,C09 | 順序・blend・opacity、params編集、preset共有の意図を保つ |
| U06 | 演奏画面と診断 | U02,U03,T07 | sync health、現在時刻、Blackout、接続復旧、GUI負荷時も演出継続 |
| U07 | 保存とエラー復旧UI | U04,U05 | unsaved表示、読込失敗で現show維持、保存失敗時の説明 |

## 統合とリリース判定

| ID | タスク | 依存 | 成果物と完了条件 |
| --- | --- | --- | --- |
| Q01 | ハードウェアなしの通し試験 | U03〜U07 | サンプル作成→保存→再読込→任意位置プレビュー |
| Q02 | Windows実機試験 | 全実装タスク | S01〜S07とPRD性能項目を再実行、構成版を記録 |
| Q03 | macOS実機試験 | 全実装タスク | Apple SiliconでQ02と同じ操作列を短い立ち会い枠に分割。macOS版と各枠のログを記録。放置運転しない |
| Q04 | 長時間・故障注入試験 | Q02,Q03 | 故障注入は短い枠でUSB切断・同期断・queue飽和・GUI遅延・保存中断を確認。2時間連続試験は別記録とし、Macは立ち会い枠確保まで未検証 |
| Q05 | ビルドと配布手順 | Q02,Q03 | Windows/macOSの成果物、署名等の方針、外部バス導入案内 |
| Q06 | 運用手順とMVP判定 | Q01〜Q05 | 起動順、Live設定、復旧、既知制限とFR01〜FR11の結果 |

## 重点テストケース

- DFの1分・10分・1時間境界、24時間wrap、fpsの切替、Full Frame直後のQF。
- QF欠落・重複・順序反転、SysExの途中切れ、Real Time byteの挿入。
- Play前のSPP、Continue前のSPP、途中参加、長時間運転中のSPP値域限界。
- Link参加直後のtransport初期値、peerなしで進むローカル時計、Liveより先の起動、追加peerのtempo変更、MTC/Linkの通知順序。
- clip開始直前・開始点・終了直前・終了点、前後seek、loop後に前フレームが残らないこと。
- 多数のNote On/OffとLED全更新を同時実行し、Note Offの欠落と入力遅延を計測。
- フレームを間引いた後も、差分送信の基準が最後の成功送信であること。
- Blackout中もLive操作が届き、解除時に古いフレームでなく現在の演出を送ること。
- 読込エラー・未対応schema・preset参照切れで、演奏中のshowを置き換えないこと。

## 要件との対応

| PRD要件 | 主なタスク |
| --- | --- |
| FR01 | S01,S05,I04,I05,Q02,Q03 |
| FR02 | S02,I02,I03,I06 |
| FR03 | S03,S04,T01,T03,T04 |
| FR04 | C05,C07,U04 |
| FR05 | S07,C06,T02,T08,T04 |
| FR06 | C07,U05 |
| FR07 | C10,U03,Q01 |
| FR08 | C09,U05 |
| FR09 | C02,C03,C08,U07 |
| FR10 | I06,T07,U06,Q04 |
| FR11 | S03,S04,Q02,Q03 |

まずD01とS01〜S04、S07を小さな検証単位として実施する。これらが成立する前に、完成版GUIや全拡張の実装へ進まない。
