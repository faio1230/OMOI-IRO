# live-bridge

P1試作。Live 12のMIDI Remote Script「OmoiIroBridge」が、曲中時刻（テンポマップ反映）、拍、テンポ、再生状態、拍子、ループをUDPで送る。アプリはこれを共通時計にする。仕様は [docs/10-live-bridge.md](../docs/10-live-bridge.md)。受信と共通時計は `omoiiro-probe bridge-clock`、MTC出力は `omoiiro-probe mtc-out`（[apps/probe README](../apps/probe/README.md)）。

合成の送信器 `sim/fake_bridge.py` は既定で13秒の場面（再生、seek、ループ、停止、無音）を送る。`--steady 秒 [--start-ms ms]` は指定した時間だけ一定に再生する（長時間の試験用）。

P0実験の [live-bridge-probe](../tools/live-bridge-probe/README.md)（DJSCBridgeProbe）が前身で、OmoiIroBridge がその後継。2026-10-28（M5冒頭）に `DJSCBridge` から改名した。B4 の実測（`win-p1-c*`、`win-s07-sss-*`）は旧名 DJSCBridge で行った。User Library に旧 `DJSCBridge` を入れている場合は、Live の Control Surface を `None` に戻してから `OmoiIroBridge` を入れて選び直す（両方とも9007へ送るため、同時に選ばない）。

スクリプトはLiveのtransport、tempo、ループ、選択状態を変更しない（読むだけ）。

## 構成

| ファイル | 内容 |
| --- | --- |
| `OmoiIroBridge/__init__.py` | `create_instance` |
| `OmoiIroBridge/OmoiIroBridge.py` | `_Framework.ControlSurface` を継承。リスナー（`current_song_time`、`is_playing`、`tempo`）と約100 msの `update_display` タイマー。`DEBUG_LOG`（既定 `False`）、`HOST`、`PORT` |
| `OmoiIroBridge/bridge_core.py` | レコード組み立て、512 byte上限、UDP送信、デバッグ用JSONL。Liveに依存しない |
| `tests/test_bridge_core.py` | 偽のSongによる単体テスト（Liveでの確認ではない） |
| `sim/fake_bridge.py` | Liveなしの確認用の合成送信器（play、seek、loop、stop、0.6 sの無音、停止中の移動）。合成源であり、これで得た結果はLiveの結果ではない |

## レコード（形式版 v = 2）

1レコード = 1データグラム（JSON、UTF-8、512 byte以下。超える場合は送らずに数える）。既定の送信先は `127.0.0.1:9007`。

```json
{"v":2,"seq":120,"kind":"listener","event":"song_time","perf_ns":691241404559000,
 "song_ms":12345,"beats":24.69,"tempo":120.0,"is_playing":true,
 "sig":[4,4],"loop":{"on":false,"start_beats":0.0,"length_beats":16.0},"errors":[]}
```

| 項目 | 内容 |
| --- | --- |
| `v` | 形式版（2） |
| `seq` | スクリプト起動からの連番。再起動で1に戻る（受信側は再起動として扱う） |
| `kind` / `event` | `listener`（`song_time` / `is_playing` / `tempo`）、`timer`（`tick`）、`meta`（`start` / `stop`） |
| `perf_ns` | Live内の `time.perf_counter_ns()`。受信側の時計とは比較しない |
| `song_ms` | `Song.get_current_smpte_song_time(Live.Song.TimeFormat.ms_time)` の hours / minutes / seconds / frames（frames = ms）から計算 |
| `beats` / `tempo` / `is_playing` | `current_song_time`、`tempo`、`is_playing` |
| `sig` | `[signature_numerator, signature_denominator]` |
| `loop` | `{on: loop, start_beats: loop_start, length_beats: loop_length}` |
| `errors` | 読み取りに失敗した項目名の配列。その項目は `null`。受信側は `errors` が空でなければ `fault` にする |

## Live APIの名前（版ごとの確認状況）

| 名前 | Live 12.4.7（Windows） | 根拠 |
| --- | --- | --- |
| `Song.get_current_smpte_song_time`、`Live.Song.TimeFormat.ms_time`、`SmpteTime.hours/minutes/seconds/frames` | 確認済み。ms_time の frames はミリ秒 | DJSCBridgeProbe の実機記録（win-o12-bridge-01..03、2026-10-28） |
| `add_current_song_time_listener` / `add_is_playing_listener` / `add_tempo_listener`、`current_song_time`、`tempo`、`is_playing` | 確認済み。Arrangement録音中は `current_song_time` リスナーが止まる | 同上 |
| `signature_numerator` / `signature_denominator`、`loop` / `loop_start` / `loop_length` | 未確認（OmoiIroBridgeで追加した項目） | Live Object Model |
| Mac（`~/Music/Ableton/User Library/Remote Scripts/`） | 未確認 | Mac枠M3 |

## インストール（Windows、Live 12.4.7）

1. Liveを終了する。
2. `OmoiIroBridge` フォルダーを丸ごと、User Library の `Remote Scripts\OmoiIroBridge\` へコピーする（`__init__.py` がこのフォルダー直下に来るようにする）。User Libraryの場所は Preferences > Library で確認する。このPCでは `%USERPROFILE%\OneDrive\ドキュメント\Ableton\User Library\Remote Scripts\OmoiIroBridge\`。`%APPDATA%\...\Preferences\User Remote Scripts\` はインスタントマッピング用で、一覧には出ない。
3. Liveを起動し、Preferences > Link, Tempo & MIDI の Control Surface に `OmoiIroBridge` を選ぶ。Input / Output は `None`。DJSCBridgeProbe を割り当てている場合は、その行を `None` に戻す（両方とも9007へ送るため）。
4. エラーはLiveの `Log.txt` に `OmoiIroBridge error in ...` として出る。正常時は何も書かない。
5. デバッグ用のローカル記録が必要な場合だけ、`OmoiIroBridge.py` の `DEBUG_LOG = True` にする（`%USERPROFILE%\OmoiIroBridge\bridge-<日時>.jsonl`）。

## テスト

```powershell
cd live-bridge
python -m unittest discover -s tests
```

Python 3.11で5件pass（2026-10-28）。Liveなしの通し確認には `sim/fake_bridge.py` を使う（合成源）。

```powershell
# 別ターミナルで受信側を起動してから
python sim\fake_bridge.py --port 9007
```
