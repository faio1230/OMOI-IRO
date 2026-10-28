# P0 検証結果

更新日：2026-10-28。環境は [environment.md](environment.md)、手順は [p0-runbook.md](p0-runbook.md)。生ログは `artifacts/p0/`。

「実測」はOSのMIDI経路を通した観測、「合成」はツール内部または合成送信器によるデコーダー試験、「未実施」は環境がなく実施していないもの。合成と配管確認の合格は、Liveや機器との接続成功を意味しない。

証拠列のパス `tools/p0-probe/...` は、2026-10-28のM1で `apps/probe/`（テスト）と `fixtures/`（fixture）へ移動した。`artifacts/p0/` は動かしていない。移植後のworkspaceでも同じ記録のreplayが同じ数値を出すことを確認済み（97 tests、CLI出力のバイト一致）。

## 進捗の区分

| 区分 | 状態 |
| --- | --- |
| ツール実装 | `ports` / `monitor`（MTC + MIDI Clock）/ `replay` / `synth` / `gen-mtc` に加え、2026-10-28に実装担当（Opusセッション）が `lpx-check`、`route-check`、Full Frame由来の不連続検出（`ff_disc`、閾値2フレーム）、Clock途絶判定（`stalled`、500 ms）を追加。52 tests pass（unit 48、合成fixture 1、実測記録replay 3）。Link観測は未実装（libclang導入中） |
| Windows機能検証 | 配管確認（合成源）に加え、2026-10-28にLive 12 Suite 12.4.7 + LiveMTC 1.1.0で runbook B2 の C1〜C7 を実施（S03、S04、S02のClock経路）。S07（Link）は観測とLive連携を実施（Start Stop Syncの問題あり）。O12の代替ブリッジ計測を実施。C8とLPX関連（S01、S05）、S02のアプリ→Live方向は未実施 |
| P1試作（自前ブリッジ + MTC出力） | 実装済み（`tools/live-bridge/DJSCBridge`、`p0-probe bridge-clock` / `mtc-out` / `mtc-out-report`。88 + 5 tests pass）。Live 12.4.7実機でrunbook B4のC1〜C7を実施し、すべて合格（下記「B4の結果」）。2時間試験は未実施 |
| Mac機能検証 | 未実施 |
| 連続安定性検証 | 未実施 |

Windows側のLive連携はClaude Desktopのcomputer use（画面操作）で行った。Live側の操作（Play/Stop、位置入力、テンポ入力、デバイスON/OFF、Preferences変更）はキー入力とクリックで行い、観測は `p0-probe monitor` が独立に記録した。

## ビルドとテスト（Windows）

| 項目 | 結果 | 証拠 |
| --- | --- | --- |
| `cargo build`（tools/p0-probe） | pass、警告なし。Rust 1.94.1 stable-msvc、midir 0.11.1 | 2026-10-28 のビルド出力 |
| `cargo test` | pass、26 tests（unit 25 + integration 1） | 同上 |

単体テストの範囲（合成）：フレーム→μs換算の精度、QFラベルの往復、1系列でacquiring、2系列でlockedと補間、Full Frameは位置更新のみ、Real Time混入SysExの復号、Full Frameによる部分系列の破棄、seekの不連続と再lock、piece欠落の破棄、piece timeout、holdover→lost→blackout、逆方向系列の検出、29.97 DFの未対応扱い、fps変更、不正ラベル。MIDI Clockは Start/tick/Stop/Continue/SPP と途中参加。

## 結果表

| ケースID | 環境 | 実行日時 | 期待結果 | 観測結果 | 判定 | 証拠 | 次の判断 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| A2 合成fixture replay | Windows、ツール内部 | 2026-10-28 | FF→locked→FF→locked→gap不連続(+12)→relock→lost | 期待どおり。records 750、mtc 602、clock 146、drop 0、locks 3、disc 1 | pass（合成） | `tools/p0-probe/fixtures/synthetic/mtc30-play-seek-gap.jsonl`、`tests/replay_synthetic.rs` | なし |
| A3-1 ループバック | Windows 11 25H2、loopMIDI Port、WinMM。MTCとClockを別接続で同一ポートから受信 | 2026-10-28 00:0x | 合成30 NDFを8 s、120 bpm、中間400 ms欠落 | FF受信→0.175 s後にlocked。912 QF、114系列、drop 0、q_drop 0。欠落で `discontinuity +12 frames` と67 ms後に再lock。送信終了後 `lost`。Clock 119.94〜120.07 bpm、Start/Stop検出。同一ポートを2回開いたため各メッセージが2回記録された | pass（配管確認、合成源） | `artifacts/p0/win-loopback-01.jsonl`、`win-loopback-01.console.txt` | 共有ポート時は1回だけ開く実装へ変更（A3-2で確認） |
| A3-2 ループバック（共有ポート） | 同上。`--mtc-in` と `--clock-in` に同じポート、`sync` として1回記録 | 2026-10-28 00:1x | 00:59:58:00から6 s、128 bpm、中間300 ms欠落、時境界を跨ぐ | 684 QF、85系列、drop 0、q_drop 0、stray 4（欠落境界の残りpiece）。`discontinuity +10 frames`、66 ms後に再lock。00:59:59:28→01:00:00:00を連続として処理。Clock 127.85〜128.04 bpm、307 ticks（理論 307.2）。records 1002 | pass（配管確認、合成源） | `artifacts/p0/win-loopback-02.jsonl`、`win-loopback-02.console.txt` | BPM揺れは `gen-mtc` の `thread::sleep` 由来。Liveの実出力で再評価 |
| A3-3 ループバック（DJSCポート） | Windows。loopMIDIにレジストリー経由で `DJSC Control` / `DJSC Timecode` / `DJSC Clock` を追加して再起動。Live停止中 | 2026-10-28 02:5x | 合成30 NDF 6 s、中間400 ms欠落、Clock 120 bpm 1 s | FF→locked、`discontinuity +12 frames`、67 ms後に再lock、送信終了後 `lost`。Clock start/stop検出、48 ticks、119.9 bpm。drop 0、q_drop 0 | pass（配管確認、合成源） | `artifacts/p0/win-loopback-03.*` | Live起動中にloopMIDIを再起動した最初の試行では、既存の `loopMIDI Port` を含め何も届かなかった（loopMIDIのTotal dataも0）。順序はloopMIDI→Live |
| PM-1 Live Clock経路の対応付け | Windows、Live 12.4.7。Live出力 `<PC名> (Port 6)` のSyncをON。音声はVoicemeeter Virtual ASIO（Voicemeeter本体は未起動） | 2026-10-28 03:02 | Live再生でDJSC ClockにF8が届く | Start/Stop検出、163 ticks / 6.06 s = 約67 bpm。Liveの位置表示も6 sで2.3.4（約6.8拍）までしか進まず、Liveのトランスポート自体が遅かった | pass（対応付け）、fail（テンポ） | `artifacts/p0/win-portmap-01.*` | 音声ドライバーをGeneric Low Latency ASIO Driverへ変更して再試験 |
| PM-2 同上（Generic Low Latency ASIO Driver、441 samples） | 同上 | 2026-10-28 03:04 | 120 bpm相当 | 291 ticks / 6.06 s（理論290.8）、BPM 119.7〜122.4（24 tick窓）、beats 12.12、Live表示4.1.1 | pass（実測、Live） | `artifacts/p0/win-portmap-02.*` | Live `(Port 6)` = WinMM `DJSC Clock` を確定。Liveのポート名はWinMM名と一致しない（下記） |
| S03 smoke LiveMTC初回 | Windows、LiveMTC 1.1.0（Bridge + MTC.amxd on Main）、起点00:00:00:00 | 2026-10-28 03:07 | Play 8 s → Stop | SPP 0 + Start → 44 ms後にFF 00:00:00:00 → 0.66 s後に locked 00:00:00:18。904 QF、113系列、drop 0、disc 0。Stop → `lost`、位置保持 00:00:08.302 | pass（実測、Live） | `artifacts/p0/win-s03-smoke-01.*` | 以降は起点01:00:00:00で実施 |
| S03 C1 Play / Stop / 再開 | Arrangement、起点01:00:00:00、120 bpm | 2026-10-28 03:08 | Play 10 s → Stop 5 s → Play 8 s → Stop | 各PlayでSPP 0 / Start → FF 01:00:00:00 → 約0.66 s後にlock（01:00:00:18）。Stop → holdover → `lost`（01:00:10.369で保持）。再開で新FFから再lock。2056 QF、257系列、drop 0、disc 0、ff 2、locks 2 | pass（実測、Live） | `artifacts/p0/win-s03-c1-arr.*` | lockまで約0.6 sはLiveMTCのQF開始遅延（FFから約0.45 s）。設計課題へ |
| S03 C2 途中位置から開始 / seek | Arrangement。停止中にSPP 254（Live表示16.4.3。位置欄への入力が意図の46.1.1にならなかった）へ移動、再生中にSPP 320へ、SPP 370へ | 2026-10-28 03:09 | `discontinuity` と2系列以内の再lock。古い位置で補間し続けない | 停止中seek：SPP + FF 01:00:31:22。Play：Clock Stop / SPP / Continue、FF 01:00:00:00（+50 ms）→ FF 01:00:32:00（+250 ms）→ lock 01:00:32:18（Continueから0.90 s）。再生中seek：Stop / SPP 320 / Continue → 0.52 s後にFF 01:00:40:15 → acquiring → lock 01:00:41:02（seekから1.09 s）。3回目はseek先が現在位置とほぼ同じで変化なし。Stop → `lost` | pass（挙動）。要検討：再lockが500 msを超える、FF経由のジャンプは `disc` に数えない | `artifacts/p0/win-s03-c2-arr.*` | デコーダーでFFによる位置差をdiscontinuityとして記録する。Play直後の `01:00:00:00` FF（LiveMTC）の扱いを決める |
| S03 C3 短いloop | Arrangement、ループ12.1.1〜16.1.1（4小節 = 8 s、Ctrl+Lで設定） | 2026-10-28 03:14 | loopごとに `discontinuity`（負）と再lock | 8.0 s周期でFF 01:00:22:00 → acquiring → 約0.59 s後にlock 01:00:22:18、3回とも同じ。QFの飛びによる `discontinuity` は0。LiveのClockはループ折返しでSPPを再送しない | pass（挙動）。要検討：loopごとに約0.6 s位置が止まる | `artifacts/p0/win-s03-c3-arr.*` | FF後にClockがplayingなら補間を続ける案を検討 |
| S04 C4 BPM 120→128→90 | Arrangement、再生中にテンポ欄へ入力 | 2026-10-28 03:10 | MTCに意図しないジャンプがない | 128 bpmでFF 01:00:35:19（約−2 s）、90 bpmでFF 01:00:59:14（約+18 s）。各約0.6 s後に再lock。ClockのBPMは127.7〜129.6、90.1〜91.3へ追従 | fail（期待に対して）。原因はLiveMTCが拍位置×現在テンポで時刻を作ること | `artifacts/p0/win-s04-c4-arr.*` | 共通時計の定義を見直す（[07-decisions.md](../07-decisions.md) O12） |
| S03 C5 Session clip変更とビュー切替 | Session、再生中に空のMIDI clipを発火、Tabでビュー切替2回 | 2026-10-28 03:15 | `discontinuity` なし、ゼロへ戻らない | 23 s連続locked、FFはPlay時の1回のみ、disc 0、Clock 119.6〜120.1 bpm | pass（clip発火の成否はLive画面で未確認） | `artifacts/p0/win-s03-c5-ses.*` | — |
| S03 C6 MTC停止 | Arrangement 90 bpm、再生中にMTCデバイスをOFF 5 s → ON | 2026-10-28 03:11 | holdover → lost（位置保持）→ blackout。Clockは独立に `playing` | OFF後 `lost`（01:00:49.634で保持）、Clockはplayingのまま。ON後はFFなしでQFだけ再開 → `discontinuity +162 frames` → 64 ms後にlock | pass（実測、Live） | `artifacts/p0/win-s03-c6c7-arr.*` | — |
| S03 C7 Clock停止 | 同上、再生中にLive出力Port 6のSyncをOFF 6 s → ON | 2026-10-28 03:11 | MTCは `locked` のまま | tick停止（age 5.5 s）、MTCはlocked継続。Sync ON後tickが再開、Start / Continueなし | pass（Clock側の停止判定はage依存） | 同上 | Clockのtick途絶をtransport不明として扱う判定を追加 |
| S03 C8 USB抜き差し | — | — | MTCに影響なし | 未実施。LPX未接続 | not-run | — | S01後 |
| S01 LPXポート・モード・RGB | Windows / Mac | — | identity、mode、RGB、入力、復帰 | 未実施。LPX未接続。`lpx-check` は実装済み（模擬デバイスと単体テストのみ。loopMIDI経由の自己問い合わせを応答として扱わないことを確認） | not-run | — | LPX接続時に `lpx-check --in/--out --duration 30 --run-id win-s01-lpx` を実施 |
| S02 仮想バスとLive入力 | Windows / Mac | — | Note/CCがLiveへ重複なく届く | Live→アプリ方向（Clock）は PM-2 で確認。アプリ→Live方向は `route-check` 実装済み（8n/9n/Bn転送、9n vel 0 → 8n kk 00）だが実ポート・Live到達は未確認 | 一部完了 | `artifacts/p0/win-portmap-02.*` | LiveのRemote ONを設定して `route-check` または `lpx-check --forward "DJSC Control"` で確認 |
| S03 MTCブリッジ両ビュー（C1〜C5） | Windows | 2026-10-28 | runbook B2 | 上記C1〜C3、C5。C4は S04 | 一部完了（Mac未実施） | 上記 | Mac枠M3 |
| S04 テンポ・位相と秒位置 | Windows | 2026-10-28 | runbook B2 C4 | 上記C4 | fail（LiveMTCの時刻がテンポ依存） | 上記 | O12 |
| S05 負荷・切断 | Windows / Mac | — | 30 fps RGBと入力の併用 | 未実施 | not-run | — | S01後 |
| S06 OS互換性matrix | Windows | 2026-10-28 | 環境値の記録 | Windows側は記録済み。Mac側は未取得 | 一部完了 | `environment.md` | Mac枠M0で取得 |
| S07 Link最小検証 | Windows、Live 12.4.7、`monitor --link`（rusty_link 0.4.9） | 2026-10-28 04:4x〜04:5x | SDK/bindingビルド、tempo/phase、途中参加、peer喪失、起動順 | 下記「S07の詳細」。ビルドと観測は動作。Liveの再生・停止（is_playing）、テンポ変更と自動化の追従、peer喪失は観測できた。ただしp0-probe側のStart Stop SyncをONにすると、LiveはPlay後にトランスポートが進まなくなる（2回再現、Live再起動で回復）。OFFなら正常 | 一部合格、重大な注意事項あり | `artifacts/p0/win-s07-link-01..05.*` | アプリ側のStart Stop Syncは既定OFF（O10）。途中参加（Live再生中にアプリ起動）とMacは未実施 |
| 連続2時間試験 | 両OS | — | NFR07 | 未実施 | not-run | — | 機能検証後に別枠で確保 |

## 設計上の確認事項（合成・配管確認から得たもの）

- 2系列整合でのlock所要時間は理論値どおり約8フレーム（約0.17〜0.18 s）。NFR06の500 ms以内の再lockは、合成源では66〜67 msで満たした。Liveの実出力では未確認。
- Full Frame後に系列が来ない間は `acquiring` で位置だけを表示し、進行させない。これは設計の「Full Frameは位置更新でありPlayではない」に一致する。
- 欠落が系列の途中で始まると、残りのpieceは `stray` として数えられ、ラベルを作らない。欠落後の不連続deltaは欠落長に依存し、`+12`（400 ms）や `+10`（300 ms、境界が系列途中）になった。
- 同一ポートにMTCとClockが来る構成では、接続を1つにして `sync` として記録する。2接続で開くと記録が重複する（A3-1）。

## 設計上の確認事項（Live 12 + LiveMTCの実測から得たもの）

- LiveMTC 1.1.0の時刻は「拍位置 × 現在テンポ」の換算値。テンポ変更（C4）、ループ折返し（C3）、seek（C2）はすべてFull Frameで位置を飛ばす。テンポを変える運用では、MTCは曲中の連続した秒位置にならない。
- Full Frameの後、Quarter Frameの送出が始まるまで約0.45 s、2系列整合のlockまで約0.6 s。Play直後には `01:00:00:00`（起点そのもの）のFull Frameが一瞬送られ、約250 ms後に正しい位置のFull Frameが来る。
- Full Frameで位置が飛んだ場合、デコーダーは `discontinuity` を数えない（FFは位置更新として扱う設計のため）。アプリの判断材料としては、FF前後の位置差も不連続として記録する必要がある。
- MTCデバイスをOFF→ONした場合（C6）はFull Frameなしで系列が再開し、QFラベルの飛びとして `discontinuity` が出て64 msで再lockした。
- LiveのMIDI Clock：位置0からのPlayは `SPP 0` → `Start`、位置≠0からのPlayは `Stop` → `SPP` → `Continue`。停止中の位置変更でもSPPとFull Frameが送られる。ループ折返しではSPPを再送しない。Clock Syncを切ると `Stop` なしにtickだけ止まる。
- Live 12.4（Windows）はloopMIDI / rtpMIDIのポートを `<PC名>`、`<PC名> (Port 2)`〜`(Port 6)` と表示する。WinMM（midir、LiveMTC Bridge）は `DJSC Timecode` 等の本来の名前を返す。対応は実測で確認する必要があり、`(Port 6)` = `DJSC Clock` を確認した（列挙順と同じ並び）。
- Liveのトランスポート速度は音声ドライバーのクロックに従う。Voicemeeter Virtual ASIOをVoicemeeter未起動で選ぶと約56%の速度で進んだ。Realtek ASIOは「オーディオエンジン無効」になった。Generic Low Latency ASIO Driverで120 bpm相当を確認した。
- Liveが起動中にloopMIDIを再起動すると、Liveが旧インスタンスのポートを掴んだままになり、新旧どのポートにも何も届かない。loopMIDIのポート追加は、Liveを終了してから行う。

## 実測記録の再解析（ツール改修後、2026-10-28）

新しい測定ではない。`artifacts/p0` の記録を改修後のデコーダーで `replay` した結果（設計担当が再実行して確認）。`ff_disc` はFull Frame前後の位置差が2フレームを超えた件数、`disc` はQuarter Frameのラベル飛び。

| 記録 | disc | ff_disc | locks | ff | stalls | 内訳 |
| --- | --- | --- | --- | --- | --- | --- |
| win-s03-c1-arr | 0 | 1 | 2 | 2 | 0 | 再開時 `lost 01:00:10:11 → 01:00:00:00`（−311） |
| win-s03-c2-arr | 0 | 3 | 2 | 4 | 0 | Play時のFF対、再生中seek、停止中seekは基準なし |
| win-s03-c3-arr | 0 | 5 | 4 | 6 | 0 | 停止中のループ設定による位置変更（+240）、Play（−240）、ループ折返し3回（−241〜−242）。折返しの2回目以降はFF前にQFが止まり `from holdover` |
| win-s04-c4-arr | 0 | 3 | 3 | 4 | 0 | Play時 +960（acquiring）、128 bpmで −73、90 bpmで +526 |
| win-s03-c6c7-arr | 1 | 1 | 2 | 2 | 1 | C6はQF由来 +162。C7は t=25.01 s `stalled`（503 ms無着）、t=31.24 s `resumed`（6738 ms） |

合成fixture `mtc30-play-seek-gap` は既存テストの結果を変えずに `ff_disc=1`（holdover中の 01:00:03:03 → FF 01:00:10:00、+207）が付く。

O13(3)「FF後にClockがplayingなら補間を続ける」の評価（`replay --ff-continue`、既定OFFの試験オプション、56 tests pass）：c3の記録で、Play時と折返し3回のlock時に「lockラベル − 補間予測位置」は +1.40 / +1.88 / +1.49 / +1.47 フレーム（約47〜63 ms）。c1、c2、c6c7でも +1.4〜1.5 フレーム、c4の90 bpm変更時だけ +5.06 フレーム（169 ms）。つまりONにすると折返し中の位置停止は無くなるが、lockの瞬間に約50 ms前へ跳ぶ。一定分はLiveMTCのFull Frame送出時刻とQFラベルのずれと考えられる。判断：LiveMTCは採用しない方向（O12）なので、この送信元に合わせた固定オフセット補正は入れない。オプションは既定OFFのまま残す。

## O12 代替ブリッジの計測実験（Windows、2026-10-28）

Live 12.4.7 のPython MIDI Remote Script `DJSCBridgeProbe`（`tools/live-bridge-probe/`、User Libraryの `Remote Scripts\` に配置、Control Surface 1に割当、Input/Output None）が `Song.get_current_smpte_song_time(ms_time)`、`current_song_time`、`tempo`、`is_playing` を曲中時刻リスナーと約100 msタイマーで読み、UDP 127.0.0.1:9007 へ送る。`p0-probe bridge-listen` が共通の単調時計で受信し、`bridge-report` で集計した。Live API の `TimeFormat.ms_time` / `smpte_30` と `SmpteTime.hours/minutes/seconds/frames` は errors なしで読め、`ms_time` の `frames` はミリ秒だった（回帰の傾きが1.0）。

| 記録 | Set | 操作 | リスナー間隔（再生中） | 再生区間の回帰（smpte ms vs 到着時刻） | 飛び | 判定 |
| --- | --- | --- | --- | --- | --- | --- |
| `win-o12-bridge-01` | 自動化なし | Play → 手動でテンポ120→140→90 → Stop → 先頭からPlay 25 s → Stop | 中央値16.6 ms、p95 17.6 ms、54.9件/s | 4区間すべて傾き0.9999〜1.0001、残差中央値2.8〜3.0 ms、p95 4.2〜4.8 ms、最大9.8〜34.5 ms | 手動テンポ変更で −895 ms（140 bpm）、+6467 ms（90 bpm）。Stopで0へ | 手動テンポ変更（自動化なし）では Live の時刻も拍×現在テンポで飛ぶ。LiveMTCと同じ性質で、Live自体の時刻ルーラーの仕様 |
| `win-o12-bridge-02` | 自動化なし、録音ボタンON | 同上（自動化記録を試みたが記録されず） | 録音中はリスナーが18.6 s止まり、タイマー分だけ届いた | 残りの区間は上と同等 | 手動変更で −147 / +492 ms | Arrangement録音中は曲中時刻リスナーが発火しない（要注意。タイマー経由は届く） |
| `win-o12-bridge-03` | `artifacts/p0/sets/djsc-tempo-automation-test.als`（既定テンプレートからXMLで生成。Song Tempo自動化 120 → 8拍目で140 → 22拍目で90 → 40拍目で120） | 先頭からPlay 31.5 s（テンポが4 s・10 s・22 sで自動変化）→ Stop → 先頭からPlay 12 s → Stop | 中央値16.7 ms、p95 17.6 ms、53.5件/s | 区間#0（31.5 s、n=2194）：傾き0.999971、残差中央値2.85 ms、p95 4.16 ms、最大6.89 ms。区間#1（12 s）：傾き1.000031、残差中央値2.90 ms、p95 4.44 ms、最大6.17 ms | 再生中の飛びなし。Stop→先頭の1回のみ | pass。テンポ自動化をまたいで秒位置が連続し、ジッターは補間で吸収できる範囲 |

補足：`beats − Clock拍` は3記録とも中央値−0.028拍、偏差p95 0.03〜0.04拍（120 bpmで約15〜20 ms）で、Live内の拍位置とMIDI Clockの拍が一致している。UDP配送のゆらぎ（到着時刻 vs Live内 perf_counter）は残差にほぼ差が出ず、ジッターの主因はLiveの約60 Hz更新周期。

## S07の詳細（Link、Windows、2026-10-28）

Live側：Preferences > Link の「開始停止同期」ON、トランスポートのLinkボタンでON/OFF。アプリ側：`monitor --link`（rusty_link 0.4.9、観測のみ、set系とcommitを呼ばない）。Set は `djsc-tempo-automation-test.als`。

| 記録 | 手順 | 観測 | 判定 |
| --- | --- | --- | --- |
| `win-s07-link-01` | LiveがLink ONの状態でアプリ起動 | 起動直後にクラッシュ（`misaligned pointer dereference`、Linkスレッド）。原因はrusty_linkがcallback closureのスタックアドレスをC側へ渡すこと。実装担当がサイズ0のclosure + 静的スロットで回避 | fail → 修正済み |
| `win-s07-link-02` | アプリ先行（peers 0）。Set読み込み後にLiveのLinkをON → Play 6 s → Stop → 停止中にテンポ128 → 120 | Live参加時にLiveのテンポ表示が90（自動化値）→120（アプリpeerのSDK既定値）へ変わった。`peers 0 -> 1`、`is_playing -> true/false`、`tempo 120 -> 128 -> 120` は観測。ただしPlay後にLiveの位置が1.1.1のまま進まず、Clock tickも0（SPPのみ） | 観測はpass。Liveの時刻停止はfail |
| `win-s07-link-03` | Link ONのままPlay/Stop → LiveのLink OFF → Play/Stop | `is_playing` の変化、`peers 1 -> 0`、`transport unknown (no peers)` を観測。Liveは引き続き進まず（Link OFF後も）。Live再起動で回復 | peer喪失の観測はpass |
| `win-s07-link-04` | 正しい順序：Live Link ON（peerなし）→ アプリ起動 → Play 14 s → Stop → Play 5 s | 再び Live の位置が進まない（is_playing は true/false を往復、Clock tick 0、Link tempoは120のまま） | fail（順序では解決しない） |
| 対照 | Live単独でLink ON → Play 6 s | 正常（4.2.2、テンポ自動化140） | pass |
| `win-s07-link-05` | Live Link ON → アプリを `--link-no-start-stop-sync` で起動 → Play 8 s → Stop（先頭へ） | Liveは正常に進む（5.2.1）。Clock 418 tick、`tempo 120 -> 140`（自動化、8.7 s）、停止で先頭に戻ると `140 -> 120`。is_playing はアプリ側がSSS OFFなので受信しない（unknown） | pass |

結論：rusty_link経由でアプリ側のStart Stop Syncを有効にすると、同一セッションのLive 12.4.7はPlay後にトランスポートが進まない。アプリ側のSSSは既定OFFにし、再生状態はMIDI Clock（Start/Stop/Continue）またはブリッジの `is_playing` から取る。Liveが参加するとアプリの既定テンポ（120）がセッションに採用される点も、アプリ先行起動では避けられない。

### Start Stop Syncの切り分け（実装担当、2026-10-28、`win-s07-sss-*`）

peerは `tools/link-sss-probe` の素の `ableton::Link` 版と `ableton::LinkAudio` 版（同梱SDKからビルド、commitしない、`--sss` でSSS ON）、およびp0-probe。Liveの進行は `DJSCBridge` の記録（再生中のサンプルの前進量の合計が2 s超ならADVANCED）で判定。Live側はLink ON、開始停止同期ON、Link Audio OFF。

| 試行 | 条件 | 結果 |
| --- | --- | --- |
| link-01 | Link ON直後の新セッション、素のLink SSS ON、最初の再生 | FROZE（`is_playing` が1 msだけtrue→false、Clockは SPP のみ） |
| after-01 | peerが抜けた後、peerなし | ADVANCED（再起動なしで回復） |
| audio-01 / link-02 / link-03 / p0-02 | 同じセッションのまま、LinkAudio SSS / 素のLink SSS / p0-probe SSS | ADVANCED（link-03は次の小節頭まで約2 s待ってから進行。Linkの量子化開始として正常） |
| link-04 | 新セッション、素のLink SSS | FROZE |
| link-05-nosss | 新セッション、素のLink SSS OFF | ADVANCED |
| audio-02 | 新セッション、LinkAudio SSS | FROZE |
| none-01 | 新セッション、peerなし | ADVANCED |
| link-06-twoplays | 新セッション、素のLink SSS、2回再生 | 2回ともFROZE。peerが抜けてもLiveは `is_playing=true`・位置0のまま、StopもLink OFFも効かず再起動が必要 |

判定：FROZEはSSS ONのpeerがいるときだけ起き、peerの種類（素のLink、LinkAudio、rusty_link）に依存しない。LinkAudio仮説とp0-probe固有説は否定。起きやすい条件は「Liveが新しく入ったセッション（Link OFF→ON直後、起動直後）での最初の再生」。回復はpeer離脱で戻る場合と、Live再起動が必要な場合がある。原因はLive 12.4.7側のStart Stop Sync開始処理にあると考えられる。公式LinkHutでの再現は、素のLink版で同じ結果が出たため未実施。

## B4の結果：自前ブリッジ + MTC出力でのC1〜C7（Windows、Live 12.4.7、2026-10-28）

実施は実装担当（Opusセッション、computer use）。設計担当が記録のreplayと `mtc-out-report` で数値を確認した。構成：`DJSCBridge`（Control Surface 1、v2レコード、`errors` なし。`signature_numerator/denominator` と `loop/loop_start/loop_length` はLive 12.4.7で読めた）→ UDP → `p0-probe mtc-out`（`DJSC Timecode` へ30 NDF出力、同じポートを `--verify-in` で受信）＋ 独立の `monitor`（同じ入力をWinMMで2プロセス同時に開けた）。Link OFF、Clock出力ON、音声はGeneric Low Latency ASIO。Setは `djsc-tempo-automation-test.als`。

全ケース共通：QF送信遅れ p95 0.14〜0.15 ms（最大0.34〜0.65 ms、C5で1回2.71 ms）、送信間隔 p95 8.45〜8.58 ms、piece 0ラベルとブリッジ位置の差 p95 4.2〜6.0 ms。

| ケース | run-id | 観測 | 判定 |
| --- | --- | --- | --- |
| C1 Play / Stop / 再開 | `win-p1-c1` | Play → FF(start) → 受信lock 150 ms。Stop → FF(stop、01:00:10:05)で位置保持。再開ではLiveが `is_playing` を先に出し11 ms後に開始位置へ戻るため FF(start、旧位置) → FF(jump、01:00:00:00) → lock 152 ms。受信MTCとブリッジ位置の差 p95 4.0 ms | pass |
| C2 seek | `win-p1-c2b`（`c2` は操作失敗） | 停止中の位置変更 → FF(hold)。再生中のseek 2回 → FF(jump) → 再lock 133 / 146 ms（LiveMTCでは約1.1 s）。差 p95 3.9 ms | pass |
| C3 短いloop | `win-p1-c3` | 4小節loop（テンポ自動化をまたぐ、1周7.33 s）。折返しごとにブリッジの `discontinuity(playing)` → FF(jump) → 再lock 140 / 141 / 152 ms。`loop` 項目のon/offも正しい | pass |
| C4 テンポ自動化 | `win-p1-c4` | 120 → 140（+2 s）→ 90（+8 s）→ 120（+20 s）の28 s。ブリッジの不連続0、FFはstartとstopのみ、resync 0。受信MTC disc 0 / ff_disc 0、差 p95 4.3 ms / 最大10.4 ms | pass（テンポ変更でMTCが飛ばない） |
| C5 Session clip / ビュー切替 | `win-p1-c5` | 32 s連続で不連続なし、追加FFなし。差 p95 5.3 ms | pass |
| C6 ブリッジ停止 | `win-p1-c6b`（`c6` は記録時間不足） | Control SurfaceをNoneに → meta stop → 250 ms後にQF停止（自走しない）。受信は holdover → lost で位置保持。戻すと script restart → FF(start) → lock 126 ms。Clockは独立にplaying | pass |
| C7 Clock停止 | `win-p1-c7` | Port 6 SyncをOFF 6 s。mtc-outとブリッジに影響なし。monitorのClockが `stalled`（507 ms）→ `resumed`（6681 ms） | pass |

合否基準との照合：seek後の再lock 126〜152 ms（基準500 ms以内）、受信MTCとブリッジ位置の差 p95 3.9〜5.3 ms（基準10 ms以内）、QF送信遅れ p95 0.15 ms（基準1 ms以内）。2時間のドリフト試験は未実施。

設計への反映：(1) ブリッジの `meta stop` を受けたら250 msを待たずに `lost` へ移す。(2) Play時はLiveの `is_playing` の後に位置更新が6〜20 ms遅れて来るので、最初のFull Frameは位置更新を最大50 ms待ってから出す（二重FFを避ける）。(3) 停滞中のサンプルを不連続に数えず、静止した再生中サンプル対で補間とQFを止める。3点とも実装済み（96 tests）。記録のオフライン再生（`mtc_out::simulate`）では、c1の再開が start 1回になり、c6bはmeta stopの時点でQFが止まり、SSS待ちの記録で不連続0・resync 0になった。c2b/c3/c4/c5/c7の回帰も実機結果と一致。改修後バイナリでのLive再確認は未実施。

## 未実施項目の理由

| 項目 | 理由 |
| --- | --- |
| C8、S01、S05 | Launchpad X未接続。`lpx-check` / `route-check` は実装済み（模擬デバイスと単体テストのみ） |
| S02のアプリ→Live方向 | `route-check` は実装済み。LiveのRemote ON設定と実ポートでの到達確認が未実施 |
| S07の残り | Start Stop Sync OFFでの途中参加（Live再生中にアプリ起動）と起動順の網羅、LinkHutによる切り分け（要ダウンロード、利用者判断）、Mac |
| 改修後デコーダーのLive再測定 | `ff_disc` / `stalled` は実測記録のreplayで確認した。改修後バイナリでのLiveMTC再測定は未実施（LiveMTCは不採用方向のため優先度低） |
| Mac全般 | 立ち会い枠を未調整。Mac側の環境値も未取得 |
| 連続2時間試験 | 機能検証を優先。別枠で確保 |
