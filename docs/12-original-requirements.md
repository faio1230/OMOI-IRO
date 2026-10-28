# OMOI-IRO 元要件（タイムライン・モジュール・ノードシステム設計要件）との対応

利用者が設計開始時にChatGPTで作成した要件文書「DJ Show Control — タイムライン・モジュール・ノードシステム設計要件」を、2026-10-28に設計担当が現在の実装と突き合わせた記録。元文書の全文は利用者の手元にあり、ここには対応状況だけを残す。元文書自身が「最初から全機能を実装する必要はないが、データモデルとアーキテクチャは拡張を妨げないものにする」「MVPの最小構成を提示する」と求めているため、MVPで外した項目は不整合ではなく段階分けとして扱う。

## 対応状況

| 元要件 | 状態 | 現在の対応 |
| --- | --- | --- |
| 2.4 任意位置からの状態再現 | 実装済み | Coreは時刻を引数に取る評価器。決定性（同じshow・時刻・拍で同じFrame）を単体テストで確認。`seed` はスキーマに予約 |
| 2.1 トラックとクリップ | 実装済み（秒単位） | layer（配列順が合成順）とtimeline clip（開始・長さ・フェード）、continuous layer。小節・拍単位の配置は未実装 |
| 2.2 時間基準 | 方針変更 | 元要件は「MTC受信」。実測でLiveMTCがテンポ自動化で飛ぶため、共通時計はLive内スクリプトの曲中時刻（[10-live-bridge.md](10-live-bridge.md)）。MTCは本アプリが外部機器向けに生成する。拍とテンポはLink/Clockから取り、テンポマップは未実装 |
| 2.3 編集機能 | 一部 | U04で追加・移動・長さ・複製・削除・数値入力・Undo/Redo。キーフレーム・オートメーション・イージング・小節スナップ・マーカー・グループ化は後続 |
| 3 Modular System | 最小 | Effect traitと組込みregistry（Solid / Chase / Pulse、type + version + params）。Modifier / Logic / Input系のモジュール種別と、ポート定義を持つ共通インターフェースは未着手 |
| 4 Node Graph System | 未着手 | ロードマップ「MVP以後」6番。データ階層は Show → layers → clips → preset で、クリップがグラフを参照する形ではない |
| 5 Timelineとグラフの統合、パラメータ公開 | 未着手（余地を確保） | U04/U05のデータモデルで、clipのparamsを将来「値または自動化」に、presetのeffectを将来「組込みまたはグラフ参照」に拡張できる形にする（下記） |
| 5.3 レイヤー合成 | 実装済み | over / add / max、opacity、master brightness。操作表示用レイヤーの独立は未実装 |
| 6 デバイス抽象化 | 実装済み | 抽象LED Frame → Device Adapter（Launchpad X、仮想機器、RGB7量子化、差分送信、再接続）。TRAKTOR F1はMVP外（「MVP以後」1番） |
| 7 リアルタイム演出との共存 | 一部 | Trigger/Actionの境界（RouteMidi、Blackout、再評価）。パッド操作で演出を上位レイヤーに重ねる機能と発火条件は後続 |
| 8 UI | 一部 | Performance View（演奏タブ）、Device Preview、接続設定は実装済み。Timeline Editor（U04）、Parameter Inspector（U05）は実装中。Node Editor、MIDI Mapping Editor（MVPはLiveのMIDI Mapを使う）は後続 |
| 9 拡張性・再現性・安定性・性能・データ互換・テスト容易性 | 実装済み | trait境界、決定性テスト、スレッド分離と容量1のmailbox、faultの隔離、NFR03計測、Schema 0.1.0→0.1.1移行、fake backend / fake clock / 合成ブリッジ / 仮想機器 |
| 9 マルチプラットフォーム | Windowsのみ確認 | macOSはM6のMac枠。LinuxはOS依存部を分離してあるが未対応 |
| 9 同期精度 | 実装済み | ブリッジとMTCの健全性（locked / holdover / lost / blackout / fault）、不連続検出、補間、再lock計測 |
| 9 安全性 | 一部 | 実装済み：マスター輝度、Blackout（LEDのみ、MIDI転送は維持）、終了時の消灯とNote Off、fault時に不正送信を続けない、設定時の逆流・二重割当の拒否、同期断の保持と消灯、再接続時の全体再送。未実装：ストロボ周波数の制限と警告、光過敏性対応プリセット、実行時のMIDIループ検出、Performance Modeでの編集ロック |
| OSC、Linux | 未着手 | 後続 |

## 今の工程で確保する余地（2026-10-28、設計担当の判断）

1. U04/U05のデータモデル：clipのparamsは現在「値」だけだが、将来「値または自動化（キーフレーム列）」を入れられるよう、保存形式と内部型を値型のタグ付きにできる構造にする（Schema 0.1.xの互換は壊さない。追加はSchema 0.2で行う）。presetのeffectも将来「組込み（type + params）」と「グラフ参照」をタグで区別できる形にする。→ U04で実装（2026-10-28）：`omoiiro_core::model::Param<T>`（enum、現在はValueのみ。保存は中の値そのものなので0.1.x形式はバイト単位で同一）、`CompiledEffect::paint(local_us, …)` が時刻を受けるので自動化は曲線を引く形で足せる、Effectの `type` enumに `graph` を足す構造（現在は読込で拒否することをテスト）、Undo/Redo履歴はshow全体のスナップショット形式なので操作追加で変わらない。clip単位のparams上書きはSchema 0.2で `clip.params` として追加予定。
2. 安全性：ストロボ相当（高速点滅）の周波数上限とマスター輝度の上限を端末設定に持ち、超える演出は警告と制限をかける。M6の実機前に入れる。
3. 元要件の「操作表示用レイヤーの独立」と「リアルタイム演出の上位レイヤー」は、Trigger/Actionの拡張としてMVP後に設計する。
