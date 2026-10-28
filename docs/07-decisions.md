# OMOI-IRO 設計判断と未決事項

## 利用者と確認済みの条件

| ID | 決定 | 根拠 |
| --- | --- | --- |
| A01 | Ableton Live 12 Suiteを使う | 利用者回答 |
| A02 | ArrangementとSessionをMVPから同等に扱う | 利用者回答 |
| A03 | Sessionもセット全体の共通タイムコードへ追従 | 利用者回答 |
| A04 | Launchpad XはアプリがProgrammer Modeで管理しLiveへ転送 | 利用者回答 |
| A05 | LTC→MTC変換は必要時の代替候補として検討可能 | 利用者回答。変換アプリの開発着手を意味しない |
| A06 | CoreはRust、GUIはTauri 2 + React / TypeScript | 当初要件 |
| A07 | 初期OSはWindows/macOS、Linuxは将来検討 | 当初要件 |
| A08 | 今回は設計段階、全機能の実装を開始しない | 当初依頼 |
| A09 | Liveの同期機構も利用してよい | 追加回答。Ableton Linkを拍同期の第一候補として評価 |
| A10 | タイムコードは30 fpsを暫定候補とする | 利用者回答は「多分30fps」。正式なrateと開始時刻は未確定 |
| A11 | Windows 10または11、MacはApple Siliconを想定 | 利用者回答。OSの具体的な版とWindowsのCPUは未確認 |
| A12 | MacとLaunchpad Xの検証環境はあるが放置運転不可 | 利用者回答。実機試験は立ち会い可能な時間に限定 |
| A13 | 共通時計はLive内スクリプトの曲中時刻サンプル、MTCはアプリが生成して外部へ出す | 利用者回答（2026-10-28、「アプリ経由で出す形にして」）。LiveMTC不採用。[10-live-bridge.md](10-live-bridge.md) |
| A14 | テンポはオートメーションで運用する | 利用者回答（2026-10-28）。手動テンポ変更はLive自体の時刻が飛ぶ |
| A15 | タイムコードは30 fps（NDF）で確定。起点はshowごとの自由な設定値とする | 利用者回答（2026-10-28）：「fpsは30、起点はそもそも自由でいい。映像もオフセット設定できる」。既定値は01:00:00:00 |
| A16 | 2時間の連続安定性試験とLaunchpad Xの機器検証（S01/S02/S05/C8）は本番アプリ実装後にまとめて実施する | 利用者回答（2026-10-28）。試作ツールでの実施は省略 |
| A17 | 本番アプリ（Rust crate構成 + Tauri 2 / React）の実装計画を作成し、着手する | 利用者回答（2026-10-28）。設計担当が計画を書き、実装担当（Opusセッション）が実装する |
| A18 | アプリの製品名は「OMOI-IRO」 | 利用者回答（2026-10-28）。由来：テーマカラーが赤であること、開発目的のイベントが「思い」をテーマにした音楽イベントであること、Launchpad Xをキャンバスに見立てていること。利用者の追加指示（同日）：フォルダー名や資料名もOMOI-IROへ変え、本番ビルド前にコード上の識別子もすべて改名する。対応：資料の題名とObsidianのフォルダーは即時改名。リポジトリーのフォルダー名（`DJ_LP_MIDI_SHOW` → `OMOI-IRO`）は実行中のセッションが掴んでいるため、利用者がセッションを閉じて改名し、再開する。識別子の改名（crate `omoiiro-*`、CLI `omoiiro-probe`、Remote Script `OmoiIroBridge`、仮想ポート `OMOI-IRO Control/Timecode/Clock`、記録の `DJSC` 接頭辞、bundle identifier）はM5の冒頭（GUI着手前、本番ビルド前）に一括で行う。設計資料の作業名「DJ Show Control」は本文中に残ってよい。商標：J-PlatPatで「Omoiiro」（標準文字）がソニーグループ株式会社の登録商標（登録6174824、商願2018-118556、出願2018-09-19、登録2019-08-30、存続、区分09/16/18/19/21/24/25/27/28）であることを利用者が確認（2026-10-28）。称呼が同一のため「OMOI-IRO」表記でも類似と判断され得る。公報（2026-10-28に利用者が取得）で第09類の指定商品に「コンピュータソフトウェア」「電子計算機用プログラム」が含まれることを確認。利用者はGitHubでオープンソース公開する意向。無償でも継続的に公開・配布するソフトウェアの名称は商標の使用と評価され得るため、公開名は「オモイイロ」と称呼が異なる名前に変える方針（候補と確認手順は設計担当が提示、最終決定は利用者）。「思色」の漢字表記も称呼が同一のため回避にならない。歴史的仮名遣いの「OMOHI-IRO」案は、J-PlatPatの称呼（類似検索）で「オモヒイロ」を引くと46件中にソニーの「Omoiiro」（オモイイロ）が含まれ（2026-10-28、設計担当が内蔵ブラウザーで確認）、同じ検索で「想ひ野」が「オモヒノ, オモイノ」と両読みで登録されていることからも、特許庁の判定ロジックは「ヒ」を「イ」と同視し得るため回避にならない。周辺には「おもいろ」（オモイロ、第31類）、「OmOirO」（オモイロ、第41・42類）もある。利用者の最終決定（2026-10-28）：上記の類似リスクを承知のうえで、公開名もOMOI-IROのまま進める（商品化しない、オープンソースで公開）。緩和策：表記は「OMOI-IRO」に統一し「Omoiiro」は使わない、READMEにソニーとの無関係の明記、識別子は1か所にまとめて申し立て時に改名できる状態を保つ。識別子の一括改名（M5冒頭）はOMOI-IROで行う。この判断は法的助言ではない |
| A19 | 本番は2026-10-29。本番機はMac（Apple Silicon）。本番で使う出力はLaunchpad XのLED、Launchpad Xの操作のLiveへの転送、MIDI Clockによる拍同期。MTC出力は使わない。ショーデータは利用者が2026-10-28夜にGUIで作る。Launchpad Xは同日深夜に接続し、1〜2時間立ち会える | 利用者回答（2026-10-28夕方、「明日本番だから今日深夜にはLPX繋げる予定、だからそれまでに完成系まで持っていきたい」）。計画は[validation/m6-mac-showday.md](validation/m6-mac-showday.md)。Macはこの時点までビルドも試験も未実施（最大のリスク） |

## 採用を提案する判断

### 同期入力を時刻と拍に分ける

MTCをshow timeの入力、Ableton LinkをBPM/拍の第一候補とする。MIDI Clockは代替として明示選択できるようにする。Linkに排他的なマスター設定はないため、本アプリは状態変更を送らず、Liveと本アプリだけのsessionを運用前提とする。Linkの拍番号からLiveの曲中の絶対時刻を作らない。詳しくは[Linkの採用案](08-ableton-link.md)。

LiveMTCを最初の候補にする。理由は、Suite環境で既存の送信ブリッジを試せるため。代替は専用M4Lブリッジ、その次に既存映像経路を利用できる場合のLTC変換。2026-10-28のWindows実測（[validation/p0-results.md](validation/p0-results.md)）では、Play/Stop/seek/loop/デバイス断でデコーダーは設計どおりに動いたが、LiveMTCの時刻は「拍位置 × 現在テンポ」の換算値で、テンポ変更時にMTCがFull Frameで飛ぶ（S04 C4はfail）。利用者の決定（2026-10-28）：共通時計の源はLive内Remote Scriptの曲中時刻サンプル、MTCはアプリが生成する出力。仕様は [10-live-bridge.md](10-live-bridge.md)。LiveMTCは不採用、LTCは条件付き代替のまま。

### 共通時計と個別クリップを分離する

Sessionのクリップ起動では演出の時刻を変更しない。Liveのグローバルtransportが変更された場合だけ共通時計へ反映する。この合意により、MVPにクリップ追跡・シーン状態の専用APIを追加しなくてよい。

### MIDIポートをアプリへ集約する

LiveはLaunchpad Xを直接開かず、仮想Controlポートから操作を受ける。LEDはアプリだけが出力する。Windowsのmulti-client可否にかかわらず、LEDの意味的な競合を防げる。

### 任意時刻から演出を評価する

timeline clipを逐次再生する状態だけで管理せず、現在位置から有効clipとlocal timeを計算する。組込み効果は解析的に評価する。拍依存の演出はBeatStateを引数に含め、未知の過去操作を時刻だけから再現できるとは扱わない。

### 初期実装の範囲を限定する

LPX 1台・64パッド・3種類のEffect・最大8レイヤー・秒単位clip・固定入力ルートを初期範囲として提案する。32/64bitの汎用plugin ABI、OSC、F1、ノードグラフ、音声処理は後続にする。拡張境界を定義することと、拡張機能をすべて実装することを分ける。

### データと端末設定を分ける

showはJSON、Schema、固定ID、安定整形で管理する。作品に必要な同期原点やEffect設定をshowへ、OSポートや機器校正を端末設定へ置く。プリセットはshow内へコピーし、外部ライブラリーの更新による意図しない変化を防ぐ。

## 確定前の項目

| ID | 項目 | 暫定案 | 確定する時点 |
| --- | --- | --- | --- |
| O01 | 映像側と共有するfpsと起点 | 決着（A15）：30 fps NDF。起点はshowの `sync.origin` で自由に設定（既定01:00:00:00）。映像側はオフセットで合わせる | — |
| O02 | Windows/macOSの具体的な対応版 | Windows側はWindows 11 Pro 25H2（build 26200.9457）、i9-12900で記録済み（[validation/environment.md](validation/environment.md)、2026-10-28）。macOS版とMac機種は未確認。A19によりMacが本番機になったので、2026-10-28夜のMac枠M0で取得する | 2026-10-28夜（M0） |
| O03 | 実機試験の時間枠 | MacとLPXは利用可能。放置不可。短時間の立ち会い試験を計画。2時間試験とLPX検証は本番アプリ実装後（A16）。A19（2026-10-28）：本番が翌日のため、Mac枠は2026-10-28夜、LPXは同日深夜に1〜2時間。2時間連続はWindowsで放置、Macは30分程度の連続再生まで | 決着（A19） |
| O04 | MTCブリッジ | 決着（2026-10-28）：LiveMTCは不採用（テンポ変更でMTCが飛ぶ、seek後の再lock約1.1 s）。自前ブリッジ `DJSCBridge` + アプリのMTC生成（`mtc-out` 試作）でWindowsのC1〜C7が合格（seek後の再lock 126〜152 ms、残差p95 5 ms以内、[validation/p0-results.md](validation/p0-results.md) B4）。残りはMacと2時間試験 | Mac枠M3と連続試験 |
| O05 | Windows仮想バス | loopMIDI 1.0.16.27で `DJSC Control` / `DJSC Timecode` / `DJSC Clock` を作成し、Live 12.4.7との経路（Clock出力→アプリ、LiveMTC→アプリ）を確認（[validation/p0-results.md](validation/p0-results.md) PM-2、S03）。Live 12.4はこれらを `<PC名> (Port N)` と表示するため、名前での対応付けはWinMM側でしか行えない。起動順はloopMIDI→Bridge→Live固定。新MIDI Servicesの仮想endpointは未検証 | アプリ→Live方向はS02（`route-check`）後 |
| O06 | LEDの更新頻度 | 30 fps、60 fpsは測定比較 | S05後 |
| O07 | 同期断と停止 | Stopはhold、同期断250 ms補間・1秒で黒 | 実演確認で調整 |
| O08 | 許容遅延 | PRDに初期目標を記載 | P0計測後 |
| O09 | 配布形態 | 個人利用向けから開始する案 | 外部配布を行う前 |
| O10 | Linkの利用可否と依存採用 | 第一候補。rusty_link 0.4.9はGPL-2.0-or-later、cmakeとbindgen（libclang）でLink SDKをビルドする。2026-10-28にLLVM 23.1.4を導入し、Windowsでビルド成功（cmake 3.26.4のためVS 2026開発者シェル + NMake generatorが必要。[validation/environment.md](validation/environment.md)）。観測機能を実装し、2026-10-28にWindowsで実測（[validation/p0-results.md](validation/p0-results.md) S07）。tempo・phase・peer数・is_playing の観測、テンポ自動化の追従、peer喪失は動作。重大な注意：アプリ側（rusty_link）のStart Stop Syncを有効にすると、Live 12.4.7はPlay後にトランスポートが進まなくなる（2回再現、順序に依らず、Live再起動で回復）。SSS OFFでは正常。設計判断：アプリはStart Stop Syncを有効にしない。再生状態はMIDI ClockのStart/Stop/Continueまたは自前ブリッジの `is_playing` から取り、Linkは tempo と beat/phase だけに使う。また、Liveより先にアプリがLinkセッションを作ると、Live参加時にアプリの既定テンポ（120）がセッションに採用される。運用はLive Link ON → アプリ参加の順とし、アプリは参加時のセッションtempoを自分の既定値にしない（p0-probeはpeer 0の間tempoを表示しない仕様に変更済み）。切り分け（2026-10-28、[validation/p0-results.md](validation/p0-results.md) S07）：素の `ableton::Link` 版でも `LinkAudio` 版でも、SSS ONのpeerがいるとLive 12.4.7は新セッションでの最初の再生で固まり、SSS OFFとpeerなしでは起きない。原因はLive側のSSS開始処理と判断し、rusty_link / LinkAudio / p0-probe固有ではない。設計判断は確定：アプリはSSSを有効にしない（既定OFF、調査用の明示オプションのみ）。本番アプリでは「Liveより先にセッションを作らない仕組み」（peer検出までLink無効化等）を設計項目にする。GPLの扱い、Mac、途中参加は未確認 | Mac確認後 |
| O11 | Live 12 Suiteの実行環境 | Windows PCにLive 12 Suite 12.4.7（Max 9.1.5同梱）とLiveMTC 1.1.0を導入済み。音声はGeneric Low Latency ASIO Driver（Realtek ASIOはエンジン無効、Voicemeeter Virtual ASIOはVoicemeeter未起動だとトランスポートが遅くなる）。Linkは未検証（libclang未導入） | S07前に音声ドライバーを再確認 |
| O12 | 共通時計とテンポ変更の関係 | 利用者回答（2026-10-28）：普段からテンポのオートメーションを書く。したがって(a)テンポ固定運用は不採用。LiveMTC 1.1.0は拍位置×現在テンポで時刻を作るため、テンポ変更のあるSetではLiveの時刻表示（tempo mapの積分）と一致せず、送信元として採用できない。残る選択肢：(c-1) Live APIの `Song.get_current_smpte_song_time`（tempo mapを反映したSMPTE時刻）を読む専用ブリッジ（Python Remote ScriptまたはM4L）を作り、位置サンプル＋再生状態をアプリへ送る。アプリ側で補間し、必要なら外部向けMTCを生成する。(c-2) Warp OFFのLTC音声トラックをArrangementに置き、LTC→MTC変換（機器またはソフト）で受ける。(b)拍ベース共通時計は映像側との秒同期に使えないため単独では不採用。設計担当の提案は(c-1)を先に試し、main thread由来のジッターが大きければ(c-2)へ。計測実験（2026-10-28、[validation/p0-results.md](validation/p0-results.md) O12節）の結果：(c-1) は成立する。Remote Scriptの曲中時刻リスナーは約60 Hz（16.7 ms間隔）で発火し、テンポ自動化（120→140→90→120）をまたいでも `get_current_smpte_song_time` は連続（31.5 sで飛びなし、残差中央値2.9 ms、p95 4.2 ms、最大6.9 ms）。設計担当の提案：MTCブリッジ（LiveMTC）に代えて、Live APIから曲中時刻・拍・テンポ・再生状態を読む自前ブリッジを共通時計の第一候補にする。送信経路はUDPでもMIDI（SysEx）でもよく、アプリ側は位置サンプルを補間し、必要なら外部機器向けMTCをアプリが生成する。留意点：手動テンポ変更（自動化なし）ではLive自体の時刻が拍×現在テンポで飛ぶので、本番は自動化で運用する。Arrangement録音中はリスナーが止まる（タイマー経由で補う）。この変更は「共通時刻はMTC」という前提の見直しになるため、利用者の確認を取ってから設計資料（02-architecture、09-claude-handoff）を改訂する | 利用者の確認後 |
| O13 | Full Frameによる位置ジャンプの扱い | 2026-10-28に実装済み：FF前後の位置差が2フレームを超えたら `discontinuity(full_frame)` として記録し、`ff_disc` をQF由来の `disc` と別に数える（[validation/p0-results.md](validation/p0-results.md) 再解析）。設計判断：(1) 停止中の位置変更によるFFも数える。停止中の位置変更は利用者の操作であり、演出側も追従すべき位置更新なので除外しない。イベントに直前の状態（from locked / holdover / acquiring）を付けて区別する。(2) Play直後の `01:00:00:00` → 正位置 のFF対は抑制しない。送信元固有の癖はアプリ層で扱う。(3) 「FF後にClockがplayingなら補間を続ける」は任意機能（既定OFF、`ff_continue_when_clock_playing`）として実装し、c3で評価した。ONでは折返し中の位置停止が無くなる代わりに、lock時に約1.5フレーム（約50 ms）前へ跳ぶ（LiveMTC固有のFF送出ずれと推定）。LiveMTC向けの固定オフセット補正は入れず、既定OFFのまま。代替ブリッジでは位置サンプル自体を連続に設計する。(4) Clock途絶は500 ms固定で `stalled`、Note Off正規化は `8n kk 00` | 代替ブリッジ（O12）の設計時に再評価 |

未回答項目は仮定を固定して進めず、必要な工程の入口で確定する。今回の設計資料は、これらを識別したレビュー稿として使用できる。

MacのIntel対応は初期の検証対象に含めない。短時間の試験を分割して機能確認を進めるが、分割試験の累積時間を2時間の連続安定性試験と同等には扱わない。長時間試験の未実施状態は、接続・機能の確認結果と分けて残す。

## 変更時に見直す範囲

| 条件の変更 | 再検討する内容 |
| --- | --- |
| Sessionで曲ごとの再生位置に追従したい | Clip identity、個別時計、M4L状態取得、showのcue起点 |
| Live標準Session操作も残したい | DAW surface所有権、モード切替、状態フィードバック |
| LTC生成・変換が必須になる | audio I/O、遅延予算、音声経路、別アプリの配布 |
| 生演奏を完全再現したい | 入力記録、tempo/phase履歴、override履歴、replay |
| 複数機器を初期対応したい | device別writer、色能力差、target割当、負荷上限 |
| Linuxを初期対応したい | ALSA、GUI依存、同期送信元、実機matrixと配布 |

## 設計段階の完了と実装段階への移行

今回の成果物はPRD、アーキテクチャ、モジュール構成、データ仕様、ロードマップ、タスク分解と技術調査である。これらは実装可能性の資料上の判断であり、実機での検証完了を意味しない。

次の作業単位はD01とP0の最小検証。S07でLink、S03/S04で共通時計の挙動を確認し、拍入力とMTCブリッジを選定してから、正式な実装計画を確定する。
