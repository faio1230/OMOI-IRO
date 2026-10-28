# P0 検証手順書

対象は [docs/09-claude-handoff.md](../09-claude-handoff.md) のP0。ツールは `p0-probe`（2026-10-28のM1で [apps/probe](../../apps/probe/README.md) へ移動）。結果は [p0-results.md](p0-results.md) に記録し、生ログは `artifacts/p0/` に置く。

名前の読み替え（M5冒頭の改名、2026-10-28）：この文書と記録の `DJSC Control / Timecode / Clock` は現在 `OMOI-IRO Control / OMOI-IRO Timecode / OMOI-IRO Clock`、`DJSCBridge` は `OmoiIroBridge`、`p0-probe.exe` は `omoiiro-probe.exe`、`tools/live-bridge` は `live-bridge/`。既存の記録は旧名のまま残す。利用者環境の移行手順：(1) Liveを閉じた状態でloopMIDIに新しい3ポートを作る（順序は loopMIDI → Live。Live起動中のloopMIDI再起動は不可）。旧DJSCポートは新ポートでの確認後に消してよい。(2) LiveのMIDI設定で新ポートの `(Port N)` 番号を確認し直し、Clock出力のSync ON、Control入力のRemote/Track ONを付け替え、旧ポートはOFF。(3) Preferences > Link, Tempo & MIDI でDJSCBridgeのControl SurfaceをNoneにしてLiveを閉じ、`live-bridge\OmoiIroBridge` をUser Libraryの `Remote Scripts\OmoiIroBridge\` にコピーしてLiveを起動し、OmoiIroBridgeを選ぶ（両方とも9007へ送るので同時に選ばない）。

パスの読み替え（M1以降）：この文書のコマンドは `tools\p0-probe` で実行する前提で書いてある。現在はリポジトリーのルートで `cargo build -p p0-probe` し、実行ファイルは `target\debug\p0-probe.exe`、記録先は `artifacts\p0\...`（`..\..\` は不要）。Link付きは `cargo build -p p0-probe --features link`、テストは `cargo test --workspace`（Link付きは `--features p0-probe/link`）。Live側スクリプトと合成送信器は `live-bridge/`（`live-bridge/sim/fake_bridge.py`）。

この手順書は「Liveなしで確認できること」「Windows + Live 12 Suite」「Macの短時間立ち会い枠」に分けている。Windows PCにはLive 12 Suite 12.4.7とLiveMTC 1.1.0を2026-10-28に導入済み（[environment.md](environment.md)）。B1とB2のC1〜C7はWindowsで実施済み（[p0-results.md](p0-results.md)）。

## 共通の約束

- 機器の状態を確認するまでmode変更やLED送信をしない。`ports` は列挙だけで何も送らない。
- 入出力ポートは完全一致名か `#index` で明示する。部分一致で機器を選ばない。
- 各試験で `--run-id` を付け、`artifacts/p0/<run-id>.jsonl` を証拠として残す。コンソール出力も `<run-id>.console.txt` に保存する。
- 合成fixtureと `gen-mtc` の結果はLive・機器との接続成功として記録しない。
- 短い試験の累積を2時間の連続安定性試験の代わりにしない。
- `not-run` は失敗でも成功でもない。実施できなかった理由を結果表に残す。

## A. Liveなしで実施できる確認

### A1 ビルドとテスト

```powershell
cd tools\p0-probe
cargo build
cargo test
```

期待：警告なしでビルド、全テストpass。

### A2 合成fixtureの再生

```powershell
.\target\debug\p0-probe.exe synth
.\target\debug\p0-probe.exe replay fixtures\synthetic\mtc30-play-seek-gap.jsonl
```

期待：`full_frame 01:00:00:00` → `locked 01:00:00:02` → `full_frame 01:00:10:00` → `locked 01:00:10:02` → `discontinuity 01:00:10:28 -> 01:00:11:12 (delta +12 frames)` → `locked 01:00:11:14`、最終状態 `lost` + blackout、`drop=0`。

### A3 仮想バス経由のループバック（Windows）

loopMIDIを起動し、`loopMIDI Port` が `ports` に出ることを確認する。

ターミナル1：

```powershell
.\target\debug\p0-probe.exe monitor --mtc-in "loopMIDI Port" --clock-in "loopMIDI Port" --duration 11 --interval-ms 1000 --run-id win-loopback-NN --out ..\..\artifacts\p0\win-loopback-NN.jsonl
```

ターミナル2（1秒後）：

```powershell
.\target\debug\p0-probe.exe gen-mtc --out "loopMIDI Port" --seconds 8 --clock-bpm 120 --gap-ms 400
```

期待：Full Frame受信 → 2系列でlocked → 中間点で `discontinuity (delta +12 frames)` と再lock → 送信終了後に `lost`。`drop=0`、`q_drop=0`、Clockは約120 bpm、Start/Stopを検出。これはOS MIDI経路とツールの配管確認であり、LiveのMTC/Clock出力の検証ではない。

## B. Windows + Live 12 Suite

### B1 準備

1. Liveを終了した状態でloopMIDIを起動し、`DJSC Control`、`DJSC Timecode`、`DJSC Clock` の3ポートを作る。GUIではloopMIDI画面で名前を入力して `+`。GUIを使わない場合は `HKCU:\Software\Tobias Erichsen\loopMIDI\Ports` にポート名のDWORD値（1）を追加してからloopMIDIを起動する（loopMIDIは終了時にこのキーを書き戻すので、停止中に追加する）。Liveが起動中にloopMIDIを再起動すると、Liveが旧インスタンスのポートを掴んだままになり、既存ポートを含めて何も流れなくなる。必ず loopMIDI → LiveMTC Bridge → Live の順に起動し、`ports` で3ポートを確認してからLiveを起動する。
2. LiveMTC（showsync、1.1.0）を導入する。配布zipの `LiveMTC Bridge` フォルダーを丸ごと置き（このPCでは `%LOCALAPPDATA%\Programs\LiveMTC Bridge\`。Program Filesは管理者権限が要る）、`MTC.amxd` をLiveのUser Libraryへ置く。User Libraryの場所はPreferences > Libraryで確認する。このPCでは `%USERPROFILE%\OneDrive\ドキュメント\Ableton\User Library\`（`Documents\Ableton\User Library` ではない）。置いた直後はブラウザーの検索に出ないことがあるので、サイドバーの「ユーザーライブラリ」からフォルダーを開く。
3. Preferences > Audio のDriver TypeをASIOにする。このPCの結果：Realtek ASIOは「オーディオエンジン無効」になる。Voicemeeter Virtual ASIOはVoicemeeter本体が起動していないとトランスポートが約56%の速度で進む（PM-1）。Generic Low Latency ASIO Driver（441 samples）で120 bpm相当を確認した。ASIOを選ぶとトランスポートにLinkボタンが現れる。音声デバイス変更後はPlayしてClockのBPMと位置表示が合うことを確認する。
4. LiveのPreferences > Link, Tempo & MIDI。Live 12.4（Windows）はloopMIDI / rtpMIDIのポートを `<PC名>`、`<PC名> (Port 2)`〜`(Port 6)` と表示し、WinMM名（`DJSC Timecode` 等）では表示しない。対応はWinMMの列挙順と同じ並びだったが、Clock出力で実測して確認する（出力 `(Port N)` のSyncをONにしてPlayし、`monitor --clock-in "DJSC Clock"` でtickが来る番号を特定する）。このPCでは `(Port 6)` = `DJSC Clock`、`(Port 5)` = `DJSC Timecode`、`(Port 4)` = `DJSC Control`（Port 5、4は列挙順からの推定）。
   - Input `DJSC Control`：Track OFF（録音時のみON）、Sync OFF、Remote ON（MIDI Map時）。
   - Output `DJSC Clock`：Clock方式を試すときだけ Sync ON（Song）。Link方式ではOFF。
   - Input/Output `DJSC Timecode`：すべてOFF（LiveMTCからアプリへの専用経路）。
   - 既定ではすべての入力でTrackがONになっているので、`(Port 2)`〜`(Port 6)` のTrackをOFFにする。
   - Launchpad実ポート：Track/Sync/Remote すべてOFF、Control Surface未割当。
5. LiveMTC Bridgeを起動し（loopMIDIより後。Bridgeは起動後にポートを再列挙しない）、MIDI Port を `DJSC Timecode`、Timecode Rate を `30 FPS`（既定は23.976）にする。Live Setで `MTC.amxd` をMainトラックへ置くと「Connected to LiveMTC Bridge」になる。デバイスの Live Set Offset（Hours）に試験用の `1` を入れて起点を `01:00:00:00` にする（運用上の確定値ではない）。
6. Linkは最初はOFF。S07でON、Start Stop SyncをONにする。
7. ループバックが起きていないことを確認する：`ports` 列挙後、`monitor --mtc-in "DJSC Timecode" --clock-in "DJSC Clock"` を起動し、Live停止中にQuarter Frameもtickも来ないことを確認する。

操作上の注意（computer useやキー操作で行う場合）：Liveの位置欄・テンポ欄はクリック後に数値を入力してEnter。位置欄は `46.1.1` のような入力が意図どおりに入らなかったので、入力後に表示を確認するか、SPPの値で記録を読む。ArrangementのループはトラックレーンでクリックとShift+クリックで時間範囲を選び、Ctrl+Lで設定とON。Stopは位置を開始点へ戻すので、再開試験はそのまま再度Play。

### B2 同期ケース（S03 / S04）

同じLive SetでArrangementとSessionを順に実施する。`monitor` を `--mtc-in "DJSC Timecode" --clock-in "DJSC Clock"` で起動し、ケースごとに `--run-id win-s03-<case>-<view>` を変える。Pocket MIDI等の第二モニターを同じポートに付けて比較できる。

| ケース | 操作 | 期待（MTC） | 期待（Clock / Link） |
| --- | --- | --- | --- |
| C1 Play / Stop / 再開 | 起点からPlay、10 s後Stop、5 s後Play | Play後に `locked`。Stop後は `holdover` → `lost` で位置保持。再開で新Full Frameまたは連続系列から再lock。Stop直後の数個のQFで再生へ戻らない | Clock: Start/Stop/Continue。Link: transport通知と順序 |
| C2 途中位置から開始 / seek | 1:30から開始、再生中に0:30へ戻し、3:00へ進める | `discontinuity` を出し、移動先で2系列以内に再lock。古い位置で補間し続けない | Clock: SPP。Link: 拍位相の再収束 |
| C3 短いloop | 4拍loopを20回 | loopごとに `discontinuity`（負のdelta）と再lock。前周の位置が残らない | 位相が各周で連続 |
| C4 BPM 120→128→90 | 再生中にtempo変更 | MTCに意図しないジャンプがない（`disc` が増えない）。増えた場合はdeltaと時刻を記録 | BPMが追従 |
| C5 Session clip変更とビュー切替 | Sessionでclipを切替、Arrangementへ戻す | `discontinuity` なし。ゼロへ戻らない | 位相連続 |
| C6 MTC停止 | LiveMTCデバイスを無効化 | `holdover`（250 ms）→ `lost`（位置保持）→ blackout（1 s） | Clock/Linkは独立に `playing` のまま |
| C7 Clock停止 / Link peer喪失 | Sync出力OFF、またはLinkをOFF | MTCは `locked` のまま | 拍側だけ品質低下 |
| C8 USB抜き差し | （LPX接続後）USBを抜いて挿す | MTCに影響なし | — |

記録後、`replay` で同じ結果が再現することを確認し、結果表に run-id、pass/fail/not-run、相対パスを書く。

Windowsでの実施結果（2026-10-28）は [p0-results.md](p0-results.md) にある。LiveMTCはseek・ループ・テンポ変更のすべてをFull Frameで反映し、QFの再開まで約0.45 s、lockまで約0.6 sかかる。C2の「2系列以内に再lock」はFull Frame後の2系列という意味では満たすが、seekからの時間では約1.1 sになる。C4の「MTCに意図しないジャンプがない」はLiveMTCでは成立しない（時刻が拍位置×現在テンポのため）。

### B3 Link（S07）

`--features link` のビルドにはlibclangが必要（[environment.md](environment.md)）。導入後に `monitor --link` を実装・使用する。確認項目：検出（peer数）、tempo変更の追従、位相、Start Stop Syncの有効/無効、再生途中の参加（初期 `is_playing=false` をStopと扱わない）、Live終了、起動順変更。アプリ側からtempo・transportを送らない。

### B4 自前ブリッジ + MTC出力でのC1〜C7再実施（P1、未実施）

[10-live-bridge.md](../10-live-bridge.md) の方式で、B2と同じケースを再実施する。LiveMTC Bridge と MTC.amxd は使わない（MTC.amxd をSetから外し、`DJSC Timecode` へ送るのは `mtc-out` だけにする）。

準備：loopMIDI（DJSC Control / Timecode / Clock）→ Live起動。Control Surface 1 = `DJSCBridge`（User Library の `Remote Scripts\DJSCBridge\`。`DJSCBridgeProbe` は外す）。Clock出力 `(Port 6)` Sync ON。Link OFF。Set は `artifacts/p0/sets/djsc-tempo-automation-test.als`。最初の実機確認では、ブリッジの記録に `errors` が無いこと（拍子・ループのAPI名は未確認）を先に見る。

起動（`tools\p0-probe`）：

```powershell
# A: ブリッジ受信 → MTC生成 → 同じポートを受けて自己検証
.\target\debug\p0-probe.exe mtc-out --out-port "DJSC Timecode" --verify-in "DJSC Timecode" --origin 01:00:00:00 --run-id win-p1-c<N> --record ..\..\artifacts\p0\win-p1-c<N>.jsonl
# B: 比較用の独立モニター（WinMMで同じ入力を2つ開けなければ A の --verify-in を外す）
.\target\debug\p0-probe.exe monitor --mtc-in "DJSC Timecode" --clock-in "DJSC Clock" --run-id win-p1-c<N>-mon --out ..\..\artifacts\p0\win-p1-c<N>-mon.jsonl
# 終了後
.\target\debug\p0-probe.exe mtc-out-report ..\..\artifacts\p0\win-p1-c<N>.jsonl
.\target\debug\p0-probe.exe replay ..\..\artifacts\p0\win-p1-c<N>-mon.jsonl
```

| ケース | 操作 | 期待（ブリッジ） | 期待（MTC出力と受信） |
| --- | --- | --- | --- |
| C1 Play / Stop / 再開 | B2と同じ | `disc(playing)=0`、`start` / `stop` の局面 | PlayでFF(start)→QF、受信lock 500 ms以内。StopでFF(stop)、受信は holdover→lost で位置保持。再開でFF(start)→再lock |
| C2 seek | 停止中と再生中 | `discontinuity(stopped / playing)` | 停止中はFF(hold)、再生中はFF(jump)→再lock 500 ms以内 |
| C3 短いloop | 4小節loop | 折返しごとに不連続（Liveの出し方は実測で記録） | 折返しごとにFF→再lock 500 ms以内 |
| C4 テンポ自動化 | 120→140→90→120 | 再生中の不連続0 | 受信MTCとブリッジ位置の差 p95 10 ms以内、`resync` 0。手動テンポ変更では不連続とFFが出る（仕様どおり） |
| C5 Session clip / ビュー切替 | B2と同じ | 不連続なし | FF増えず |
| C6 ブリッジ停止 | Control Surface を None にする、戻す | `lost` → 戻すと script restart | QF停止（自走しない）→ FF(start) → 再lock |
| C7 Clock停止 | Port 6 Sync OFF → ON | 影響なし | 影響なし。monitorのClockは `stalled` |

合否基準：seek後の再lock 500 ms以内、残差 p95 10 ms以内、QF送信遅れ p95 1 ms以内、2時間試験は別枠。

Windowsでの実施結果（2026-10-28、すべて合格）は [p0-results.md](p0-results.md) B4。操作上の注意：Arrangementのスクラブ領域（小節番号の下の帯）をクリックすると、停止中は「そこから再生」になる。停止中の位置変更はトラックレーンのクリック（挿入マーカー）で行う。Control SurfaceをProbeからDJSCBridgeへ切り替えると `seq` が1に戻り、受信側は script restart として扱う。WinMMでは同じ入力ポートを2プロセスで同時に開けた。

## C. Launchpad X（S01 / S02）

`lpx-check` と `route-check` は未実装。実装後の手順は以下の順にする。

1. `ports` でLPXの2系統（WindowsではDAW用とMIDI用の表示名が異なる）を識別し、MIDI用の入出力を完全一致名で指定する。
2. Device Inquiry（`F0 7E 7F 06 01 F7`）で応答とfirmware版を記録する。
3. Mode照会（`F0 00 20 29 02 0C 0E F7`）で現在のmodeを記録する。
4. Programmer Modeへ移行（`... 0E 01 F7`）し、低輝度の単色（例：RGB 10/0/0）を1パッドへ送る。
5. パッド押下・解放を受信し、Note On velocity 0をNote Offへ正規化して `DJSC Control` へ転送した記録を残す。
6. 通常終了とCtrl+Cの両方で消灯とLive Modeへの復帰（`... 0E 00 F7`）を試み、失敗したら表示する。
7. LiveのControl SurfaceにLaunchpad Xが割り当てられていないことを確認する。

## D. Mac の短時間立ち会い枠

各枠15〜20分。放置しない。枠ごとに環境情報（機種、macOS版、Live/Max版、LPX firmware、rustup target）をまず記録する。

| 枠 | 事前準備 | 枠内の操作 | 記録 |
| --- | --- | --- | --- |
| M0 環境 | `rustup target list --installed`、`cargo build` をMacで実行できるか | OS/Live/Max/LPX firmwareの取得、IAC Driverの有効化、`ports` 列挙 | environment.md のMac欄 |
| M1 LPX | `lpx-check` 実装済み | C. の1〜7 | S01, S02 |
| M2 Link | libclang（Xcode CLT）で `--features link` をビルド済み | B3の項目 | S07 |
| M3 同期 | LiveMTC導入、Live Set準備、IACバス3本 | B2のC1〜C5 | S03, S04 |
| M4 併用・障害 | M1〜M3合格 | LED更新と入力の併用、C6〜C8 | S05 |

Macの作業はWindowsで準備したコマンド列をそのまま使い、枠内では実行と記録だけを行う。解析とreplayはMacを占有せずに行う。2時間の連続試験は別枠として扱い、確保できるまで `not-run` のまま残す。
