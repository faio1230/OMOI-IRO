# Claudeに渡すP0作業プロンプト

このファイルのコードブロック内を、プロジェクトフォルダーを開いたClaude Codeへ貼り付けてください。開始時のフォルダーはWindowsでは `<repo>` です。別PCへコピーした場合は、そのコピー先を使ってください。

このプロンプトは、貼り付けて実行した時点でP0の最小実装と検証を依頼するものです。完成版アプリを一括で実装する依頼ではありません。

```text
このプロジェクトの設計資料を読み、DJ Show ControlのP0技術検証を進めてください。
作業を計画の提示だけで終えず、環境確認、最小ツールの実装、可能なテスト、実機検証手順と結果の文書化まで進めてください。

最初に読むもの：
1. CLAUDE.md と README.md
2. docs/09-claude-handoff.md
3. docs/07-decisions.md
4. docs/08-ableton-link.md
5. docs/00-feasibility.md と docs/02-architecture.md
6. docs/01-prd.md、docs/05-roadmap.md、docs/06-tasks.md

設計の前提：
- Ableton Live 12 Suiteを音楽再生のマスターとする。
- ArrangementとSessionの両方を初期から扱う。
- 演出はセット全体の共通タイムコードに追従する。Session clip単位の再生位置追跡は不要。
- Launchpad XはアプリがProgrammer Modeで管理し、Note/CCをLiveへ転送する。
- 拍・BPMはAbleton Linkを第一候補とし、MIDI Clockを代替として明示選択できるようにする。
- 共通時刻はMTC。LiveMTC等のブリッジを検証する。Linkのbeatを曲中の時刻に読み替えない。
- Linkには排他的なマスター指定がない。本アプリからtempo・beat・transport変更を送らない。
- Linkの途中参加でtransport初期値をLiveの停止状態と断定しない。
- タイムコードは30 fpsが暫定候補。正式なrateと起点は未定。サンプルの01:00:00:00を運用上の確定値にしない。
- Windowsは10または11、MacはApple Silicon。具体的なOS版等は環境調査で確認する。
- MacとLaunchpad Xは検証可能だが放置できない。Mac実機試験は短い立ち会い枠に分ける。

今回の作業範囲：
1. 既存ファイルと変更を確認し、導入済みのツール、OS、CPU、取得できるLive/Max・driver情報、MIDI環境を調査する。
2. Rustの最小CLI同期モニターを作る。新規ならtools/p0-probe/等の独立した検証用packageを優先する。
3. ポート一覧、MTC観測、LinkのBPM/拍/peer/transport観測、ログ保存とreplayを用意する。
4. 明示した入出力ポートだけでLPXのidentity、Programmer Mode、RGB、入力転送、終了時復帰を確認できるようにする。
5. 再生・停止・再開・seek・loop・tempo変更・Session切替・途中参加・同期断・USB再接続のテスト手順を作る。
6. 実行できる範囲を検証し、証拠を記録する。環境不足の項目はnot-runとして残す。

実装方針：
- P0では完成版の5 crate構成やTauri GUIを先に生成しない。
- Link bindingをoptionalにし、Linkのbuild問題でMTC/MIDIの独立作業を止めない。
- callback内でGUI処理、render、ファイル書込、送信待ちをしない。有界queueを使い、欠落を記録する。
- 異なる入力connectionのtimestampを直接比較しない。共通の単調時計へ対応付ける。
- P0は30 NDFから検証してよい。他のfpsは未対応と明示し、勝手に30 fps扱いしない。
- 記録replayや合成fixtureの成功を、Liveや実機との接続成功として報告しない。
- 依存API・対応OS・Link SDKの利用条件は一次資料で確認する。既存設計に誤りがあれば根拠を示して修正する。

作業の進め方：
- 読取専用で分かることは調べ、通常の実装判断やローカルテストのたびに確認を求めない。
- 不明点があっても、その回答を必要としない作業は進める。
- Liveや機器が使えなければ最小ツールと手順を完成させ、必要な接続・操作を短くまとめる。
- Macへのアクセスや試験時間は推測しない。立ち会いが必要な実機操作は準備を済ませてから利用者と調整する。
- システムdriverの新規導入や既存Live Setの変更が必要なら、対象と理由を具体化して利用者に確認する。
- 共通時計の意味を変える、MTC要件を外す、LTC変換の新規開発へ進む等の変更は、証拠と選択肢を示して確認する。
- F1、OSC、ノードエディター、外部plugin、完成版GUIへ範囲を広げない。

成果物：
- docs/validation/environment.md
- 最小CLIと実装した操作の実行方法
- docs/validation/p0-runbook.md
- docs/validation/p0-results.md
- 必要な範囲の設計判断・タスク状態の更新

完了報告では、実装したもの、実行したコマンドと結果、実測と合成テストの区別、未実施項目、次の判断を示してください。
ツール完成と両OS実機検証完了を区別し、短時間試験を連続2時間試験の代わりに数えないでください。
P0の結果をまとめたところで区切り、P1以降の完成版実装には自動的に進まないでください。

まず環境確認から開始してください。
```
