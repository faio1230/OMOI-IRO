# OMOI-IRO 実装計画（P1以降、2026-10-28版）

利用者の決定（[07-decisions.md](07-decisions.md) A13〜A17）を受けて、`tools/` の試作から本番アプリへ移るための計画。設計担当（Fableセッション）が保守し、実装担当（Opusセッション）が実装する。[05-roadmap.md](05-roadmap.md) と [06-tasks.md](06-tasks.md) のタスクIDをそのまま使い、P0の結果で変わった点をここに集約する。

## 設計からの変更点（P0の結果）

| 項目 | 設計時 | 現在 |
| --- | --- | --- |
| 共通時計の源 | MTC受信（LiveMTC） | Live内Remote Script `DJSCBridge` の曲中時刻サンプル（[10-live-bridge.md](10-live-bridge.md)） |
| MTC | 入力 | アプリが生成する出力（外部機器向け）。MTC decoderは外部MTC源と自己検証用に残す |
| FR03「MTC受信」 | Quarter Frame / Full Frame受信 | 「ブリッジ受信 + MTC出力」と読み替える。受信側の健全性規則はブリッジに適用 |
| 再生状態 | Link Start Stop Sync | ブリッジの `is_playing`（代替はClockのStart/Stop/Continue）。アプリはSSSを有効にしない |
| タイムコード | 30 fps暫定、起点未定 | 30 fps NDF確定。起点はshowの `sync.origin`（既定01:00:00:00）で自由 |
| テンポ | 変更時のMTCジャンプを検証 | 自動化で運用する前提。手動変更はLive自体の時刻が飛ぶ |
| 拍入力 | Link第一候補 | 同じ。Link ONはLive側が先。アプリは参加時のセッションtempoを既定値にしない |
| 検証済みの試作 | なし | `tools/p0-probe`（decoder群、mtc-out、Link観測、lpx-check、route-check、記録とreplay）、`tools/live-bridge`、実測記録 `artifacts/p0` |

## 構成

[03-modules.md](03-modules.md) の5 crate + desktop shellを維持し、次を加える。

```text
OMOI-IRO/
  Cargo.toml                 # workspace
  crates/
    show-core/               # 変更なし
    show-devices/            # launchpad_x は tools/p0-probe/src/lpx.rs を移植
    show-midi/               # ports / backend / routing は p0-probe の ports.rs, monitor.rs(ingress), route.rs を移植
    show-store/
    show-runtime/
      src/sync/
        bridge.rs            # p0-probe bridge_clock.rs（共通時計の第一の源）
        mtc_out.rs           # p0-probe mtc_gen.rs（MTC生成の状態機械）
        mtc.rs               # p0-probe mtc.rs（外部MTC受信・自己検証）
        clock.rs             # p0-probe midiclock.rs
        transport.rs         # ブリッジ / Clock / Link の統合とTransportSnapshot
      src/link/              # p0-probe link.rs（optional feature、SSSなし、peer 0でtempo非表示）
      src/diagnostics/       # p0-probe record.rs / report / replay
  apps/
    desktop/                 # Tauri 2 + React
    probe/                   # 旧 tools/p0-probe。crateの上に載る薄いCLI。replayテストの回帰資産を維持
  live-bridge/
    DJSCBridge/              # tools/live-bridge から昇格（Python、Live側）
  tools/                     # 実験用（live-bridge-probe、link-sss-probe）。本番コードから参照しない
  fixtures/                  # artifacts/p0 から抜いた記録（replay用）
```

依存方向は変えない：`show-core` は midir・Tauri・OS時計に依存しない。`show-runtime::sync` のdecoder群は時刻を引数に取る純粋な状態機械のまま移す（p0-probeの設計を維持）。

## マイルストーン

各マイルストーンは「完了報告 → 設計担当がテストを再実行して受け入れ」で閉じる。実機を使う確認は A16 により M6 にまとめる。

| M | 内容 | 対応タスク | 完了条件 |
| --- | --- | --- | --- |
| M1 Workspaceと移植 | workspace作成、5 crate + `apps/probe` の骨格、p0-probeのdecoder群・記録・replay・lpx・route・Link観測をcrateへ移植。`apps/probe` が従来と同じコマンドで動く | C01、C04、T01、T02、T03（骨格）、I02、I04（codec部分） | p0-probeの既存テスト（96件）が新配置で全部pass。`cargo build --features link` が通る。CLIの出力形式が変わらない |
| M2 Coreとデータ | show型、Schema検証、意味検証、Solid / Chase / Pulse、合成、headless render | C02、C03、C05〜C10 | 任意時刻評価・合成・保存再読込のテスト。`examples/demo.show.json` をheadlessで任意時刻renderできる |
| M3 Sync統合 | ブリッジ受信・Clock・Linkを `TransportSnapshot` に統合、show time換算（origin、input offset）、holdover / lost / blackout、MTC出力をRuntime配下へ | T04、T08、T07（同期部分） | `artifacts/p0` の記録（c1〜c7、bridge-01..03、sss）をfixtureにした回帰テスト。p0-probeで得た数値（再lock、残差）が変わらない |
| M4 MIDIとRuntime | midir backend + fake backend、操作Router、LPX Adapter、LED writer、scheduler、mailbox、override、診断。headlessの「追従モード」（ブリッジ → Core → 仮想LPX / 実機LPX） | I01、I03、I05〜I07、T05、T06 | fake backendとfake clockで切断・負荷・queue overflowの再現。30 fps renderのp95計測（NFR03） |
| M5 GUI | 冒頭でOMOI-IROへの一括改名（A18）：crate `omoiiro-core/-devices/-midi/-store/-runtime`、CLI `omoiiro-probe`、Remote Script `OmoiIroBridge`、仮想ポート `OMOI-IRO Control/Timecode/Clock`、記録とJSONLの `DJSC` 接頭辞、Tauriの製品名・ウィンドウ題名・bundle identifier・配布物名。改名後にテストと記録のreplayが同じ結果になることを確認してからGUIへ。続けてTauri shell、IPC、接続・同期画面、プレビュー、timeline編集、preset、演奏画面、保存 | U01〜U07 | 機器なしでショー作成 → 保存 → 再読込 → 任意位置プレビュー（Q01） |
| M6 実機と長時間 | Windows実機（S01、S02、S05、C8を本番アプリで）、2時間試験、Mac枠（M0〜M4） | Q02〜Q04、A16 | runbookの手順で記録を残し、p0-results.md に追記 |

M1とM2は独立なので並行できる（実装担当が1セッションなら M1 → M2 の順）。M3はM1とM2の後。M4はM3の後。M5はM4の後。

## 実装の規則（P0から継続）

- callbackでは時刻採取・コピー・try_send だけ。デコード・描画・ファイル書込は別スレッド。
- 接続ごとのbackend timestampを比較しない。共通の単調時計を時刻軸にする。
- Liveへtempo・transport・Linkの状態変更を送らない。LinkのSSSは有効にしない。
- 合成fixture・replayの成功を実機合格として報告しない。
- 完成版GUIを先に作らない（M5まで）。F1、OSC、ノードグラフ、LTC変換は範囲外。
- 既存のp0-probeの挙動（数値、イベント名、JSONL形式）は移植で変えない。変える場合は設計担当へ先に送る。

## 分担

| 領域 | 担当 | 備考 |
| --- | --- | --- |
| `crates/`、`apps/`、`live-bridge/`、`tools/`、`fixtures/` | 実装担当 | 新規作成と移植 |
| `docs/`、`prompts/`、`artifacts/p0` の既存記録 | 設計担当 | 結果と判断の記録 |
| `schemas/`、`examples/` | 共同 | 変更案は実装担当が送り、設計担当が承認してから反映 |

連絡：着手時に担当範囲、完了時に変更点・テスト結果・確認事項。設計と食い違う点は、その作業だけ止めて他を進める。

## 最初の作業（M1）

1. `Cargo.toml`（workspace）と5 crateの空の骨格。`apps/probe` を作り、`tools/p0-probe` のコードを移す。`tools/p0-probe` は削除せず、READMEで「`apps/probe` へ移動」と案内して中身を空にする（記録のパスを参照している文書があるため、`artifacts/p0` は動かさない）。
2. 移植先の対応表（上記構成）に従ってモジュールを配置し、p0-probeのテスト96件とlive-bridgeのテスト5件を新配置で通す。
3. `fixtures/` を作り、`apps/probe/fixtures` の実測記録を共有する。
4. `cargo build --features link` の手順（VS 2026開発者シェル、NMake、LIBCLANG_PATH）をworkspaceのREADMEに移す。
5. 完了報告：ディレクトリー構成、`cargo test` の結果、変更した公開API。

## 進捗

| M | 状態 | 備考 |
| --- | --- | --- |
| M1 | 完了（2026-10-28） | `cargo test --workspace` 97件pass（旧96件 + CLI引数テストの分離1件）、Link付き99件。CLI出力は旧バイナリとバイト一致（15通り）。設計との差異を承認：monitor固有のingressは `apps/probe` に残す（M4のMidiBackend契約で統一）、LPXのポートを開くセッションは `apps/probe`、MonotonicClockは `show-midi` に置き `show-runtime` から再公開、fixturesはルート `fixtures/`、`sync/transport.rs` はM3用の空ファイル。ビルド手順は `BUILD.md` |
| M2 | 完了（2026-10-28） | 133件pass（show-core 24、show-store 9、probe 26、runtime 61、midi 8、devices 5）。show-coreの依存はserdeのみ。Schema正本は手書きのまま（jsonschema =0.60.2、ネットワーク参照なし）、DTOはdeny_unknown_fields、往復テストでずれを防ぐ。`p0-probe render` で任意時刻のFrameを出力。評価規約は [04-show-data.md](04-show-data.md) に記録。Schema改訂（preset.schema.json、source: live_bridge、if/then）はM3で案を受ける |
| M3 | 完了（2026-10-28） | 149件pass（probe 33、runtime 70、core 24、store 9、midi 8、devices 5）。`show-runtime::sync::transport`（TransportSnapshot：時刻側と拍側の健全性を独立、output follow/hold/black、recovery fade）、`output::mtc`（RuntimeからMTC生成）、`diagnostics::transport_replay`、`p0-probe transport`。回帰：c4は28 s飛びなし、c6bは1.0 s後にblack→再起動でfollow、c2bのmtc-out数値固定。Schema 0.1.1案を承認（下記）。設計との差異：FrameRate型の二重化はM4で統一、回復フェードは再点灯時点から、mtc-outのCLI既定は250 ms/1 s |
| Schema 0.1.1 | 完了（2026-10-28、150件pass） | `schema_version` 0.1.1（0.1.0はmigrationで読む）、`sync.timecode.source` enum `live_bridge`（既定）/`mtc`、`sync.mtc_output.enabled`、content/effectのoneOfをif/thenへ、`schemas/preset.schema.json`（`kind` は `omoiiro.preset`）、beat_sourceの説明にSSS不使用を追記。schemas/とexamples/の変更は実装担当が行い、設計担当が確認する |
| M4 | 完了（2026-10-28） | 173件pass（probe 33、runtime 83、core 24、midi 15、store 10、devices 8）。MidiBackend契約（midir + fake）、Ingress（1024 B上限、世代）、Router（whitelist、active note追跡、fault時のNote Off）、LaunchpadXAdapter + VirtualLpx（golden bytes）、LedWriter（差分基準は最後の送信成功、失敗・再接続でfull refresh）、接続管理（Disconnected / Ready / Recovering / Fault、0.5 s再試行、同名ポートはFault）、Trigger/Action（BlackoutはLEDのみ）、FrameScheduler + Mailbox（容量1）、ShowExchange（検証済みrevisionのみadopt）、perf Histogram、`p0-probe follow` / `bench-render`。計測（合成）：NFR03 render p95 2 µs（release）、followのjitter p95 0.54 ms。設計との差異を承認：接続のIdentifying/ConfiguringはM6、同名ポートはエラー（#indexで指定）、overflow/送信失敗のfaultは明示reset、出力消失のfaultは再接続で自動解除、render fpsはshowから、仮想LPXはプロセス内のみ |
| M5 改名 | 完了（2026-10-28） | `crates/omoiiro-*`、`omoiiro-probe`、`live-bridge/OmoiIroBridge`、ポート名 `OMOI-IRO Control/Timecode/Clock`、midirクライアント名。改名前後で70件の出力がバイト一致、173件pass。fixtures・tools・docs・artifactsは旧名のまま。利用者環境（loopMIDIポート、LiveのRemote Script）の移行手順は runbook 冒頭 |
| M5 GUI | 進行中 | U01完了（2026-10-28、176件pass）：`omoiiro_runtime::session::FollowSession`（Runtimeの起動・停止境界、CLIのfollowもこの上に）、`apps/desktop`（パッケージ omoiiro-desktop、実行ファイル omoiiro.exe、identifier jp.faio.omoiiro、productName OMOI-IRO）、command 8個（入力はRust側で検証、エラーは {code, message}）、capabilityは8 commandのみ（core:default・plugin不使用、CSPはself+ipc）、型付きDTO（RustとTSの一致をテスト）、画面は起動設定・状態・上書き・イベントログ（200 ms poll、演出時計はGUIで進めない）。版固定：tauri 2.12.5、@tauri-apps/api・cli 2.12.4、React 19.3.1、Vite 8.3.5、TypeScript 5.9.3。判断を承認：状態は200 ms poll（LEDプレビューはU03でChannel）、ファイル選択ダイアログはU07、単体実行は `npx tauri build --debug --no-bundle`、desktop crateをworkspaceに含めbuild.rsが仮のdistを置く。U03完了（2026-10-28、179件pass）：LEDプレビューはRustのFrameだけを表示（Tauri Channel、容量1、ackが来るまで次を送らない、最大30 fps、間引き件数を表示）。表示元はRuntime出力（gain・Blackout・明るさ適用済み、RGB7量子化を戻した値）とローカルプレビュー（`omoiiro_runtime::preview` のLocalClock / LocalPreview。Runtimeへの参照を持たず、Live・Link・機器へ送らない）。追従中のscrubは独立プレビューに切り替わる。preview.bpmはRuntime停止中だけ変更可。command 7個追加（capability合計15、plugin不使用）。判断を承認：scrubはスライダーを離した時点で確定、独立プレビューの拍はshowのpreview.bpm、ローカルプレビューにタイムコード表示なし。U02完了（2026-10-28、184件pass）：端末設定 `omoiiro_runtime::settings::TerminalSettings`（version 1、`%APPDATA%\jp.faio.omoiiro\terminal-settings.json`、一時ファイル経由の保存、壊れたファイルは既定値で開いて理由表示、未知キー拒否）、役割 control_in / control_out / clock_in / led_out / timecode_out、`check_roles`（列挙と照合のみ。完全一致か #番号、同名はFault、逆流・二重割当・Launchpadらしい名前を理由つきで拒否、今ないポートは警告）、受信確認 `settings_listen`（3秒、種類別に数える、何も送らない）、command 4個追加（capability合計19）、`runtime_start` は show_path だけを受け端末設定を検証、MTC出力をFollowSessionに接続（showの `mtc_output.enabled` と timecode_out の両方があるときだけ送信）。判断を承認：候補はdatalist、受信確認は3秒固定、MTC出力の再接続はU06、遅延補正・色校正はM6。U06完了（2026-10-28、186件pass）：演奏タブと接続設定タブ、大きなタイムコード、BLACKOUT、同期と接続の状態を5タイルで別表示（時刻源・拍源・LED・操作経路・タイムコード出力、接続世代・エラー・暗転理由・faultの原因とNote Off保留数、reset）、「全ポートを再接続」（command 20個目）、`ReconnectingOutput` でMTC出力の自動再接続（epoch更新でFull Frameから送り直し）、タイムコード出力ポート不在でも開始可、診断の常時公開（jitter / render / LED送信 / render→送信 / callback→Live、p50/p95/p99/max、各件数）。GUI負荷試験（状態取得4スレッド+CPU 2スレッド中に100件のNote転送、45 frame/1.5 s、callback→送信 p99 44 µs）。判断を承認：診断は累計、拍源unknownは橙、タイムコード出力の状態は送信時更新。U04完了（2026-10-28、192件pass、command 25個）：`omoiiro_core::edit`（EditCommand 9種、適用はコピー→validate→不正ならRejected、ID採番、スナップはなし/1 s/30 fpsフレーム、Historyはshow全体のArcスナップショット最大200件、dirty / mark_saved）、desktopのeditor（履歴はRust側、採用した版はShowExchangeで次のframe境界に反映、ローカルプレビューは位置を保って差し替え）、タイムラインタブ（レイヤー行、ドラッグ移動と長さ変更、クリップ欄の数値入力、Undo/Redo、キー操作）。拡張の余地（[12-original-requirements.md](12-original-requirements.md)）：`Param<T>`（今はValueのみ、保存形式は不変、0.2でKeyframesを追加）、Effectの `type` にgraph variantを足せる構造、履歴はスナップショット形式なので操作追加で形式が変わらない。判断を承認：レイヤー間移動はクリップ欄で、数値入力もスナップ、continuousレイヤー追加とプリセット差し替えはU05、保存はU07。U05完了（2026-10-28、196件pass、command 27個）：EditCommandに9種追加（set_layer、add_continuous_layer、set_layer_preset、add_preset、set_preset（共有編集）、delete_preset（使用中は理由つき拒否）、insert_preset（import、衝突はError/Rename）、set_clip_preset、edit_clip_effect（共有中は `<id>-clip-N` へ複製してそのclipだけ差し替え））、preset_import / preset_export（1 MB上限、Schema 0.1.1検証、一時ファイル経由）、レイヤー設定とプリセット欄（範囲と単位をSchemaに合わせて表示、判定はRust側）。判断を承認：ダイアログはU07、不透明度は離した時点で1 command、新規の既定値、typeの変更は新規作成のみ。安全上限完了（2026-10-28、202件pass）：端末設定 `safety`（max_brightness 0.05〜1、max_flash_hz 0.5〜30 既定3.0、limit_flash 既定false）、`omoiiro_core::safety`（cap_frameは各チャンネルの天井、Pulse = tempo/60/beats_per_cycle、Chase = (8/width)/period）、送信直前の順序は render → Overrides → cap → 量子化、警告と消灯はrevision・tempo・上限の変化時だけ再計算、`limit_flash` がtrueのときだけ該当レイヤーを消灯（showは変えない）、演奏タブの「安全」タイルとタイムラインの⚠表示。判断を承認：明るさスライダーはgainのまま上限は独立の天井（横に「上限 N%」表示）、拍源無効時もpreview.bpmで警告、判定は単一presetの周波数（レイヤー間の干渉は扱わない）。U07完了（2026-10-28、205件pass、command 30個、capability 32個 = command 30 + dialog:allow-open / allow-save）：`save_atomic`（一時ファイル→flush→rename、失敗時は元ファイル維持と理由表示、未保存のまま）、名前を付けて保存、パス検証（拡張子 .show.json / .preset.json、サイズ上限、書込先フォルダー）、読込失敗は現在のshowもRuntimeも変えずにコードとJSON Pointerの一覧を表示、未保存表示と終了確認（保存して終了 / 保存せずに終了 / キャンセル。保存失敗なら閉じない）、ファイル選択（dialog pluginはパスを返すだけで読み書きはRust側）、最近使ったショー（端末設定、最大10件）、無変更saveのバイト一致。判断を承認：終了確認はeventでなく状態取得で伝える、拡張子の自動補完、未保存中は別showを開かせない。環境：target\debug が約28 GBに達しC:の空きが100 MBまで減ったため incremental を削除し `CARGO_INCREMENTAL=0` でビルド（BUILD.mdに記載）。Q01途中（2026-10-28、実装担当の利用上限で中断）：新規ショー（`editor_new`、command 31個、capability 33個）、fake_bridge の `--steady`、プレビューの位置指定、headless runner `omoiiro-probe q01` は23項目PASS（合成。プレビューとrenderの差は最大1段、追従中の編集は1 frame後に採用、jitter p95 552 µs）。GUIスクリプト `apps/desktop/scripts/q01-gui.ps1` はG1.1までPASS。Q01完了（2026-10-28、206件pass、Link付き208件）：headless 23項目PASS、GUI 6項目PASS、G1.2（保存ダイアログの無人操作）のみ手動確認項目。10分soak：18,034 frame（30 fpsどおり）、jitter p95 0.52 ms、working set 45.7→46.2 MB（孫プロセス除く）。試験中に `editor_view` がRuntimeのrevisionを返さない不具合を発見・修正。結果は [validation/q01-results.md](validation/q01-results.md)。**M5完了** |
| M6 準備 | 完了（2026-10-28、211件pass） | リリースビルド（`npx tauri build`、NSIS、署名なし。`omoiiro.exe` 18.0 MB、インストーラー 3.7 MB、ビルド106 s。release の bench-render p95 2 µs、headless Q01 23項目PASS）。手順書 [validation/m6-runbook.md](validation/m6-runbook.md)（LPX S01/S02/S05/C8、Live C1〜C7を本番アプリで、2時間試験の記録と判定、Mac枠）。接続時の機器確認 `output::identify::DeviceIdentify`（Identifying → Configuring → Ready、別機種・ブートローダー・応答なしは理由つきfault、Readyまでframeを送らない、fakeで検証）。設計判断：Launchpad出力の許可は端末設定 `devices.allow_launchpad_led`（既定false、確認文言つき）、メモリー判定は30分基準で10%以内かつ+50 MB以内、family/firmwareはS01で確認 |
| M6 実機 | 未着手（準備完了、212件pass） | 端末設定 `devices.allow_launchpad_led`（既定false、警告文、control_out / timecode_out へのLaunchpadは引き続き拒否、許可ありの開始はDevice Inquiryから）、soakスクリプトの拡張（子孫プロセス込みのworking setを5分ごと、判定は30分基準で10%かつ+50 MB以内、1%以下の揺れは除外、`-Show` / `-NoFakeBridge`）、BUILD.mdに出力先の移動手順（D:は空き3.2 GBで不向き）。注意：試走で `q01-gui-soak.png` が上書きされた（Q01の数値は報告書に残る）。利用者の環境準備（OmoiIroBridge入れ替え、MIDI設定、LPX接続）とMac枠の日程が前提。2026-10-28夕方：Windows側のLive準備（OmoiIroBridge、ポート対応 Port 2/3/4 = Control/Timecode/Clock、Clock Sync ON、Control Track/Remote ON）完了（`artifacts/m6/win-m6-prep-02.jsonl`、801サンプル、faults 0）。途中でloopMIDIの折り返しが止まり、midisrv再起動で復旧（environment.md）。同日、本番が翌2026-10-29でMacが本番機と判明（A19）。Windows 2時間連続試験（NFR07）合格（メモリー+2.2%、飛ばしたframe 0、送信失敗 0、残差p95 1.23→1.27 ms）。Windows本番アプリ（release）でC1〜C7合格（再lock 129〜159 ms、テンポ自動化で不連続 0、残差 p95 1.09 ms。[validation/m6-results.md](validation/m6-results.md)）。Mac対応の事前作業完了（同日、214件pass）：bundle.targets に app / dmg を追加、CoreMIDI名の扱い（LPXの In / Out 違いを同一機器と判定、DAWポートを理由つきで拒否、日本語IAC名のテスト）、`scripts/mac-setup.sh`、`fixtures/terminal-settings.mac.example.json`。Mac実機で `.app` ビルド、S01、Live追従（ブリッジ+Clock）、LED追従を確認（[validation/m6-results.md](validation/m6-results.md)）。実機で見つかった2件を同日修正：新規showの既定をmidi_clockに変更しGUIに同期設定欄と未保存時の3択を追加（215件）、Clock原点未確定時にブリッジの拍で原点を補完（218件）。利用者の依頼で演出を追加（同日夕方、Schema 0.1.2）：点滅 `builtin.strobe`、ノイズ `builtin.noise`、Chaseの `reverse`（222件）、波紋 `builtin.ripple`（入力駆動。PadInputをrenderに渡す経路、プレビュー盤クリック、225件）。雨 `builtin.rain`・波形 `builtin.wave`・球 `builtin.orb`（229件）。プリセットの並行制作は [13-preset-authoring.md](13-preset-authoring.md)。以後の順序は [validation/m6-mac-showday.md](validation/m6-mac-showday.md) に従う：Macのビルドと接続 → Mac + LPX → Windows C1〜C7（並行） → 連続試験はWindowsで放置 |
| M6 | 未着手 | — |

## 見積りの見直し

[05-roadmap.md](05-roadmap.md) の初期見積り27〜45人日のうち、P0とP1試作で同期部分（T01〜T04、T08の大半）と機器コーデック（I04の一部）は実装済み。残りはM1の移植、M2のCore、M4のRuntime、M5のGUIが主で、実装担当が連続して作業できれば暦日3〜5週が目安。Mac枠と2時間試験は別途。

## リスク

- rusty_link はGPL-2.0-or-later。配布形態（個人利用から開始、O09）と整合させる。Link SDK自体の別ライセンス条件も確認する。
- Remote Script APIのLive版依存。Live更新時に `live-bridge/README.md` のAPI確認表を更新する。
- Windowsのcmake 3.26がVS 2026を知らない問題。CIを組む場合はcmake 4.xかNinjaを使う。
- Macは未検証。M6まで実機に触れないので、M1〜M5でmacOS固有コード（CoreMIDI名、配置先）は条件付きコンパイルで隔離する。
