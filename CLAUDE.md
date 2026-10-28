# OMOI-IRO（作業名 DJ Show Control）作業ガイド

このプロジェクトは、Ableton Live 12 Suiteに追従するRust製のMIDI入力ルーティング・LED演出アプリ「OMOI-IRO」を開発している。GUIの計画はTauri 2 + React / TypeScript。

## 最初に読む資料

- [引き継ぎ設計とP0仕様](docs/09-claude-handoff.md)
- [合意済み条件と未決事項](docs/07-decisions.md)
- [Ableton Link採用案](docs/08-ableton-link.md)
- [全資料の索引](README.md)

P0の開始プロンプトは[prompts/claude-p0.md](prompts/claude-p0.md)。このガイドを読んだだけで作業範囲を広げず、そのターンで利用者が依頼した工程を進める。

2026-10-28時点：P0は完了し、共通時計はLive内スクリプトのブリッジ方式に改訂された（[docs/10-live-bridge.md](docs/10-live-bridge.md)）。実装は[docs/11-implementation-plan.md](docs/11-implementation-plan.md)のマイルストーンに従う。Cargo workspaceは`crates/`と`apps/probe`、ビルド手順は[BUILD.md](BUILD.md)。

## 設計上の重要条件

- Liveが再生マスター。ArrangementとSessionの演出は共通タイムコードへ追従する。
- Linkは拍・tempo、MTCは曲中の共通時刻。Link単独で絶対位置を取得できるとは扱わない。
- Linkへtempoやtransport変更を送らない。途中参加時のtransport状態は未確定として扱う。
- LPXの入力とLEDはアプリが管理。Liveへの操作転送とLED出力は別経路。
- CoreはTauriとMIDI I/Oから独立させる。P0では完成版の全crateを作る必要はない。
- 30 fpsは暫定。タイムコード起点、具体的なOS版、対応保証は未確定。
- MacはApple Silicon。MacとLPXの実機環境はあるが放置できない。

## 進め方

文書の確定条件、提案、未確認事項を区別する。既存成果物と利用者の変更を保持する。利用者から新しい条件が示されたら、関連する設計とタスクを整合させる。

環境不足でも、機器なしでできる実装・テスト・手順準備は進める。実機がない状態で試験成功を記録しない。実測、合成データによるテスト、未実施を分け、採用判断の根拠を残す。

P0依頼では最小ツールと技術検証までで区切る。F1、LTC変換アプリ、OSC、ノードグラフ、完成版GUI等を先回りして追加しない。
