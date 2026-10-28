# live-bridge-probe

代替ブリッジの計測実験用（O12）。P1の本番向けスクリプトは [live-bridge](../../live-bridge/README.md) の DJSCBridge が後継で、このフォルダーは実験記録の再現用として残している。Live 12のMIDI Remote Script「DJSCBridgeProbe」が、テンポマップを反映した曲中時刻を読んでUDPで送る。読める周期とジッターを、`p0-probe bridge-listen` / `bridge-report` で数値化する。採用判断の材料であり、本番ブリッジではない。

スクリプトはLiveのtransport、tempo、その他の状態を変更しない（読むだけ）。

## 構成

| ファイル | 内容 |
| --- | --- |
| `DJSCBridgeProbe/__init__.py` | `create_instance` |
| `DJSCBridgeProbe/DJSCBridgeProbe.py` | `_Framework.ControlSurface` を継承。リスナー登録、約100 msの `update_display` タイマー、終了処理 |
| `DJSCBridgeProbe/probe_core.py` | レコード組み立て、SmpteTimeの読み取り、UDP送信とバッファ付きJSONL。Liveに依存しない |
| `tests/test_probe_core.py` | 偽のSongでの単体テスト（Liveでの確認ではない） |

## レコード

1レコード = 1データグラム（JSON、UTF-8）。送信先は `127.0.0.1:9007`。同じ内容を `%USERPROFILE%\DJSCBridgeProbe\bridge-<日時>.jsonl` へも追記する（64件ごとか1秒ごとに `update_display` からflush）。

```json
{"v":1,"seq":42,"kind":"listener","event":"song_time","perf_ns":123456789012,
 "smpte_ms_fields":{"str":"...","hours":0,"minutes":0,"seconds":6,"frames":250},"smpte_ms":6250,
 "smpte30":{"str":"...","hours":0,"minutes":0,"seconds":6,"frames":7},
 "beats":12.5,"tempo":120.0,"is_playing":true}
```

- `kind`：`listener`（`event` = `song_time` / `is_playing` / `tempo`）、`timer`（`event` = `tick`、約100 msの `update_display`）、`meta`（`start` / `stop`）。
- `perf_ns`：Live内の `time.perf_counter_ns()`。p0-probeの単調時計とは別の時計。
- `smpte_ms`：`song.get_current_smpte_song_time(Live.Song.TimeFormat.ms_time)` の `hours` / `minutes` / `seconds` / `frames` から、frames = ミリ秒として計算した値。`smpte30` は `TimeFormat.smpte_30` の値。
- `beats` = `song.current_song_time`、`tempo` = `song.tempo`、`is_playing` = `song.is_playing`。
- 読み取りに失敗した項目は `errors` に入る。`str` には `str(SmpteTime)` を常に残すので、属性名が想定と違っても中身を確認できる。

### Live APIの名前（確認状況）

| 名前 | 根拠 | 状態 |
| --- | --- | --- |
| `Song.get_current_smpte_song_time`、`Live.Song.TimeFormat`、`TimeFormat.smpte_25` | 付属スクリプト MackieControl/TimeDisplay、MiniLab_3/transport のバイトコード内の文字列 | 存在を確認 |
| `TimeFormat.ms_time`、`TimeFormat.smpte_30` | Live Object Modelの非公式資料 | Live 12.4.7で `errors` なしに読めた（停止中、設計担当の実機確認 2026-10-28） |
| `SmpteTime.hours` / `minutes` / `seconds` / `frames` | 同上。`str()` は `000:00:00:000` 形式 | 同上（停止中のため値は0） |
| `ms_time` で `frames` がミリ秒になること | 同上 | 確認済み。再生中の記録（artifacts/p0/win-o12-bridge-01）で、到着時刻に対する回帰の傾きが0.999971（設計担当の実機確認 2026-10-28） |
| `add_current_song_time_listener`、`add_is_playing_listener`、`add_tempo_listener`、`current_song_time`、`tempo`、`is_playing` | Live Object Modelの一般的なAPI | 動作を確認。ただしArrangement録音中は `current_song_time` リスナーが止まり、タイマー（約100 ms）分しか届かなかった（win-o12-bridge-02） |

初回の実機確認では、`errors` が出ていないことと、`smpte_ms_fields.str` と各フィールドが一致していることを最初に見る。

## インストール（Windows、Live 12.4.7）

1. Liveを終了する。
2. `DJSCBridgeProbe` フォルダーを丸ごと、LiveのUser Library内の `Remote Scripts\DJSCBridgeProbe\` へコピーする（`__init__.py` がこのフォルダー直下に来るようにする）。User Libraryの場所は Preferences > Library で確認する。このPCでは `%USERPROFILE%\OneDrive\ドキュメント\Ableton\User Library\Remote Scripts\DJSCBridgeProbe\`。`%APPDATA%\Ableton\Live 12.4.7\Preferences\User Remote Scripts\` はインスタントマッピング（UserConfiguration.txt）用で、ここに置いてもControl Surface一覧には出なかった（2026-10-28、設計担当の実機確認）。
3. Liveを起動し、Preferences > Link, Tempo & MIDI の Control Surface で空いている行に `DJSCBridgeProbe` を選ぶ。Input / Output は `None`。
4. 先に `p0-probe bridge-listen` を起動しておく（スクリプトは送るだけで、受け手がいなくてもエラーにしない）。
5. 外す場合は、Control Surfaceを `None` に戻し、フォルダーを削除する。

エラーはLiveの `Log.txt`（`%APPDATA%\Ableton\Live 12.4.7\Preferences\Log.txt`）に `DJSCBridgeProbe error in ...` として出る。正常時は何も書かない。

## 計測

```powershell
# リポジトリーのルートで実行する（M1以降、p0-probe は apps/probe へ移動）
# MTC / Clockと同時に記録する（同じ単調時計）。Ctrl+Cか--durationで終了
.\target\debug\p0-probe.exe bridge-listen --mtc-in "DJSC Timecode" --clock-in "DJSC Clock" --run-id win-o12-bridge-01 --out artifacts\p0\win-o12-bridge-01.jsonl
# 解析
.\target\debug\p0-probe.exe bridge-report artifacts\p0\win-o12-bridge-01.jsonl
.\target\debug\p0-probe.exe bridge-report artifacts\p0\win-o12-bridge-01.jsonl --json
```

`bridge-report` の出力は [apps/probe README](../../apps/probe/README.md) の bridge-report 節を参照。

## テスト

```powershell
cd tools\live-bridge-probe
python -m unittest discover -s tests
```

Python 3.11で5件pass（2026-10-28）。偽のSongとローカルUDPを使うテストで、Live上での動作確認ではない。
