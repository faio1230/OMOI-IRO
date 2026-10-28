# Opusセッションに渡すP1実装プロンプト

このファイルのコードブロック内を、プロジェクトフォルダー `<repo>` を開いた別のClaude Code（Opus）セッションへ貼り付けてください。

役割分担：Opusセッションが実装担当、既存のFableセッション（アドレス `dj-lp-midi-show-b6`。Claude Desktopへ移す前は `dj-lp-midi-show-e8` だった。computer use可）が設計担当。設計担当は設計資料・検証文書・Live実機操作を受け持ち、実装担当は `tools/p0-probe/` のコードを受け持つ。

```text
このプロジェクトのP0検証は2026-10-28にWindows側まで完了している。あなたは実装担当として、P0で未実装だった機能と、実測で分かった問題への対応を tools/p0-probe/ に実装してほしい。設計担当のセッション（アドレス dj-lp-midi-show-b6。ListAgentsで見つからなければ、このプロジェクト名のFableセッションへ送る）が同時に動いているので、分担と連絡の規則を守ること。

最初に読むもの：
1. CLAUDE.md と README.md
2. docs/validation/p0-results.md（Windows実測の結果と設計上の確認事項）
3. docs/validation/p0-runbook.md（手順。特にB1、B2、C）
4. docs/validation/environment.md
5. docs/07-decisions.md（O04、O05、O11〜O13）
6. docs/09-claude-handoff.md、docs/02-architecture.md
7. tools/p0-probe/README.md と src/（mtc.rs、midiclock.rs、monitor.rs、replay.rs、ports.rs、gen.rs）

P0の実測で確定している事実（再調査不要）：
- Windows 11 25H2、Rust 1.94.1 stable-msvc、midir 0.11.1（WinMM）。p0-probe は26 tests pass。
- loopMIDIに DJSC Control / DJSC Timecode / DJSC Clock がある。起動順は loopMIDI → LiveMTC Bridge → Live。
- Live 12.4.7（Windows）はloopMIDIポートを "<PC名> (Port N)" と表示する。WinMM側は本来の名前。Live "(Port 6)" = "DJSC Clock"。
- LiveMTC 1.1.0 のMTC（30 NDF）は seek・loop・テンポ変更をすべてFull Frameで反映する。Full Frame後にQuarter Frameが始まるまで約0.45 s、2系列整合のlockまで約0.6 s。Play直後に起点（01:00:00:00）のFull Frameが一瞬来て、約250 ms後に正しい位置のFull Frameが来る。
- テンポ変更でMTCの秒位置が飛ぶ（LiveMTCは拍位置×現在テンポ）。この扱い（O12）は設計担当と利用者が決める。実装側で勝手に補正しない。
- LiveのMIDI Clock：位置0からのPlayは SPP 0 → Start、位置≠0は Stop → SPP → Continue。ループ折返しではSPPを再送しない。Sync OFFでは Stop なしにtickだけ止まる。
- 生記録は artifacts/p0/*.jsonl（win-s03-*, win-s04-*, win-portmap-*, win-loopback-*）。これらは実測なので、fixtureとして使ってよい。

今回の実装範囲（この順で）：
1. デコーダー改修（O13）：Full Frameで位置が飛んだとき、直前の補間位置との差が閾値（暫定2フレーム）を超えれば discontinuity として記録・表示する。artifacts/p0/win-s04-c4-arr.jsonl と win-s03-c3-arr.jsonl のreplayで、Full Frame由来の不連続が検出されることをテストにする。既存の合成fixtureのテスト結果は変えない。
2. MIDI Clock途絶の判定：tickが一定時間（暫定500 ms、BPMから算出した2拍分でもよい）来なければ transport を "unknown"（または stalled）にし、playing のまま表示しない。win-s03-c6c7-arr.jsonl のC7区間で確認する。
3. lpx-check の実装：docs/validation/p0-runbook.md C の1〜7の順。完全一致名か #index で入出力を指定。Device Inquiry（F0 7E 7F 06 01 F7）→ Mode照会（F0 00 20 29 02 0C 0E F7）→ Programmer Mode（... 0E 01 F7）→ 低輝度RGB 1パッド → 押下・解放の受信 → 通常終了とCtrl+Cの両方で消灯とLive Mode復帰（... 0E 00 F7）。機器の応答を確認するまでmode変更もLED送信もしない。機器未接続のときはエラーで止まる。SysExの組み立てと応答解析は単体テストにする。
4. route-check の実装：LPX MIDIポート（またはテスト用の任意入力）からのNote/CCを "DJSC Control" へ転送し、Note On velocity 0 を Note Off に正規化する。転送した記録をJSONLに残す。Live側のRemote ON設定は設計担当が行う。
5. Link観測（--features link）：libclangが導入されたら着手。導入前は未実装のまま。実装時は tempo・beat・phase・peer数・is_playing の観測だけ。Linkへtempoやtransportを送らない。is_playing の初期 false をLive停止と扱わない。

実装方針（P0から継続）：
- midir callbackでは時刻採取・コピー・try_send だけ。デコード・表示・ファイル書込は別スレッド。
- 異なるconnectionのbackend timestampを直接比較しない。共通の単調時計を時刻軸にする。
- 30 NDF以外を30 fps扱いしない。
- 合成fixtureやreplayの成功を、Liveや機器との接続成功として報告しない。
- Tauri GUI、F1、OSC、ノードグラフ、LTC変換、完成版crate構成へ進まない。

分担と連絡：
- あなたが編集するもの：tools/p0-probe/ 配下（src、tests、fixtures、README.md、Cargo.toml）。
- 編集しないもの：docs/ 配下、prompts/、artifacts/p0/ の既存記録、CLAUDE.md。設計変更や検証結果の追記が必要なら、内容を設計担当へ送る。
- 連絡は SendMessage で dj-lp-midi-show-b6 宛て。着手時に「担当範囲と触るファイル」、完了時に「変更点、cargo test の結果、設計担当に確認したい点」を送る。設計担当からの返信を待つ必要がある判断（閾値の決め方、表示形式、設計資料との食い違い）は、その作業だけ止めて他を進める。
- 機器を使う確認（LPX、Live操作）は設計担当がcomputer useで行う。実装完了後に実行コマンドと期待結果を渡す。

完了報告：実装した項目、cargo build / cargo test の結果、実測記録のreplayで確認した内容、未実施項目（libclang待ち、LPX未接続）、設計担当への確認事項を分けて示す。

まず 1 のデコーダー改修から始めてください。
```
