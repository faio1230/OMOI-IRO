# 本番前日の計画（Mac）— 2026-10-28 → 本番 2026-10-29

利用者の回答（2026-10-28夕方、A19）で条件が確定した。本番機はMac（Apple Silicon）。本番で使うのは Launchpad X の LED、Launchpad X の操作の Live への転送、MIDI Clock による拍同期の3つ。MTC 出力は使わない。ショーデータは利用者が今夜 GUI で作る。Launchpad X は今日の深夜に接続し、その後1〜2時間立ち会える。

この時点で Mac ではビルドも試験もしていない。したがって今日の主役は Mac で、Windows は「利用者の手が要らない確認」と「放置できる連続試験」に回す。

## 1. 今日の順序

| 順 | 作業 | 誰が | 利用者の手 | 目安 |
| --- | --- | --- | --- | --- |
| 1 | Mac への経路を開く（Tailscale、リモートログイン、公開鍵）と開発ツールの導入 | 利用者 | 要 | 15〜30分 |
| 2 | コード転送、`cargo test`、probe と GUI のビルド、`omoiiro-probe ports` | Claude（SSH） | 不要 | 30〜60分（初回ビルド） |
| 3 | Live（Mac）の準備：IAC、OmoiIroBridge、Control Surface、MIDI 設定 | 利用者（手順は §3） | 要 | 15分 |
| 4 | Mac + Live の追従確認（C1、C2、C4、C6 の短縮版） | 利用者が Live を操作、Claude が記録 | 要 | 20分 |
| 5 | ショーデータの作成 | 利用者（GUI） | 要 | 任意の時間 |
| 6 | Windows で本番アプリの C1〜C7（アプリ論理の確認） | 実装担当（Opus） | 不要 | 並行 |
| 7 | Mac + LPX：S01、S02、C8 の短縮版、本番ショーで30分の連続再生 | 利用者 + Claude | 要（深夜） | 60〜90分 |
| 8 | 連続試験（放置） | Windows で実行 | 不要 | 夜間 |

5 は 2〜4 と並行できる。show ファイルにポート名は入らない（端末設定が持つ）ので、Windows の GUI で作って Mac へ持っていっても、Mac の GUI で作っても同じ。Live の Set の時刻と合わせやすい方で作る。

## 2. Mac への経路（利用者）

Windows 側で専用の鍵 `~/.ssh/omoiiro_mac` を作成済み（2026-10-28）。Mac は Tailscale 上の `<mac-host>`（<tailscale-ip>）。

1. Mac で Tailscale を起動してログインする。Windows 側の `tailscale status` で `<mac-host>` が online になれば通る。
2. システム設定 → 一般 → 共有 → リモートログイン を ON にする（フルディスクアクセスは不要）。
3. Mac のターミナルで公開鍵を登録する（公開鍵の文字列は Windows 側から渡す）。

```sh
mkdir -p ~/.ssh && chmod 700 ~/.ssh
echo 'ssh-ed25519 AAAA… omoiiro-design@desktop-vt6vilj' >> ~/.ssh/authorized_keys
chmod 600 ~/.ssh/authorized_keys
```

4. 開発ツール。Link は使わないので cmake と libclang は不要。

```sh
xcode-select --install          # 済みなら「already installed」
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Rust（aarch64-apple-darwin）
brew install node               # Node 22 以上
```

5. Mac のユーザー名と、Live 12 の版（Live → About）を Windows 側に伝える。

Windows 側の接続と転送（Claude が実行）。リポジトリーは git 管理ではないので tar で送る。

```sh
ssh -i ~/.ssh/omoiiro_mac <user>@<mac-host> 'uname -m; sw_vers; xcode-select -p; cargo --version; node --version'
cd <repo>
tar czf - --exclude=target --exclude=node_modules --exclude=dist --exclude=artifacts . | ssh -i ~/.ssh/omoiiro_mac <user>@<mac-host> 'mkdir -p ~/OMOI-IRO && tar xzf - -C ~/OMOI-IRO'
```

## 3. Mac のビルド（Claude、M0）

```sh
cd ~/OMOI-IRO
cargo test --workspace                       # Link なし
cargo build --release -p omoiiro-probe
./target/release/omoiiro-probe ports         # IAC と LPX の正確な名前を控える
cd apps/desktop && npm ci && npx tauri build # .app は target/release/bundle/macos/
```

- `tauri.conf.json` の `bundle.targets` は Windows 用の `nsis` だけなので、Mac では `app` と `dmg` を足す（実装担当）。bundle が失敗しても `npx tauri build --no-bundle` で `target/release/omoiiro` がそのまま起動できる。
- 署名と公証はしない。自分でビルドしたバイナリーなので Gatekeeper の警告は出ない想定。出たら「このまま開く」。
- 端末設定の保存先は Tauri の `app_config_dir`（Mac は `~/Library/Application Support/jp.faio.omoiiro/terminal-settings.json`）。
- CoreMIDI の名前は装置名 + ポート名になる。IAC は日本語環境だと「IACドライバ OMOI-IRO Clock」のような表記になりうる。`omoiiro-probe ports` の表示をそのまま端末設定に入れる（`#番号` も可）。

GUI が Mac で動かない場合の逃げ道：CLI で同じ Runtime を動かせる。

```sh
./target/release/omoiiro-probe follow --show <本番.show.json> \
  --clock-in "<IAC … OMOI-IRO Clock>" \
  --control-in "<Launchpad X LPX MIDI Out>" --control-out "<IAC … OMOI-IRO Control>" \
  --led-out "<Launchpad X LPX MIDI In>" --allow-device \
  --out ~/OMOI-IRO/artifacts/m6/mac-follow.jsonl
```

LPX のポートは2系統ある。`LPX MIDI` を使い、`LPX DAW` は使わない。

## 4. Live（Mac）の準備（利用者）

1. Audio MIDI 設定 → ウインドウ → MIDI スタジオ → IAC ドライバ をダブルクリック → 「装置はオンライン」にチェック → ポートに `OMOI-IRO Control` と `OMOI-IRO Clock` を追加する（`OMOI-IRO Timecode` は今回使わない。作っても害はない）。
2. `live-bridge/OmoiIroBridge` フォルダーを `~/Music/Ableton/User Library/Remote Scripts/OmoiIroBridge/` に置く（User Library の場所は Live の 環境設定 → Library で確認。転送後は Claude が SSH で置ける）。Live を再起動する。
3. Live 環境設定 → Link, Tempo & MIDI：
   - Control Surface のどこかを `OmoiIroBridge`（Input / Output は None）。
   - Launchpad X を Control Surface に割り当てない（アプリと二重になる）。既に自動で入っていたら None にする。
   - MIDI Ports：Output `IAC … OMOI-IRO Clock` の Sync を ON、他の出力の Sync は OFF。Input `IAC … OMOI-IRO Control` の Track と Remote を ON。Launchpad X の Input は Track / Remote とも OFF（操作はアプリ経由で Control から入る）。
   - Link は OFF。Start Stop Sync は使わない。
4. 音声はいつもの構成でよい。再生しながら Clock が出ているかは Claude が `omoiiro-probe monitor --clock-in` で確認する。

## 5. Mac + Live の確認（M2、20分）

Set は本番の Set（テンポオートメーション入り）。`omoiiro-probe monitor --clock-in "<Clock>" --out artifacts/m6/mac-m2-mon.jsonl` を回しながら、アプリ（または CLI follow、LED は `virtual`）を開始する。

| ケース | 操作 | 期待 |
| --- | --- | --- |
| C1 | Play → Stop → 再開 | 時刻源 locked → hold → follow。拍源は Clock を受けて OK |
| C2 | 再生中に別の位置へ seek | 位置の飛びとして扱い、すぐ再 lock |
| C4 | テンポが変わる区間を通す | 不連続 0、拍源のテンポ表示が追従 |
| C6 | Control Surface を一度 None に戻して戻す | lost → holdover → 消灯 → 再 lock |

## 6. Mac + LPX（M1 / M3、接続後60〜90分）

1. `omoiiro-probe ports` で LPX の名前を控え、端末設定に control_in = `LPX MIDI Out` 側、led_out = `LPX MIDI In` 側、`devices.allow_launchpad_led` = true を入れる。
2. S01：アプリを開始 → LED タイルが identifying → configuring → 接続 になる。firmware 表記を記録。パッドを押すと操作経路のタイルが数え、Live の MIDI 入力表示が点く。停止で全消灯と Live Mode 復帰。
3. S02：Live の MIDI Map でパラメーターを1つ割り当て、パッドで動く。Blackout 中も操作が届く。
4. C8：追従中に USB を抜く → 再接続待ち、Live に鳴りっぱなしの note が残らない。挿し直す → 自動で Ready に戻り全 LED を送り直す。
5. 本番ショーで30分の連続再生（利用者がショーを直しながらでよい）。終了時に診断（飛ばした frame、send_errors、fault、保留中 Note Off）を記録する。

合否は m6-runbook と同じ。明るさの体感が強すぎれば端末設定の `safety.max_brightness` を下げる（show は変えない）。点滅の上限は既定 3 Hz。意図して速い点滅を使うなら `max_flash_hz` を上げる。

## 7. ショーデータ（利用者、今夜）

- 新規ショーを作り、同期は時刻源 live_bridge、拍源 midi_clock、MTC 出力 off、起点は既定（01:00:00:00）のまま、render は 30 fps。
- Live の Set の曲中時刻に合わせてクリップを置く。アプリのタイムコードは Live の曲中時刻（Arrangement の時間）に起点を足したもの。
- 保存先は Mac から読める場所（iCloud / OneDrive）か、Claude が scp で Mac へ置く。拡張子は `.show.json`。
- プレビューで色と動きを確認する。実機の色は RGB 7 bit に量子化されるので、淡い色は潰れやすい。

## 7b. 当日までに Mac で入れ替えるもの（2026-10-28 夜の時点）

- アプリ：GitHub の main の最新受け入れコミットからビルドした `OMOI-IRO.app`（20 種の演出、発火レイヤー、キーフレーム、複合素材、録音、音量メーター入り）。
- Live の Remote Script：`live-bridge/OmoiIroBridge` の新版（音量メーター v=3 を送る）。**アプリと同時に入れ替える**。古いアプリ（17:35 版）は v=3 を受けると fault になる。新しいアプリは古いスクリプトでも動く（音量は途絶扱い）。
- 端末設定、起動順、権限ダイアログに変更はない。
- 音に反応させたいトラックは Live で名前の頭に `[LED]` を付ける（最大4本）。

## 8. 本番当日の起動手順

1. Launchpad X を USB で接続する（Live を起動する前）。
2. Live を起動して本番の Set を開く。環境設定で OmoiIroBridge が Control Surface にあり、IAC の Clock Sync が ON のまま。
3. OMOI-IRO を起動 → 接続設定で5つの役割が緑（control_in / control_out / clock_in / led_out、timecode_out は空のまま）。
4. 本番ショーを開いて「開始」。演奏タブのタイルが、時刻源 待機（Live 停止中）、拍源 待機、LED 接続、操作経路 OK になる。
5. Live を Play → LED が追従し、タイムコードが進む。**順序は必ず「アプリを開始」→「Live を Play」**。MIDI Clock は Play の合図（Start / 位置情報）がないと拍の頭が分からないので、逆順だと Pulse 系が透明のままになる（ブリッジの拍で原点を補う修正が入れば緩和されるが、順序は守る）。
6. 本番中の対応：
   - 明るさを即座に落とす：BLACKOUT（LED だけ消える。操作の転送は続く）。
   - LED が止まった：「全ポートを再接続」。直らなければ LPX の USB を抜き差し。
   - アプリが落ちた：アプリだけ再起動して「開始」。Live はそのまま。再 lock は1秒以内。Pulse 系が戻らなければ、曲の切れ目で Live を Stop → Play（Clock の原点が取り直される）。
   - 全部やめる：アプリを終了する（終了時に全消灯と Live Mode 復帰）。Live と音は影響を受けない。
7. 終了後にアプリを終了してから Live を終了する。

## 9. 記録と未実施の扱い

- Mac の記録は `artifacts/m6/mac-*.jsonl` と画面。結果は本番後に p0-results / m6 の結果として整理する。
- 未実施のまま残すもの：Mac での2時間連続（放置不可）、S05 の 60 fps 比較、G1.2。「本番で動いた」は連続安定性の証明として扱わない。
