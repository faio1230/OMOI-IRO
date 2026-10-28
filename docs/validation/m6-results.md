# M6 結果（実機）

2026-10-28から記録。判定基準は [m6-runbook.md](m6-runbook.md)。Macと本番の計画は [m6-mac-showday.md](m6-mac-showday.md)。

## Windows：本番アプリ（release）でのLive追従 C1〜C7（2026-10-28）

環境：Windows 11、Live 12.4.7、OmoiIroBridge、loopMIDI（Port 2/3/4 = `OMOI-IRO Control / Timecode / Clock`）、`omoiiro.exe` release（16:28ビルド、214件pass）、LEDは仮想、操作経路は未使用。受け側は `omoiiro-probe monitor`（MTC = Timecode、Clock）。アプリの状態は `apps/desktop/scripts/m6-follow.ps1` が画面の文字を0.6秒ごとに記録。実施は実装担当。

残差の計り方：ブリッジのUDPは1宛先でアプリが受けるため、監視側では受けられない。代わりに、Liveの曲中時刻が実時間と1:1で進むことを使い、連続区間ごとの受信QF時刻と直線のずれを残差とした（`tools/m6/mtc_residual.py`）。B4（P0）のブリッジ位置との差とは別の指標。

| ケース | 結果 | 判定 |
| --- | --- | --- |
| C1 再生・停止・再開 | startとcontinueのたびにFF、再lock 152 ms / 129 ms。停止中はhold | 合格 |
| C2 seek | 停止中の移動でFF。再生中の飛びはFF(jump)後 129 ms / 158 ms で再lock | 合格 |
| C3 4小節ループ | 4回の折返しともFF後 142〜159 ms で再lock | 合格 |
| C4 テンポ自動化（60 s、90〜140 bpm） | 不連続 0、resync 0、残差 p95 1.09 ms / 最大 2.42 ms、速度比 0.99998 | 合格 |
| C5 ビュー切替4回 | 不連続 0、残差 p95 1.42 ms。SetにSessionクリップがなくクリップ起動は未実施 | ビュー切替のみ合格 |
| C6 ブリッジ停止→復帰（再生中） | lost → 0.6 s以内にhold → 1.2 s以内に消灯。戻すとlocked/follow、MTCはFF後 158 ms で再lock。拍源は終始locked。回復フェードは記録間隔（0.6 s）では未確認 | 合格 |
| C7 Clock Sync OFF→ON（再生中） | 時刻・MTC・LEDに影響なし（残差 p95 0.78 ms）。拍源 locked → lost → 再開でlocked。holdoverとPulse透明化は未確認 | 合格（2点未確認） |

アプリの累計（約6分）：frame 10,978、飛ばしたframe 0、LED送信失敗 0、MTC送信 16,900 / 失敗 0、破棄 0。位置の飛び8件はすべて停止中の移動。

気づいた点と判断：
1. Clockを止めて再開すると拍の数が途中から続きになる（SPPが来ない）。本番中にSyncを切り替えなければ起きないので修正しない。
2. 再生開始直後の約1秒、bpm表示が3.5や5.9になる（推定の立ち上がり）。表示だけなので修正しない。
3. アプリのウィンドウを前面に出したままだと自動操作によるLive操作が止まる（自動操作の都合）。

記録：`artifacts/m6/win-m6-c{1..7}-mon.jsonl`、`win-m6-c6-mon-try1.jsonl`（初回の操作ミス）、`win-m6-app-{01,02,03}.jsonl`、`m6-live.show.json`、`win-m6-prep-02.jsonl`、`win-m6-prep-live-midi-prefs.jpg`。

## Windows：連続試験（NFR07、2026-10-28、合格）

条件：Windows、release `omoiiro.exe`（16:28版）、OmoiIroBridge、fakeなし。Liveは4小節（5〜9小節、テンポ自動化入り）のループ再生。LEDは仮想、操作経路は未使用、MTC出力あり。16:37:56〜18:37:57の7,200 s、途中操作なし。実施は実装担当。

| 項目 | 基準 | 結果 | 判定 |
| --- | --- | --- | --- |
| メモリー（子孫込み7プロセス） | 30分時点から+10%以内かつ+50 MB以内、単調増加なし | 30分 506.9 MB → 120分 518.0 MB（+11.1 MB、+2.2%）。最大 524.2 MB（115分）。増減を繰り返し単調増加ではない。`omoiiro.exe` 単体 41.9→42.2 MB | 合格 |
| 飛ばしたframe | 0.1%以下 | 215,988 frame中 0 | 合格 |
| send_errors | 0 | LED送信 215,997 / 失敗 0、MTC送信 860,646 / 失敗 0 | 合格 |
| fault | 0 | 時刻源は終了までlocked / follow、拍源locked、安全の警告 0、破棄 0 | 合格 |
| resync | 0 | 受信側MTC：不連続 0、rate_changes 0、dropped 0。FFによる飛び981件はループ折返し（7200 ÷ 7.33 ≒ 982）。生成側のresync件数は画面に出ないため未取得 | 受信側で合格 |
| 残差p95の推移 | 開始時と2時間後で変わらない | 0〜5分 1.23 ms → 115〜120分 1.27 ms（24区間の範囲 1.10〜1.39 ms、最大 11.12 ms）。蓄積なし | 合格 |

残差はMTC受信時刻と実時間直線とのずれ（C4と同じ測り方）。記録：`artifacts/m6/win-m6-soak-app.jsonl`（5分ごと）、`win-m6-soak-mon.jsonl`（122 MB、gitignore）。未確認：Macでの連続試験（放置不可）、実機LPXへの出力（今回は仮想）。ループが手順書の8〜16小節より短い4小節だった点は記録に残す。

## Windows：演出拡張後の回帰と計測（2026-10-28 夜、実装担当）

| 項目 | 結果 |
| --- | --- |
| Q01回帰（Schema 0.1.3、20種の演出入り release） | headless 23 / 23、GUI 6 / 6（G1.2 も無人で通過）。`apps/desktop/docs/q01-report-2.md` |
| 全showの一括検査 `omoiiro-probe check-shows` | fixtures 14件 + examples 1件、すべて読込・正規形保存・再読込一致・3時点の描画に合格 |
| 重いshowの10分soak（粒子2層、frames、text、recording駆動の熱・余熱・星座、meter glow、fake_bridge `--meter`、仮想LED） | 本体 43.4 → 44.3 MB（横ばい）、子孫込み 493.3 → 524.5 MB（+6.3%、516〜529で頭打ち）。合格。PadLogと履歴は増え続けない |
| Live 12.4.7 の音量メーター（v3ブリッジ、実機） | 停止中 約10 Hz、再生中 24.6 Hz（間隔 p50 33.5 ms / p95 66.3 ms / 最大 82.8 ms）。曲中時刻のリスナーは同時に 60.0 Hz で影響なし。`[LED] Kick`（Core LibraryのEDM Kick）の値は平滑化済みで 0.75〜0.92。通し（Live → アプリ、fixture 59）：時刻源は終始locked、glowで64点灯、orb区間52点灯 |

Liveのメーターは平滑化済みの値で、拍の立ち上がりは1〜2フレーム遅れる。キックに反応させるには `floor` で下を切って伸長する（0.1.3の後半でfloorの意味を「引いて伸長」に変更）。

## Mac：ビルド、Live追従、Launchpad X（2026-10-28）

環境：MacBook Air（Apple Silicon、arm64）、macOS 14.8.2、Xcode、cargo 1.99.1、Node 22.23.4（`~/node` に公式tarballを展開、PATHのみ）、Live 12 Suite、User Library `~/Music/Ableton/User Library`。設計担当がWindowsからSSH（Tailscale）で操作。コードはtarで転送（`~/OMOI-IRO`）、フロントエンドのdistはWindowsで作って同梱。空き容量 9.4 GB → ビルド後 8.0 GB（テストはMacで回していない）。

| 項目 | 結果 |
| --- | --- |
| M0 ビルド | `cargo build --release -p omoiiro-probe` 54 s、`-p omoiiro-desktop` 2 m 02 s。いずれも成功 |
| ポート列挙 | `midir/CoreMIDI`。LPXは `Launchpad X LPX MIDI Out`（入力）/ `Launchpad X LPX MIDI In`（出力）と `LPX DAW`。IACは利用者の設定待ち |
| S01（CLI `lpx-check`） | Device Inquiry応答 `00 20 29 03 01 00 00 00 02 08 09`（family 03 01、firmware 2.8.9相当）。Live Mode → Programmer Mode切替をmode queryで確認、pad 11点灯、押下・解放97件（複数同時押しを含む、ベロシティあり）。記録 `artifacts/m6/mac-s01-lpx.jsonl`（Mac側）。Live Mode復帰は終了ログで確認予定 |
| 補足 | LPXは「LAUNCHPAD X」という164 KBのFATボリュームとしてもマウントされる。機器確認は通常モードとして通過 |
| `.app` ビルド | Node 22.23.4を導入後 `npm ci && npx tauri build`：`OMOI-IRO.app`（18.93 MiB）と `OMOI-IRO_0.1.0_aarch64.dmg`（5.65 MiB）。成果物は `~/OMOI-IRO/dist-mac/` に退避し、空き容量のため `cargo clean`（再ビルドは probe 1分、アプリ 2分、.app 約5分） |
| IAC | 利用者が作成。CoreMIDI名は `IACドライバ OMOI-IRO Control` / `IACドライバ OMOI-IRO Clock`（日本語環境、装置名つき）。端末設定 `~/Library/Application Support/jp.faio.omoiiro/terminal-settings.json` に control_in = `Launchpad X LPX MIDI Out`、led_out = `Launchpad X LPX MIDI In`、control_out / clock_in = IAC、timecode_out なし、allow_launchpad_led true で配置 |
| Runtime経路の確認（CLI follow、20 s、Liveなし） | LED：Identifying → Ready（generation 3）、598 frame（30 fps）、送信失敗 0、full refresh 1、render→送信 p95 86 µs。操作経路：IAC Control出力とLPX入力が接続。時刻源はunlocked（ブリッジなし）のため出力はblack。記録 `artifacts/m6/mac-follow-dry.jsonl` |
| Live（Mac）の準備 | Live 12.4.6（Macは12.4.7でない）。Control Surface 1 = OmoiIroBridge（Log.txtで確認）、IAC Clock Sync ON、IAC Control Track/Remote ON、Link OFF。空のSet |
| M2 Live追従（CLI `bridge-clock`、120 s、利用者がPlay→約80 s→Stop） | ブリッジ：locked、サンプル 8,311（再生中 7,897、約100 Hz）、再生中の不連続 0、start時の不連続 1（想定どおり）、stall 0、fault 0。Clock：start 1 / SPP 1 / stop 1、tick 3,781（120 bpm × 78.8 sと一致）、bpm 119.999。Liveの再生・停止、ブリッジとClockの両経路がMacで動作。記録 `artifacts/m6/mac-m2-prep2.jsonl`（`mac-m2-prep.jsonl` は窓の外で再生した空振り） |
| LED追従（GUI、`OMOI-IRO.app`、demo.show.json、実機LPX） | 利用者が「開始」→ LiveをPlay → LaunchpadのLEDがショーに追従（17:10ごろ、利用者の目視「LED光りました」）。途中でLPXの接続が一度切れて挿し直したが、その後の開始で復帰。Mac + Live 12.4.6 + OmoiIroBridge + IAC Clock + Launchpad X の本番構成で通し動作を確認。数値の記録はアプリの診断を本番前に控える |
| Pulseが見えない（2件） | (a) demoの `beat_source` が `ableton_link` で、MacはLinkなし → 拍unknownでPulse透明。`editor_new` の既定もLinkで、GUIに変更欄がなかった。→ Opusが同日修正（既定 midi_clock、`set_sync` 編集欄、未保存時の3択。215件pass）。Macには `examples/demo-mac.show.json`（midi_clock版）を配置。(b) Liveが再生中にアプリを「開始」すると、Clockのtickは届く（10 sで480 tick）がStart/SPPが来ないため位相未確定でPulse透明。Stop → Playで位置情報+Continueが来て復帰（`/tmp/mac-clk-peek2.jsonl`：song_positions 2、continues 1、position_valid true）。→ 運用ルール「開始してからPlay、再起動したらStop → Play」を §8 に追加。根本対応としてOpusに「Clock原点未確定のときブリッジのbeatsを仮の原点として一度使う」をGO → 同日実装（218件pass）：`ClockDecoder::seed_origin`。拍源midi_clockで原点未確定、ブリッジlocked・playing・advancing、Clock推定bpmとブリッジtempoの差5%未満のとき、1接続につき1回だけブリッジbeats（age×tempoで外挿、×24で丸め）を原点に入れる。Start/SPPで上書き。イベントログ・拍源タイル「bridge の拍で原点を補完」・診断 `clock_seeded`。Windows実機で確認（18:40、Live 12.4.7のループ再生中に新releaseを開始）：0.02 sでブリッジlocked、0.12 sで原点補完（8.54 beats）、拍源タイルlocked「bridge の拍で原点を補完」、補完件数1。`artifacts/m6/win-m6-seed-01.jsonl`。Macは再ビルド後に確認 |

