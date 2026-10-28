# 演出60案の対応表と拡張計画

2026-10-28、利用者が Codex に出させた「60種類の演出と共通基盤」の案を受けて、設計担当が分類した。元案は、発火（押下・解放・長押し・クリップ開始・反復）、寿命とキーフレーム、図形・領域・経路・マスク、粒子、複合素材、フレーム列と文字、入力履歴と状態、音反応、という共通基盤を求めている。方向性は [12-original-requirements.md](12-original-requirements.md) の元要件（タイムライン・モジュール・ノードグラフ）と一致しており、本番後の拡張計画として採用する。

本番は 2026-10-29。利用者の判断（2026-10-28 18時台、「今の速度感ならいける」）で、凍結時刻は設けず、§3 → §4 の順に進められるところまで進める。安全側の規則：本番用のビルド（Mac の `/Applications`）はテストが通って設計が受け入れた区切りだけを採用し、本番の Set で使う演出は Mac の実機で表示を確認したものに限る。未確認の演出は本番で使わない。

## 1. 区分

| 区分 | 意味 |
| --- | --- |
| 今すぐ | 既存の9種（solid / chase / pulse / strobe / noise / ripple / rain / wave / orb）とレイヤー合成・クリップのフェードで作れる。プリセット制作だけ（[13-preset-authoring.md](13-preset-authoring.md)） |
| 小拡張 | 既存 Effect に任意項目を足す、または入力駆動の小さな Effect を1つ足す。既存ファイルはそのまま読める。本番前の候補 |
| 新基盤 | 発火モデル、キーフレーム、領域・経路、粒子、マスク、複合素材、フレーム列、入力状態、記録のいずれかが要る。本番後 |
| 外部入力 | 音声入力など、取得経路の検証から必要。本番後 |

## 2. 対応表

| No | 名称 | 区分 | 今の作り方、または必要なもの |
| --- | --- | --- | --- |
| 01 | 水滴 | 今すぐ | ripple：青、ring_width 0.5〜0.8、speed 6、lifetime 1200 |
| 02 | 衝撃波 | 今すぐ | ripple：白、ring_width 2.5、speed 14、lifetime 300、fade exp |
| 03 | インクのにじみ | 今すぐ | ripple：ring_width 3、speed 1.5、lifetime 3000 |
| 04 | 二重の波紋 | 今すぐ | ripple の常時レイヤーを2枚（細く速い／太く遅い）、blend max |
| 05 | パッドの余熱 | 小拡張 | 入力駆動の `tap_glow`（押した位置が色の段階を経て消える、半径、反復） |
| 06 | 十字砲 | 小拡張 | 入力駆動の `cross`（押した位置から4方向へ走る光） |
| 07 | ダイヤモンド | 小拡張 | ripple に `shape: diamond`（マンハッタン距離） |
| 08 | 四角いソナー | 小拡張 | ripple に `shape: square`（チェビシェフ距離） |
| 09 | 火花 | 新基盤 | 粒子 |
| 10 | 打ち上げ花火 | 新基盤 | 粒子 + 経路 |
| 11 | 重力のしずく | 新基盤 | 粒子（重力・反射） |
| 12 | タップ・エコー | 小拡張 | `tap_glow` の反復（repeats、repeat_interval_ms、repeat_decay） |
| 13 | 長押しチャージ | 新基盤 | 発火モデル（保持・解放） |
| 14 | 星座をつなぐ | 新基盤 | 入力履歴 + 線分 |
| 15 | 熱の蓄積 | 新基盤 | パッドごとの状態と減衰 |
| 16 | デジタル・レイン | 今すぐ | rain：down、`#00C040`、head `#C8FFD8`、tail 5、flicker 0.5 |
| 17 | 逆さの雨 | 今すぐ | rain：up、青 |
| 18 | 火の粉 | 今すぐ | rain：up、橙、tail 1、density 0.2 |
| 19 | 横殴りの雪 | 今すぐ | rain：right、白青、speed 10 |
| 20 | ネオンの交通 | 今すぐ | rain 2枚（right シアン、left ピンク）、blend add |
| 21 | 水面のうねり | 今すぐ | wave 2〜3枚（speed と wavelength を変える）、thickness 2、blend max |
| 22 | 二重らせん | 小拡張 | wave に `phase`（0〜1）。2枚を phase 0 / 0.5 で |
| 23 | ネオン・リボン | 今すぐ | wave 2枚（太い色 + 細い明るい芯、同じ speed） |
| 24 | 月の浮遊 | 今すぐ | orb：edge 1.5、orbit、orbit_radius 1、speed 0.1 |
| 25 | 双子の惑星 | 今すぐ | orb 2枚（軌道半径・速度を変える） |
| 26 | ピンボール | 今すぐ | orb：radius 0.8、bounce、speed 8 |
| 27 | 蛍の庭 | 今すぐ | noise：緑、density 0.08、rate 0.7、smooth true、depth 0.8 |
| 28 | ワープ・トンネル | 小拡張 | ripple に `trigger: interval`（固定原点 origin_x/y、interval_ms）+ `shape: square` |
| 29 | 溶けるプラズマ | 新基盤 | 連続ノイズ場（値ノイズの補間）。暫定は noise smooth + 多色レイヤー |
| 30 | 8×8動画サンプラー | 新基盤 | フレーム列 + 変換ツール |
| 31 | 白い一撃 | 今すぐ | solid 白のクリップ、fade_out 150 ms |
| 32 | 色の着地 | 今すぐ | 31 の下にテーマ色の solid クリップ |
| 33 | 一走査 | 今すぐ | chase のクリップ長 = period_us（1周で終わる） |
| 34 | クロス・スイープ | 今すぐ | chase x と y を同時刻に2クリップ |
| 35 | 粒から豪雨へ | 今すぐ（近似） | rain の density を段階的に上げたクリップを連結。連続変化はキーフレーム（新基盤） |
| 36 | 加速する走査 | 今すぐ（近似） | period を短くした chase クリップを連結し、最後に 31。連続加速はキーフレーム |
| 37 | アイリス・オープン | 新基盤 | マスク |
| 38 | シャッター | 新基盤 | マスク |
| 39 | ピクセル崩壊 | 新基盤 | 粒子 + 元絵の取得 |
| 40 | 吸い込み→爆発 | 新基盤 | 粒子（収束・放射）。近似は 28 の逆向き（新基盤） |
| 41 | カラー・ウォッシュ | 今すぐ | solid |
| 42 | ブラインダー | 今すぐ（配置のみ） | solid のクリップ。押している間の点灯は発火モデル（新基盤） |
| 43 | 横／縦ビーム | 今すぐ | chase、width 1 |
| 44 | 動くスポット | 今すぐ | orb：radius 1、edge 1、orbit または bounce |
| 45 | 左右交互 | 小拡張 | `regions`（pattern halves_lr、拍または秒でステップ） |
| 46 | 上下交互 | 小拡張 | `regions`（halves_tb） |
| 47 | 市松チェイス | 小拡張 | `regions`（checker） |
| 48 | 外周ランナー | 小拡張 | `regions`（perimeter、1セルずつ28ステップ） |
| 49 | 内→外ファン | 小拡張 | `regions`（fan：中央から両端へ対称） |
| 50 | 4群ステップ | 小拡張 | `regions`（quadrants、order 指定） |
| 51 | 8灯ディマー・ウェーブ | 小拡張 | `regions`（columns、soft 1.0 で明るさの山が伝わる） |
| 52 | ムービングヘッド・ペア | 小拡張 | orb に `orbit_phase`。2枚を phase 0 / 0.5 で対称運動 |
| 53 | 光のスタンプ | 新基盤 | ビットマップ素材 + 発火 |
| 54 | ピクセル・タイポ | 新基盤 | フォントとスクロール |
| 55 | 4→3→2→1 | 新基盤 | ビットマップ列 |
| 56 | 叩くたびに色が進む | 新基盤 | 入力回数の状態 |
| 57 | 光を植える | 新基盤 | パッドごとの状態 |
| 58 | 盤面を塗る | 新基盤 | パッドごとの状態と消去操作 |
| 59 | 音量の景色 | 外部入力 | 音声入力経路（Live からの音量は現状のブリッジにない。Remote Script の output meter か OS の音声取得かを検証） |
| 60 | 演奏の残像ループ | 新基盤 | 入力の記録と反復再生、操作 UI |

集計：今すぐ 26（近似2件を含む）、小拡張 14、新基盤 19、外部入力 1。

## 3. まず入れるもの（小拡張）

優先順。§4 の基盤はこの後に続ける。

1. 既存 Effect への任意項目：ripple `shape`（circle / diamond / square）と `trigger: interval`（origin_x / origin_y / interval_ms）、wave `phase`、orb `orbit_phase`。省略時の描画は不変。
2. `builtin.regions`「領域ステップ」：pattern（halves_lr / halves_tb / checker / quadrants / columns / rows / perimeter / fan）、color、color_b（任意、交互の相手色）、step（beat か seconds）、step_beats または step_ms、order（forward / reverse / pingpong）、soft（0〜1、隣のステップへのクロスフェード）、duty（0.05〜1）。時刻と拍だけで決まる。45〜51 を1つの Effect で賄う。
3. `builtin.tap_glow`「余熱」：入力駆動。color_start / color_mid / color_end、lifetime_ms、radius（0〜3、線形減衰）、repeats（1〜8）、repeat_interval_ms、repeat_decay。05 と 12。
4. `builtin.cross`「十字」：入力駆動。color、speed、length、lifetime_ms。06。

## 3a. 進捗（2026-10-28 夜）

| 段階 | 状態 | コミット |
| --- | --- | --- |
| §3 小拡張（ripple shape / interval、wave phase、orb orbit_phase、regions、tap_glow、cross） | 完了（235件） | 9efb737 |
| §4-1 発火モデル（triggered レイヤー、press / release / hold / clip_start / interval、Schema 0.1.3） | 完了（242件） | 0aba923 |
| §4-2 キーフレーム（Param<T>::Keyframes、ease、最悪値の安全解析） | 完了（249件） | 88f4548 |
| §4-3 領域・経路・図形（regions custom、builtin.path、builtin.shape、shape モジュール） | 完了（257件） | 5724e07 |
| §4-4 マスク（shape の invert / wipe、レイヤーの self マスク。下を隠す遷移は黒図形の over） | 完了（262件） | 43f3d8f |
| §4-5 粒子（builtin.particles：burst / fountain / rocket / implode、解析解で seek 可、bounce は折り返し。最悪負荷 p95 4.02 ms） | 完了（273件） | 80eca4b |
| §4-6 複合素材（show 内 `compounds`、参照 + 複製して独立、tint と time_scale、内部レイヤー4まで） | 完了（280件） | a7b0c5f |
| §4-7 フレーム列・ドット絵・文字（builtin.frames、builtin.text、外部形式の取り込み、粒子の全体上限768） | 完了（291件） | 4ae9123 |
| §4-8 入力状態と記録（builtin.pads / cycle / constellation、クリップの recording、入力リセット、tap_glow の charge） | 完了（302件） | 0138a75 |
| §4-9 音反応（ブリッジ v3 の meter、builtin.meter、`[LED]` トラック） | 完了（Rust 307件 + Python 8件。Windows の Live 12.4.7 で実測：再生中 24.6 Hz、Mac は未確認） | 5f43a36 |
| 固め（回帰 29/29、check-shows、重いshowの10分soak、Liveのmeter実測、panic安全、fuzz 18,000件、転送遅延、本番用雛形とパレット） | 完了（312件） | 5cc4384 |
| 素材バンク（Schema 0.1.4：プリセット・複合素材の `metadata` category / tags / favorite、GUI の素材ライブラリー：検索・カテゴリ・タグ・お気に入り・24コマのサンプル表示・レイヤーに配置）、録音反復の有界化（最新1,024件）とペイント・色順の累積状態の分離、LED プレビューの切り替え文言（実機出力／手元で再生）。Codex 制作、Opus 文言、設計確認 | 完了（Rust 324件、check-shows 18ファイル。実機未確認） | （この commit） |
| 素材の共通ライブラリー（A20）：登録した `.show.json` の素材を一覧し編集中のショーへ複製（`InsertCompound`、同じ内容は重複させない）、端末設定 `library_shows`、ライブラリーファイルは書かない | 完了（327件。GUI・実機未確認） | 25853d3、7e1964c |
| タイムラインの再生操作バー（先頭へ・再生・停止・位置・テンポ・表示元、Space、ルーラーで位置移動、プレビュー状態の集約） | 完了（GUI・実機未確認） | f11c9fc、6b3cec2 |
| 手元の時計（A21）：Runtime の時刻源 `local`（LocalClock で動き、bridge・Clock・Link・MTC を開かない）、`runtime_transport_*`、演奏タブの時刻源選択、操作バーとの接続 | 完了（328件。実機未確認） | d336208、6b3cec2 |

これで多くの案を作るための基盤が揃ったが、60案の納品全体は未完了。39 ピクセル崩壊、29 プラズマの完成素材、30 動画・GIF変換ツール、拍単位のクリップ配置・発火反復、13 の解放時の放射、8テーマのパレットと全60案の成果物・検証一覧が残る。30 のフレーム列再生、13 の保持中の成長と解放後の減衰は実装済み。59 は音量入力で、周波数分析は未対応。Windows の Live では実測済み（Mac は未確認）。Mac 実機での表示確認は未実施。

2026-10-28 Codexレビュー後の修正：録音反復を最新1,024件までの有界処理に変更。ペイント・色順の累積状態を短期履歴から分離。GUI録音は停止時刻までの余白を保存し、ループ長を編集可能にした。保存形式は0.1.3のまま。短周期、60秒経過、履歴上限、リセット、シーク、編集後の保持、録音末尾、Undo/Redo、保存・再読込を回帰テストで確認済み。`cargo test --workspace` 321件、フロントエンドの型検査・ビルド、18ショーの読込・再保存・描画検査が通過。GUI実操作と実機確認、配布版への反映は未実施。

## 4. 基盤（Codex 案を採用、順序案。§3 の後に着手）

1. 発火モデル：押下・解放・保持・クリップ開始・反復。入力駆動 Effect（ripple、tap_glow、cross）を共通の発火源に載せ替える。押下中の点灯（42）と長押し（13）。
2. キーフレーム：`Param<T>` の Keyframes variant（[12-original-requirements.md](12-original-requirements.md) で確保済み）。35、36 の連続変化。
3. 領域・経路・図形：regions の一般化（任意セル集合、位相差）、経路（直線・外周・周回・8の字・点列）、図形（線・円・四角・ひし形、塗りと輪郭）。
4. マスクとトランジション：レイヤーに時間変化するマスク。37、38。
5. 粒子：発生・速度・寿命・重力・反射・上限。seek 可能にするため「発生時刻から決定的に位置を計算」する設計（rain と同じ考え方）。09、10、11、39、40。
6. 複合素材：複数レイヤーと時間差を1素材として保存・配置・複製。外側から色や速さを差し替え。参照かコピーかを明示。
7. フレーム列・ドット絵・文字：8×8 のフレーム列、フォント、スクロール、動画や GIF からの変換ツール（依存は別途）。30、53、54、55。
8. 入力状態と記録：パッドごとの蓄積・減衰、入力回数、直前の点、記録と反復再生、リセットと上限。14、15、56、57、58、60。
9. 音反応：入力経路の検証（Live の Remote Script から output meter を送る案が第一候補。周波数帯域はその経路では取れないので、取れるものだけを明示）。59。

各段階で Schema を上げ、旧ファイルは読めることをテストで保証する。負荷の上限（同時イベント、粒子数、素材サイズ、内部レイヤー数）は粒子と複合素材の段階で決める。

## 5. パレット用ショー（Codex の制作範囲）

テーマ：水底、溶鉱炉、ネオン都市、月面、機械室、無彩色、花とインク、OMOI-IRO。各テーマに背景・タップ・アクセント・終了演出を「今すぐ」の26案から選んで構成する。作り方と確認方法は [13-preset-authoring.md](13-preset-authoring.md)。小拡張が入ったら対応表の番号で追加する。
