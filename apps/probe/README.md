# omoiiro-probe（apps/probe）

OMOI-IRO の技術検証用CLI。M1（2026-10-28）で `tools/p0-probe` から移動し、デコーダー・コーデック・解析はworkspaceのcrate（`omoiiro-runtime`、`omoiiro-midi`、`omoiiro-devices`）へ移した。このCLIは引数の解析、ポートの接続、スレッド、表示、JSONL書き込みだけを持つ。パッケージ名と実行ファイル名はM1では `p0-probe` のままとし、M5冒頭に `omoiiro-probe` へ改名した。コマンド・表示・JSONL形式は移動前と同じ。コマンドはリポジトリーのルートで実行し、実行ファイルは `target\debug\omoiiro-probe.exe` になる。

元の説明：P0 技術検証用CLI。設計資料の [docs/09-claude-handoff.md](../../docs/09-claude-handoff.md) にある最小の同期モニターにあたる。workspaceのメンバー（`apps/probe`）。

## できること

| サブコマンド | 動作 | 機器への送信 |
| --- | --- | --- |
| `ports` | MIDI入出力ポートを列挙する | なし |
| `monitor` | 指定した入力ポートでMTC（Quarter Frame / Full Frame）と、必要ならMIDI Clockを観測し、JSONLへ記録する | なし |
| `replay` | JSONL記録をデコーダーへ流し直し、イベントと最終状態を表示する | なし |
| `synth` | 合成fixture `fixtures/synthetic/mtc30-play-seek-gap.jsonl` を生成する（`synthetic` と明記） | なし |
| `gen-mtc` | 合成の30 NDF MTC（と任意でMIDI Clock）を明示した仮想バスへ送る。配管確認用で、Launchpadらしい名前のポートは拒否する | 指定した仮想バスのみ |
| `lpx-check` | Launchpad Xの Device Inquiry → Mode照会 → Programmer Mode → 低輝度RGB 1パッド → 押下・解放の受信 → 消灯とLive Mode復帰（runbook C） | 指定したLPX出力。応答を確認するまでmode変更もLED送信もしない |
| `route-check` | 入力（LPXのMIDIポート、またはテスト用の任意入力）のNote/CCを `OMOI-IRO Control` へ転送し、記録する | 指定した出力（既定 `OMOI-IRO Control`） |
| `bridge-clock` | OmoiIroBridge（[live-bridge](../../live-bridge/README.md)）のUDPサンプルから共通時計を作る（健全性、不連続、停滞、補間）。MTC / Clockと同じ記録に入れる。旧名 `bridge-listen` も同じ動作 | なし |
| `transport` | Runtime の `Transport`（ブリッジ + MIDI Clock または Link）を1秒ごとに表示し、任意でJSONLに記録する（描画まで行うのは `follow`） | なし |
| `follow` | headless追従モード（M4）：ブリッジ → `Transport` → Core render（showのfps）→ LED writer（既定は仮想Launchpad X、任意で出力ポート）。`--control-in` で操作経路（→ `OMOI-IRO Control`）も同時に動かす | 既定はなし（仮想LPX）。`--led-out` / `--control-in` を指定したときだけ、そのポート。Launchpadらしい名前は `--allow-device` なしでは拒否（実機はM6） |
| `q01` | Q01 機器なしの通し試験（headless）：空のショーから作成・保存・再読込・任意位置プレビューと `render` の一致・合成ブリッジでの追従（fake backend）・失敗系・計測。結果は `apps/desktop/docs/q01-files/q01-headless-result.json`。合成源 | なし（fake backend、プロセス内の UDP） |
| `bench-render` | NFR03の計測：64パッド8レイヤーの負荷showを30 fpsの時刻で評価し、render時間の分布を出す | なし |
| `render` | showファイルを任意のshow時刻で描画し、8 x 8のグリッドをテキストかJSONで出す（機器不要。`omoiiro-store` で読込・検証、`omoiiro-core` で描画） | なし |
| `bridge-report` | bridge-clockの記録を解析する（到着間隔、回帰、飛び、MTC / Clockとの差） | なし |
| `mtc-out` | ブリッジのサンプルからMTC（30 / 25 / 24 NDF）を生成して出力する。`--verify-in` で出力を受け直して記録できる | 指定した出力（既定 `OMOI-IRO Timecode`）。Launchpadらしい名前は拒否 |
| `mtc-out-report` | mtc-outの記録を解析する（QF送信時刻、Full Frame後の受信lock、ラベルと位置の差） | なし |

`monitor --link` はAbleton Linkの観測（`--features link` でビルドしたときだけ）。Linkへは何も送らない。

### 実機確認の状況（2026-10-28時点、Windows）

2026-10-28（M5冒頭）に識別子を改名した：`p0-probe` → `omoiiro-probe`、`DJSCBridge` → `OmoiIroBridge`、ポート `DJSC Control / Timecode / Clock` → `OMOI-IRO Control / Timecode / Clock`、crate `show-*` → `omoiiro-*`。下の表と `artifacts/p0`・`fixtures/live` の記録は改名前の名前で行った実測で、記録内のポート名は旧名のまま。改名の前後で、fixtures の replay・bridge-report・mtc-out-report・render の出力70件はバイト一致した。

結果の正本は [docs/validation/p0-results.md](../../docs/validation/p0-results.md)。この表は各サブコマンドがどこまで確かめられているかの目安で、「実測」はOSのMIDI経路・Live・機器を通した確認、「合成」は合成源や模擬デバイスによる確認を指す。Macは全項目が未実施。

| サブコマンド | 実測 | 合成・単体テストのみ | 未実施 |
| --- | --- | --- | --- |
| `ports` | loopMIDI / 旧DJSCポートの列挙 | — | LPX接続時の表示名 |
| `monitor`（MTC / Clock） | Live 12.4.7 + LiveMTC 1.1.0でrunbook B2のC1〜C7（S03、S04）、Live Clock経路（PM-2） | ループバック（A3-1〜3） | C8（USB抜き差し） |
| `monitor`の `ff_disc` / `stalled` | 実測記録（c3、c4、c6c7）のreplayで確認 | 単体テスト | 改修後のバイナリでのLive再測定 |
| `monitor --link` | S07（`win-s07-link-02..05`）：peers、tempo追従（自動化を含む）、is_playing、peer喪失を観測。Start Stop Sync ONではLiveのトランスポートが進まない（既定OFFに変更） | Liveなしの2プロセス試験 | Start Stop Sync OFFでの途中参加・起動順の網羅、LinkHutでの切り分け |
| `follow` | — | `live-bridge/sim/fake_bridge.py`（合成）で16秒の配管確認（lock、seek / loopの飛び、Blackout、reload、stop、lost）。fake backend と fake clock による単体テスト（低速LED、抜き差し、queue overflow） | Live、実機LPX（M6）、`--control-in` を実ポートで |
| `bench-render` | — | このPCでの計測（下記）。機器の試験ではない | Mac |
| `replay` | 実測記録の再解析 | 合成fixture | — |
| `replay --ff-continue` | 実測記録（c1〜c4、c6c7）の再解析。O13で固定オフセット補正は不採用 | 単体テスト | — |
| `gen-mtc` | — | ループバックの合成源として使用 | — |
| `lpx-check` | — | 模擬デバイスによる手順テスト。loopMIDI経由で、応答なしのとき止まることを確認 | S01（LPX未接続） |
| `route-check` | — | 単体テスト（正規化、記録） | S02のアプリ→Live方向 |
| `bridge-listen` / `bridge-report` | DJSCBridgeProbe（Live 12.4.7）で `win-o12-bridge-01..03` を計測し、合格 | 偽の送信器による配管確認、単体テスト | — |
| `bridge-clock`（Bridge decoder） | DJSCBridge（v2、現 OmoiIroBridge）を Live 12.4.7 で動かし、runbook B4 の C1〜C7（`win-p1-c*`）と S07 の切り分け（`win-s07-sss-*`）で使用。sig / loop の読み取りも確認 | 実測記録のreplay、`sim/fake_bridge.py`、単体テスト | Mac |
| `mtc-out` / `mtc-out-report` | B4 の C1〜C7 で合格（seek後の再lock 126〜152 ms、受信MTCとブリッジ位置の差 p95 3.9〜5.3 ms、QF送信遅れ p95 0.15 ms以下）。50 ms待ちと meta stop の即停止は、改修後に記録のオフライン再生で確認しただけ | 合成源の自己検証、オフライン再生、単体テスト | 改修後のバイナリでのLive再確認、2時間のドリフト試験、Mac |

## ビルドとテスト

ビルド手順（Link付きを含む）はリポジトリールートの [BUILD.md](../../BUILD.md)。

```powershell
# リポジトリーのルートで実行する
cargo build
cargo test
```

2026-10-28（M6 準備）：workspace全体で 212 tests pass（`apps/probe` 33、`omoiiro-runtime` 101、`omoiiro-core` 37、`omoiiro-midi` 15、`omoiiro-store` 10、`omoiiro-devices` 8、`omoiiro-desktop` 8）。`--features omoiiro-probe/link` では Link SDK側の2件を加えて214件。M3時点は149件、Schema 0.1.1で150件、M4で173件、U01で176件、U03で179件、U02で184件、U06で186件、U04で192件、U05で196件、安全上限で202件、U07で205件、Q01で206件。

| テスト | 場所 | 内容 | 区分 |
| --- | --- | --- | --- |
| unit | `crates/omoiiro-runtime/src/**`（bridge、mtc、clock、mtc_out、link、diagnostics）、`crates/omoiiro-midi/src/**`（ports、routing）、`crates/omoiiro-devices/src/launchpad_x`、`apps/probe/src/**`（lpx の手順、route の記録、mtc-out の集計） | デコーダー、SysEx組み立てと応答解析、LPXの手順（模擬デバイス）、転送の正規化 | 合成 |
| `apps/probe/tests/render_cli.rs` | `examples/demo.show.json` | `render` の出力（ラベル、グリッド、決定性、不正showのエラー） | 合成 |
| `apps/probe/tests/replay_synthetic.rs` | ルートの `fixtures/synthetic/` | 合成fixture | 合成 |
| `apps/probe/tests/replay_live.rs` | ルートの `fixtures/live/` | `artifacts/p0/` の実測記録のコピー（`*.bridge.jsonl` は bridge sample と meta だけを抜き出したもの）をreplay | 実測記録の再生。新しい機器・Live測定ではない |

### モジュールの移動先（M1）

| 旧 `tools/p0-probe/src` | 新しい場所 |
| --- | --- |
| `mtc.rs` / `midiclock.rs` / `bridge_clock.rs` / `mtc_gen.rs` | `omoiiro-runtime::sync::{mtc, clock, bridge, mtc_out}`（`desired_from` と `PLAY_SETTLE_US` は `sync::mtc_out` へ） |
| `bridge.rs` の UDP受信 | `omoiiro-runtime::sync::bridge_ingress` |
| `bridge.rs` の解析 / `mtc_out.rs` の `analyse`・`simulate` | `omoiiro-runtime::diagnostics::{bridge_report, mtc_out_report}` |
| `record.rs` / `report.rs` / `replay.rs`（`run` を除く） | `omoiiro-runtime::diagnostics::{record, report, replay}` |
| `link.rs` | `omoiiro-runtime::link`（feature `link`） |
| `clock.rs` / `ports.rs`（`print` を除く） | `omoiiro-midi::{clock, ports}`（`omoiiro-runtime::clock` として再公開） |
| `route.rs` の `route` / `Routed`、`RawIn` / `make_callback` / `MidiSend` | `omoiiro-midi::{routing, backend}`（M4で `MidiBackend` 契約、`Router`、fake backend を追加。`monitor` / `transport` / `route-check` の入力は `MidiBackend` 経由） |
| `lpx.rs` の SysEx・応答解析 | `omoiiro-devices::launchpad_x` |
| 実行・表示部分（`monitor`、`gen`、`synth`、`lpx-check` のセッション、`route-check`、`mtc-out` の実行、各 report の表示、`ports` の表示） | `apps/probe/src` |

## 使い方

```powershell
# ポート列挙。何も送らない
.\target\debug\omoiiro-probe.exe ports
.\target\debug\omoiiro-probe.exe ports --json

# MTCとClockを観測して artifacts/p0/<run-id>.jsonl に記録（Ctrl+C か --duration で終了）
.\target\debug\omoiiro-probe.exe monitor --mtc-in "loopMIDI Port" --clock-in "loopMIDI Port 1" --run-id win-s03-01 --out artifacts\p0\win-s03-01.jsonl

# 同じポートにMTCとClockが来る場合は同じ名前を渡す。1回だけ開き、記録のsourceは `sync` になる
.\target\debug\omoiiro-probe.exe monitor --mtc-in "#1" --clock-in "#1" --duration 30

# 記録の再生
.\target\debug\omoiiro-probe.exe replay artifacts\p0\win-s03-01.jsonl
.\target\debug\omoiiro-probe.exe replay fixtures\synthetic\mtc30-play-seek-gap.jsonl --json

# Liveなしの配管確認：別ターミナルで monitor を起動してから合成MTCを送る
.\target\debug\omoiiro-probe.exe gen-mtc --out "loopMIDI Port" --seconds 8 --clock-bpm 120 --gap-ms 400
```

ポート指定は「完全一致の名前」「`#index`」「一意に決まる部分一致（大文字小文字無視）」のいずれか。複数に一致する部分文字列はエラーにする。`lpx-check` と `route-check` は送信を伴うので、完全一致の名前か `#index` だけを受け付ける。

### Full Frame後の補間継続（O13の試験オプション、既定OFF）

```powershell
.\target\debug\omoiiro-probe.exe replay --ff-continue fixtures\live\win-s03-c3-arr.jsonl
```

`Params.ff_continue_when_clock_playing` を有効にすると、MIDI Clockが `playing` の間に来たFull Frameでは、次のlockまで位置をFull Frameから実時間で進める（Clockが止まった時点で固定）。既定では従来どおり位置を止める。lock時に `freewheel_locked <label> predicted=... (label - predicted ±N frames)` を出し、summaryの最後（`--json` では `ff_continue_when_clock_playing.locks`）に各lockの差をまとめる。今のところreplay専用で、`monitor` には組み込んでいない。

## Link観測（monitor --link、runbook B3）

```powershell
# LiveのLinkを先にONにしてから起動する。MTC / Clockと同時に記録できる（同じ単調時計）
.\target\debug\omoiiro-probe.exe monitor --link --mtc-in "OMOI-IRO Timecode" --clock-in "OMOI-IRO Clock" --run-id win-s07-link-01 --out artifacts\p0\win-s07-link-01.jsonl
# Start Stop Syncをこのpeerで有効にする（調査用。下記の注意を参照）
.\target\debug\omoiiro-probe.exe monitor --link --link-start-stop-sync --duration 30 --run-id win-s07-link-sss-on
```

**注意（2026-10-28 実測、S07）**：このpeerでStart Stop SyncをONにすると、同じセッションのLive 12.4.7はPlay後にトランスポートが進まなくなった（Liveの `is_playing` はtrueになるが、曲中時刻は0のまま、Clock tickも0）。アプリとLiveのどちらを先に起動しても再現し、LiveのLinkをOFFにしても戻らず、Liveの再起動が必要だった。このため既定はOFFにしている。OFFでは `is_playing` を受け取らないので、transportは常に `unknown` になる。ONは `--link-start-stop-sync` を明示したときだけで、旧オプション `--link-no-start-stop-sync` は受け付けるが何もしない。

- 観測するもの：peer数、tempo、beat、phase（quantum 4）、is_playing。SDKのcallback（num_peers / tempo / start_stop）で変化を即時に受け、1秒ごとにapp session stateをcapture（commitしない）する。
- Linkへは何も送らない。tempo・beat・transportを設定するAPIも、session stateのcommitも呼ばない（`omoiiro-runtime::link` のテストで呼び出しがないことを検査している）。
- transportの表示：peer数0なら `unknown`。peerがいて `is_playing=true` なら `playing`。`stopped` になるのは、peerがいる間に `is_playing` の変化を観測した後だけ。参加直後の `is_playing=false` は `unknown` のまま。peerが0になると `unknown` に戻る。
- イベント：`peers a -> b`、`tempo x -> y`、`is_playing -> true (peers n)` を別々に出す。peerがいない間の変化は `(no peers; transport stays unknown)` と付けて出す。
- 記録（`source = link`）：callbackの生通知は `kind = notify`、変化は `kind = event`、1秒ごとのサンプルは `kind = state`（`link_clock_us` はLink自身の時計で、`t_mono_us` とは比較しない）。meta startに `start_stop_sync`、`quantum`、SDKの初期tempo（120）を、meta endに最終状態を書く。
- tempoの表示：peerがいる間だけ表示し、peer数0では `--` にする。単独のときにSDKが返すのは自身の初期tempo（`AblLink::new` の120）で、Liveの値ではないため、イベントにも `(local, no peers)` を付ける。
- 起動順：このツールのSDKは、単独のときtempo 120のセッションとして動く。こちらからはcommitしないが、アプリを先に起動してからLiveでLinkをONにすると、Liveのtempoがアプリの120に置き換わった（2026-10-28 実測、自動化値90 → 120。Linkのセッション統合規則どおり）。必ずLiveのLinkを先にONにしてから起動する。
- 初回起動時にWindowsファイアウォールがUDP受信の許可を求めることがある（Linkはmulticast UDP 20808）。

表示行の例（形式。実測値ではない）：

```
[   12.000s] | LINK peers=1 120.00bpm playing  beat=37.52 phase=1.52/4 is_playing=true | q_drop=0
```

## bridge-listen / bridge-report（代替ブリッジの計測、O12）

```powershell
.\target\debug\omoiiro-probe.exe bridge-listen --mtc-in "OMOI-IRO Timecode" --clock-in "OMOI-IRO Clock" --run-id win-o12-bridge-01 --out artifacts\p0\win-o12-bridge-01.jsonl
.\target\debug\omoiiro-probe.exe bridge-report artifacts\p0\win-o12-bridge-01.jsonl [--json]
```

- `bridge-listen` は `monitor` にUDP受信（既定 `127.0.0.1:9007`、`--bind` で変更）を加えたもの。受信スレッドは到着時刻（`t_mono_us`）の採取、コピー、`try_send` だけを行う。記録は `source = bridge`、`kind = sample`、`payload` = データグラムのJSON（解析できなければ `raw` と `parse_error`）。`--mtc-in` / `--clock-in` / `--duration` / `--run-id` / `--out` は `monitor` と同じ。表示行は1秒ごと。
- `bridge-report` の出力：
  - リスナー `song_time` サンプル（再生中）の到着間隔の中央値 / p95 / 最大と、1秒あたりの件数。タイマー（約100 ms）の到着間隔。
  - 再生区間ごとに、smpte ms と到着時刻（ms）の最小二乗直線の傾きと、絶対残差の中央値 / p95 / 最大。Live内の `perf_ns` に対する回帰も並べて出す。到着時刻側の残差にはUDP配送とomoiiro-probe側のゆらぎが含まれ、`perf_ns` 側はLive内で読めた時刻のゆらぎに近い。区間は飛びの位置で分割する。
  - 飛び：再生中に「smpte msの差 − 到着時刻の差」の絶対値が20 msを超えたもの。停止中に20 msを超えて動いたものも出す。前後のtempoを併記する（seek / loop / テンポ変更の区別は操作ログと突き合わせて付ける）。
  - 同じ記録にMTC / Clockがあれば、各再生サンプルの時刻で、MTC補間位置（lockedのときだけ）との差 `smpte ms − MTC ms`、Clock拍（playingのときだけ）との差 `beats − Clock beats` を出す。生の中央値（LiveMTCの起点オフセットを含む）と、中央値からの絶対偏差の中央値 / p95 / 最大。
- 単体テストは合成サンプルによるもの（`bridge.rs`）。Liveなしの配管確認として、偽の送信器（probe_coreと偽のSong）からローカルUDPで2 s送り、bridge-listen → bridge-reportで144件、間隔中央値16.3 msを確認した（合成源。Liveでの計測ではない）。

## follow（headless追従モード、M4）

```powershell
.\target\debug\omoiiro-probe.exe follow --show examples\demo.show.json --view
.\target\debug\omoiiro-probe.exe follow --show examples\demo.show.json --control-in "#2" --out artifacts\p0\win-m4-follow-01.jsonl
```

- スレッド：render（`omoiiro_runtime::follow::spawn_render`）、LED writer（`spawn_led`）、操作経路（`spawn_control`）、ブリッジUDP受信、このコマンド本体（Transportへの供給、1秒ごとの表示と記録、stdinのコマンド）。操作経路はLED送信と別スレッドなので、長いLED SysExの送信待ちがLiveへのNote Offを遅らせない。
- render：単調時計で `k / fps` ごとに評価する。遅れたframeは飛ばし、現在のslotのframeだけを評価する（追いつくための連続renderはしない）。結果は容量1のmailboxに置き、LED writerが取る前に次のframeが来たら上書きする（`mailbox_overwritten`）。
- 評価：`TransportSnapshot` の output が follow / hold ならその show_time を描画し、black なら黒。gain = transportのgain × brightness override。Blackout overrideは最終段で黒にし、操作経路は止めない。
- LED writer：差分の基準は最後に送信が成功したframe。送信失敗、再接続、ポート消失のあとは Programmer Mode 選択からやり直して全LEDを送る。Launchpad Xへは1 frame = 1 SysEx（変化したパッドだけ、最大64個）。
- 操作経路（`--control-in`）：Note / CC だけを転送し、Note On vel 0 は Note Off にする。鳴っているnoteを追跡し、同じnoteの重複Note Onと、鳴らしていないnoteのNote Offは送らない。入力queueのoverflowと送信失敗は fault（以後は転送しない。`reset` で解除）で、鳴っているnoteにNote Offを送る。入力ポートの消失・設定変更でもNote Offを送る。出力（`OMOI-IRO Control`）が消えた間のNote Offは、再接続時に送る。チャンネル全体への All Notes Off は送らない。
- 接続管理：ポートは完全名か `#index`（出力）で指定する。起動時にないポートは0.5秒ごとに再試行し、現れたら接続する。ポート一覧から消えたら切断扱いにする。同じ名前のポートが2つあるときは選ばずに fault にする（`#index` で指定し直す）。
- `--allow-device` で Launchpad らしい LED 出力を許可したときは、接続ごとに機器を確認してから LED を送る（Device Inquiry → Programmer Mode。応答は同じ機器の `--control-in` から受けるので、操作入力にも同じ LPX を指定する）。
- `--timecode-out <ポート>`：show の `sync.mtc_output.enabled` が true のとき、同じ Transport から MTC を送る（`mtc-out` と同じ生成器。M5 U02 で追加）。
- stdinのコマンド：`blackout on|off`、`brightness <0..1>`、`reload`（このスレッドで読込・検証し、通ったものだけ次のframe境界で差し替え。失敗時は今のshowのまま）、`reset`（操作経路のfault解除）、`quit`。
- 表示：`[時刻] frame <番号> rev <showのrevision> [dark=<理由>] gain <値> | LED <状態> [gen <接続世代>] lit <点灯数> | TRANSPORT ...`。`--view` で仮想LPXの8 x 8（上の行から、`#` 点灯）。
- 終了時のJSON（`--out` の `meta` / `end` にも入る）：render（frames、skipped_frames、jumps、show_swaps、jitter、render_time）、led（frames_sent、send_errors、mailbox_overwritten、full_refreshes、send_time、render_to_sent）、control（転送・抑止・fault・解放の件数、callback_to_send）、dropped（各queueの破棄件数）。時間は p50 / p95 / p99 / max（µs）。
- 合成源での確認（2026-10-28、`fake_bridge.py`、仮想LPX、debug build、16秒）：481 frames、skipped 0、jitter p95 0.54 ms・max 1.29 ms、render p95 47 µs、render→送信 p95 78 µs、seek / loop / 原点戻りを jumps 3 として検出、`reload` で revision 2 に差し替え。合成の結果でLive・実機の結果ではない。

## bench-render（NFR03）

```powershell
.\target\release\omoiiro-probe.exe bench-render --frames 9000
```

- 負荷：8レイヤー（over / add / max、opacity 0.5〜0.94）、2秒のclipを0.5秒ずつ重ね、両端にfade。Solid、Chase（x / y）、Pulse（sine / triangle、拍あり）。全64パッドが点灯する。1 frameの計測範囲は render threadの仕事（評価と量子化）。
- 2026-10-28、このWindows PC：release 9000 frames で p50 2 µs、p95 2 µs、max 22 µs。debug で p95 13 µs、max 106 µs。上限 5 ms（NFR03）に対して pass。`cargo test` にも同じ計測（debug、1800 frames、p95 ≤ 5 ms を検査）がある。

## transport（Runtimeの時刻・拍の統合、M3）

```powershell
.\target\debug\omoiiro-probe.exe transport --show examples\demo.show.json --clock-in "OMOI-IRO Clock"
.\target\debug\omoiiro-probe.exe transport --clock-in "OMOI-IRO Clock" --origin 01:00:00:00 --out artifacts\p0\win-m3-transport-01.jsonl
```

- 実装は `crates/omoiiro-runtime/src/sync/transport.rs`。時刻源はブリッジ（`show_time_us = song_ms * 1000 + input_offset_us`、`tc` は `origin + show_time`）。再生状態はブリッジの `is_playing` が正。拍源は show の `beat_source`（`--show` なしでは `--clock-in` / `--link`）の1つだけで、自動fallbackはしない。
- 表示：`TRANSPORT <playing|stopped|unknown>[ (stalled)] show_time= tc= output=<follow|hold|black>(<gain>) time=<ブリッジの健全性> beat=<拍>@<bpm> [<拍の健全性>]`。
- 同期断の規則（show の `sync.loss` / `stop_behavior`）：再生中で locked / holdover なら follow。lost と fault は、最後の正常サンプルから `blackout_after_ms` まで位置を保持（hold）し、その後は黒。正常な Stop は `hold` なら保持、`blackout` なら黒（復帰時のフェードなし）。同期断による黒から戻るときは `recovery_fade_ms` で gain を上げる。拍側は独立で、`holdover_ms` まで外挿、その後は保持、`blackout_after_ms` を過ぎると位相を無効にする（Pulse は透明）。
- 再接続（T03）：`Transport::reconnect_bridge / reconnect_clock / reconnect_link` で古い状態を捨て、接続世代を上げる。入力は到着時の単調時計で刻み、接続ごとの backend timestamp は比較しない。
- `--out` の記録：`bridge` / `clock` の入力と、1秒ごとの `source = transport`、`kind = state`。
- mtc-out は同じ `Transport` を経由して MTC を出す（`omoiiro_runtime::output::mtc`）。M3 で切り替えた後も、合成源の自己検証の数値と JSONL のレコードの形は変わらないことを確認した。

## render（headless描画、M2）

```powershell
.\target\debug\omoiiro-probe.exe render --show examples\demo.show.json --at 9000000 --beat 0.25
.\target\debug\omoiiro-probe.exe render --show examples\demo.show.json --at 9000000 --json
```

- `--at` はshow時刻（μs、曲頭 = 0、負も可）。`--beat` は四分音符単位の拍位置（Pulse用）。省略すると拍は不明として扱い、Pulseは透明になる。`--tempo` は表示用。
- 読込は `omoiiro-store`（Schema検証 → 意味検証）。不正なshowは JSON Pointer 付きのエラーで終了する。
- テキスト出力は上の行から下の行（y = 7 → 0）、各行は x = 0..7。JSON出力は `rows_top_down` と、LED index（`y * 8 + x`、左下原点）順の `leds_bottom_left`。`mtc_label` は `origin + show_time`。

## bridge-clock（共通時計、P1）

```powershell
.\target\debug\omoiiro-probe.exe bridge-clock --origin 01:00:00:00 --run-id win-p1-bridge-01 --out artifacts\p0\win-p1-bridge-01.jsonl
.\target\debug\omoiiro-probe.exe replay artifacts\p0\win-p1-bridge-01.jsonl
```

仕様は [docs/10-live-bridge.md](../../docs/10-live-bridge.md) の「アプリ側の受信と共通時計」。実装は `crates/omoiiro-runtime/src/sync/bridge.rs`（入出力なし、単体テスト付き）。

- 健全性：`unlocked` → `acquiring`（1サンプル、または不連続の直後）→ `locked`（連続2サンプルが整合）→ `holdover`（再生中に50 ms無着、補間を続ける）→ `lost`（250 ms無着、位置保持、1 sで消灯）。停止中は500 ms以内の到着だけを見る。`fault`：`errors` あり、`song_ms` / `is_playing` の欠落、未知の形式版、`seq` の大きな逆行。`seq` が1に戻った場合はスクリプトの再起動として扱う。小さな逆行（並び替え）は無視して数える。
- 不連続（抑制しない）：`discontinuity(playing)` は再生中に `|Δsong − Δ到着| > 20 ms`。`discontinuity(start|stop)` はPlay・Stopの瞬間に `[−20 ms, Δ到着 + 20 ms]` の外へ動いたもの。`is_playing` がtrueになってから50 ms以内の飛びも `start` に分類する（LiveはPlayを先に知らせ、開始位置への移動が数ms遅れて来るため）。`discontinuity(stopped)` は停止中に1フレーム（1/30 s）を超えて動いたもの。
- 停滞：`is_playing=true` なのに曲中時刻が300 ms進まなければ `stalled`。再生扱いにはしない（S07でLiveが固まった状態）。位置が変わらないサンプル対は不連続に数えない。30 ms以上空いた再生中のサンプル対で位置が同じなら、`stalled` の判定（300 ms）を待たずに補間をやめる（`advancing = false`）。その状態から動き出した最初の飛びは `discontinuity(start)` に分類する（Linkの開始待ちからの再開）。
- `meta stop`（スクリプトの切断）を受けたら、250 msのholdoverを待たずにすぐ `lost` にする。次のサンプル（再起動）で解除する。
- 表示：`BRIDGE <健全性> <transport> song=<曲中時刻> tempo= age= n= disc=<全不連続> (playing <再生中>, start <Play直後>) stalls= tc=<origin + song + offset の30 fpsラベル>`。共通時刻は `show_time = song_ms × 1000 + input_offset_us`（曲頭 = 0）、MTCラベルは `origin + show_time`（docs/10）。
- 記録：サンプルは `source = bridge`、`kind = sample`、decoderのイベントは `kind = event`。meta startに `bridge_origin` と `bridge_offset_ms`、meta endに最終状態。
- `replay` はブリッジのサンプルもdecoderに流し、イベントと最終状態を出す。形式版1（DJSCBridgeProbe の `smpte_ms`）も受け付ける。
- 実測記録のreplay（`tests/replay_live.rs`。記録の再解析で、新しい測定ではない）：`win-o12-bridge-03`（テンポ自動化）は再生中の不連続0、停止中に先頭へ戻った1件。`win-o12-bridge-01` は手動テンポ変更の2件（6256 → 5379 ms、11595 → 18068 ms）と、停止中に先頭へ戻った1件。`win-o12-bridge-02`（録音中はリスナーが止まる）は停滞0、faultなし。Playの直後に開始位置へ戻る動き（1378 → 5 ms、2212 → 6 ms）は `discontinuity(start)` の2件で、再生中の不連続は0。

## mtc-out（MTC出力、P1）

```powershell
# Liveの曲中時刻からMTCを生成して OMOI-IRO Timecode へ出す。--verify-in で同じバスを受け直して記録する
.\target\debug\omoiiro-probe.exe mtc-out --out-port "OMOI-IRO Timecode" --verify-in "OMOI-IRO Timecode" --origin 01:00:00:00 --run-id win-p1-mtcout-01 --record artifacts\p0\win-p1-mtcout-01.jsonl
.\target\debug\omoiiro-probe.exe mtc-out-report artifacts\p0\win-p1-mtcout-01.jsonl [--json]
```

仕様は docs/10-live-bridge.md の「MTC出力」。状態機械は `crates/omoiiro-runtime/src/sync/mtc_out.rs`（時刻を引数に取る純粋な実装、単体テスト付き）、送信スレッドは `crates/omoiiro-runtime/src/output/mtc.rs`（Runtime の `Transport` から駆動）、CLIは `apps/probe/src/mtc_out.rs`、集計は `crates/omoiiro-runtime/src/diagnostics/mtc_out_report.rs`。

- ブリッジが locked / holdover で再生中ならQuarter Frameを送る。停止中（`stalled` を含む）はQFを止め、停止位置のFull Frameを1回送る。停止中に位置が1フレームを超えて動けば、もう一度Full Frameを送る。lost / fault / unlocked、または再生中の acquiring では何も送らない。
- `is_playing` がtrueになったら、次のサンプル（位置の更新）を最大50 ms待ってから Full Frame(start) を1回出す（LiveはPlayを先に知らせ、開始位置への移動を数ms遅れて送るため、旧位置のFFを出さない）。再生中でも位置が動いていない間（`advancing = false`。Linkの開始待ちなど）はQFを出さずに保持し、動き出してから start を出す。ブリッジが `lost`（`meta stop` を含む）になればQFを止める。
- Play開始と復帰では、Full Frameの直後、次のフレーム境界からQFを再開する。seekやループでは、ブリッジが不連続のあと1サンプルの間 acquiring になるので、その間QFを止める（1サンプルの位置を外挿しない）。Full Frameの理由は、`start`（Play開始、または不連続のない復帰）、`jump`（再生中の不連続。seek、ループ、手動テンポ変更）、`resync`（1フレームを超える誤差）、`stop`、`hold`（停止中の初回または位置変更）。
- ラベルは各系列のpiece 0送出時のフレームで、系列ごとに2フレーム進む。送信スケジュールは独自の時間軸で進め、ブリッジの位置との誤差の10 %ずつを寄せる（ブリッジ側の±数msのゆらぎをラベルに出さない）。誤差が1フレームを超えた場合は `resync`（Full Frame → 再開）。
- 送信は専用スレッドで行う。粗いsleepのあと、残り2 ms以下はyield、0.2 ms以下はspinで待つ。
- 記録：送信は `source = mtc_out`、`kind = send`（`type` は `qf` / `ff`、`label`、QFでは `t_sched_us` と `late_us`、piece 0では `label_us` と `src_pos_us`、FFでは `reason`）。`--verify-in` の受信は `source = mtc`、`kind = midi`（monitorと同じ形式）。ブリッジのサンプルとイベントも同じファイルに入る。`--verify-in` は記録と集計のためだけに使い、アプリの時計には使わない。
- `mtc-out-report` の出力：QFの送信遅れ（送信 − 予定）と送信間隔、Full Frameの一覧、start / jump / resync のFull Frameから受信側がlockするまでの時間、piece 0 のラベルと送信時のブリッジ位置の差、受信MTCとブリッジ位置の差（両方 locked で再生中のサンプルだけ）。
- Liveなしの自己検証（合成源 `live-bridge/sim/fake_bridge.py` → mtc-out → loopMIDI Port 1 → `--verify-in`、2026-10-28）：QF 1143件、送信遅れは中央値0.07 / p95 0.14 / 最大0.22 ms、送信間隔は中央値8.33 / p95 8.38 ms、Full Frame後の受信lockは125〜158 ms（5回）、ラベルとブリッジ位置の差はp95 0.50 ms、受信MTCとブリッジ位置の差はp95 0.41 ms。最大値11999 msは、seek直後にブリッジがすでに新しい位置にあり、受信側がまだ古い位置でlockしていた1サンプル分。これは合成源での配管確認で、Liveでの結果ではない。

## lpx-check（Launchpad X、runbook C）

```powershell
# 1. LPXのポートを確認する（Windowsでは2系統。MIDI用を使う。DAW用ではない）
.\target\debug\omoiiro-probe.exe ports
# 2〜6. 完全一致名か #index で入出力を指定。30秒間パッドを受け付けてから消灯・Live Mode復帰
.\target\debug\omoiiro-probe.exe lpx-check --in "<LPXのMIDI入力名>" --out "<LPXのMIDI出力名>" --duration 30 --run-id win-s01-lpx --record artifacts\p0\win-s01-lpx.jsonl
# 同じ確認をCtrl+Cで終える（--durationなし）
.\target\debug\omoiiro-probe.exe lpx-check --in "<入力>" --out "<出力>" --run-id win-s01-lpx-ctrlc --record artifacts\p0\win-s01-lpx-ctrlc.jsonl
# 5. パッド入力を OMOI-IRO Control へも転送する（WinMMでは同じ入力ポートを2プロセスで開けないため、LPX入力の転送はこちらで行う）
.\target\debug\omoiiro-probe.exe lpx-check --in "<入力>" --out "<出力>" --forward "OMOI-IRO Control" --duration 30 --run-id win-s02-lpx-route --record artifacts\p0\win-s02-lpx-route.jsonl
```

| 手順 | 送信 | 期待する応答・表示 | 応答がないとき |
| --- | --- | --- | --- |
| 2 Device Inquiry | `F0 7E 7F 06 01 F7` | `F0 7E dd 06 02 00 20 29 03 01 00 00 v1 v2 v3 v4 F7`。`identity.kind = launchpad_x`、`version` にfirmware版 | エラーで終了。以降は何も送らない |
| 3 Mode照会 | `F0 00 20 29 02 0C 0E F7` | `F0 00 20 29 02 0C 0E mm F7`。`initial_mode`（0 = live、1 = programmer） | エラーで終了。以降は何も送らない |
| 4 Programmer Mode | `... 0E 01 F7`、続けてMode照会 | mode 1 の応答で `programmer_confirmed = true`、`programmer mode confirmed, pad 11 lit` | 消灯とLive Mode復帰を試みてエラー |
| 4 LED | `F0 00 20 29 02 0C 03 03 0B 0A 00 00 F7`（パッド11、RGB 10/0/0） | 左下パッドが暗い赤 | — |
| 5 入力 | なし | `press pad 11 value N` / `release pad 11`。上段・右列ボタンは `cc`。Note On velocity 0 はreleaseとして数える | — |
| 6 終了 | `... 03 03 0B 00 00 00 F7`、`... 0E 00 F7`、Mode照会 | `cleanup.ok = true`、`confirmed_mode = 0`、パッド消灯 | `WARNING: restore incomplete` を表示し、終了コード1 |
| 7 | なし | 終了時に表示される注意に従い、LiveのControl SurfaceにLaunchpad Xが割り当てられていないことを画面で確認する | — |

Device Inquiryのfamily code（アプリ 03 01、bootloader 03 11）はLaunchpad X Programmer's Referenceに基づく値で、実機では未確認。Novationの別familyが返った場合は既定で止まる。返ったバイト列を記録し、`--allow-unknown-family` を付けて続行するかを判断する。

LPXは押し続けるとaftertouch（An / Dn）を送ることがある。記録はするが `press` / `release` には数えず、転送もしない。

## route-check

```powershell
# LPX以外の任意入力（例：loopMIDI Port）から OMOI-IRO Control へNote/CCを転送する
.\target\debug\omoiiro-probe.exe route-check --in "loopMIDI Port" --out "OMOI-IRO Control" --duration 60 --run-id win-s02-route --record artifacts\p0\win-s02-route.jsonl
```

- 転送するのは Note Off（8n）、Note On（9n）、CC（Bn）だけ。Note On velocity 0 は同じチャンネルの `8n kk 00` に正規化する。その他は記録だけする。
- 記録：入力は `source = route_in`（`lpx-check --forward` では `lpx_in`）、`kind = midi`。転送は `source = control_out`、`kind = forward`、`payload` に `bytes`（送ったもの）、`from`（受けたもの）、`normalized`。
- 入力と出力が同じポートなら拒否する（ループ防止）。Launchpadらしい名前の出力も拒否する。
- Live側の `OMOI-IRO Control` の Remote ON 設定は設計担当が行う。

## 表示の読み方

```
[    6.070s] | MTC      30 locked    raw=01:00:04:26 pos=01:00:04.949 adv age=0ms qf=547 seq=68 drop=0 disc=1 ff=1 ff_disc=0 | CLK 120.07bpm playing  phase=0.92 beats=9.92 pos-ok age=12ms ticks=238 | q_drop=0
```

| 項目 | 意味 |
| --- | --- |
| `MTC 30` | Quarter Frameから読んだfps。`29.97df` は表示するが時刻換算しない（`fault [unsupported frame rate]`） |
| `locked` 等 | `unlocked` → `acquiring`（系列1組）→ `locked`（連続2組が2フレーム差）。無通信が50 msを超えると `holdover`（250 msまで補間）、超えると `lost`（補間上限で位置保持、1 sで `blackout`）。逆方向系列や不正ラベルは `fault` |
| `raw` | 最後に完成した系列のラベル。piece 0 が送られたフレームを表すので、現在位置より最大2フレーム遅れる |
| `pos` | 補間位置。`adv` が付いているときだけ時間とともに進む |
| `qf / seq / drop / disc / ff` | Quarter Frame数、完成系列数、破棄した系列数、QFラベルの不連続（欠落、Full Frameなしの再開）数、Full Frame数 |
| `ff_disc` | Full Frameで位置が飛んだ回数（O13）。Full Frame直前の表示位置（locked / holdover / lost の補間・保持位置、acquiring中は前のFull Frameの位置）との差が暫定2フレームを超えたもの。`disc` には含めない。eventsには `discontinuity(full_frame) <直前位置のラベル> -> <Full Frame> (delta ±N frames, from <直前の状態>)` と出す。Play直後のLiveMTCの `01:00:00:00` → 正位置のFull Frame対も抑制せずに数える |
| `CLK` | 直近24 tickからのBPM、transport、四分音符位相、拍数。`pos-?` はStartもSPPも未受信で位相が未確定 |
| `stalled` | transportがplayingのまま、tickが暫定500 ms来ていない（LiveのClock Sync OFFはStopを送らずtickだけ止める）。BPMと拍数は最後の値を表示し、`age` で経過を示す。tickが再開すれば Start/Continue なしで `playing` に戻す。eventsには `stalled (no tick for N ms)` / `resumed (ticks back after N ms)` |
| `q_drop` | callback→pipeline の有界queue（8192）で落とした件数 |

## 設計上の約束

- midir callbackでは単調時刻の採取、バイト列のコピー、`try_send` だけを行う。表示・ファイル書込・デコードは別スレッド。
- 接続ごとのbackend timestampは記録するだけで、時刻軸には共通の単調時計（`t_mono_us`）を使う。
- `locked` は連続する2系列の整合で判定する。Full Frameは位置更新であり再生ではない。
- 30 NDFを対象にし、他のfpsを30 fps扱いしない。
- 合成fixtureと `gen-mtc` の結果は、Liveや機器との接続成功を意味しない。
- `fixtures/live/` の記録のreplayは、記録済み入力に対するデコーダーの確認であり、新しいLive・機器測定ではない。`lpx-check` の手順テストは模擬デバイスによるもので、Launchpad Xの実機確認ではない。
- Linkへtempoやtransportを送らない。Link観測はcaptureだけを行う。

## 記録形式

1行1レコードのJSONL。`run_id`、`seq`、`t_mono_us`、`source`（`mtc` / `clock` / `sync` / `lpx_in` / `lpx_out` / `route_in` / `control_out` / `meta`）、`kind`（`midi` / `event` / `send` / `forward` / `start` / `end` / `synthetic`）、`payload`（`midi` では `bytes` の16進文字列と `backend_ts_us`）。生ログは `artifacts/p0/` に置き、設計文書へ埋め込まない。
