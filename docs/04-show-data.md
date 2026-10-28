# OMOI-IRO ショーデータ仕様案

## 保存形式

MVPはUTF-8のJSONファイル1つをショーの単位とする。拡張子は`.show.json`。外部assetや音声を含めないため、ZIPやデータベースを導入せずGitで差分を確認できる。レビュー用の[Schema](../schemas/show.schema.json)と[サンプル](../examples/demo.show.json)を添付する。

JSON SchemaはDraft 2020-12。`schema_version: "0.1.0"`は設計案の版であり、公開互換性を約束する1.0ではない。未知のフィールドやEffectは黙って無視せず、読込エラーとして位置と理由を示す。

## 最上位のフィールド

| フィールド | 意味 |
| --- | --- |
| `$schema` | ローカルSchemaへの相対参照 |
| `schema_version` | データ構造の版 |
| `id` / `name` | 固定のshow IDと表示名 |
| `seed` | 将来の確定的な乱数効果にも使える32 bit seed |
| `sync` | MTCのfps・原点・offset、拍源、停止・同期断設定 |
| `render` | LED更新目標fps、master brightness |
| `preview` | ローカル再生用のBPM。外部追従時には使わない |
| `targets` | 論理的な機器とsurface profile |
| `presets` | ショー内に埋め込んだEffect定義 |
| `layers` | 合成順を持つ演出レイヤー |

## 時間と同期

配置時刻は整数μs。`start_us: 1000000`はショー起点の1秒後。durationは正数、fade時間は非負数。開始点を含み終了点を含まない。fpsを変えても秒単位のclipを勝手に伸縮しない。

MTCの`origin`は`HH:MM:SS:FF`のラベル。`frame_rate`は`24`、`25`、`29.97df`、`30`の列挙値とする。利用者の暫定候補は30 fps。サンプルは30 NDF、`01:00:00:00`を起点にしているが、正式なrateと起点は未確定。映像側との共有が必要なら同じfpsと起点へ合わせる。タイムコードのfpsとLED更新のfpsは独立した設定とする。

DFではラベル検証に追加の意味検証を行う。例として`00:01:00:00`と`00:01:00:01`は29.97 DFで無効、`00:10:00:00`は有効とする。内部はラベルから連続frame indexへ変換してから有理数で秒へ換算する。JSON Schemaの正規表現だけではこの条件を判定しない。

`input_offset_us`の正値は演出を先行させる。`holdover_ms`は最終正常受信から補間できる上限、`blackout_after_ms`は同じ最終正常受信から消灯までの時間。後者は前者より大きいことを意味検証する。正常なStopの`stop_behavior`は同期断と別に適用する。

MVPの`beat_source`は`ableton_link`または`midi_clock`。推奨サンプルは`ableton_link`。Link Adapterの初期規約は四分音符位相用quantum 1。Start Stop Syncはアプリ側で有効にしない（2026-10-28改訂、S07でLive 12.4.7のトランスポートが進まなくなる問題。再生状態はブリッジの`is_playing`かClockから取る）。拍入力未確定時に`preview.bpm`や別の拍入力へ自動fallbackさせない。拍位相が確定しないPulseは透明、または既に有効だったフレームを同期断規則に沿って保持する。

2026-10-28の改訂：共通時計の源はLive内スクリプトのブリッジ（[10-live-bridge.md](10-live-bridge.md)）になり、MTCはアプリが生成する出力になった。`sync`の`origin`と`frame_rate`（30 NDF確定、起点はshowごとに自由）はMTC出力のラベルに使う。Schema 0.1.0の`sync.timecode.source`（`mtc`）は、M3で`live_bridge`を既定値として追加する案を実装担当が出し、設計担当が承認してから変更する。Linkのbeatを共通タイムコードに読み替えない点は変わらない。

## Targetと座標

targetは`id`、`name`、`profile`からなる。profile初期値は`novation.launchpad_x.grid64.v1`。show内にOSのMIDIポート名・USB接続番号・個体のconnection IDを保存しない。

surfaceは左下原点のLEDリストとしてCoreへ読み込む。汎用的な機器能力をAdapterが持ち、showはそのprofileを参照する。MVP validatorはtarget 1台だけを受け付ける。複数台へ広げるときにSchemaと実行予算を同時に更新する。

## PresetとEffect

各presetは固定ID、表示名、Effectのtype/version/paramsを持つ。Effect versionはパラメーターの意味を変えるときに上げる。MVPの定義は以下。

| type | params | 評価 |
| --- | --- | --- |
| `builtin.solid` v1 | `color` | 全面の定色 |
| `builtin.chase` v1（0.1.2で任意の `reverse`） | `color`, `period_us`, `width`, `axis` | local timeからxまたはyの位置を計算。帯以外は透明 |
| `builtin.pulse` v1 | `color`, `beats_per_cycle`, `depth`, `waveform` | 四分音符位相から色の寄与を変化 |
| `builtin.strobe` v1（0.1.2） | `color`, `mode`（hz / beat）, `rate_hz`, `beats_per_cycle`, `duty`, `phase_offset` | 矩形波の点滅。hzはクリップのローカル時刻、beatは拍位相（不明なら透明）。安全解析の対象 |
| `builtin.noise` v1（0.1.2） | `base_color`, `highlight_color`, `density`, `rate_hz`, `seed`, `smooth`, `depth` | hash(seed, cell, step)で決まる粒。時刻だけで決まる。安全解析は rate_hz/2 |
| `builtin.ripple` v1（0.1.2） | `color`, `speed`, `ring_width`, `lifetime_ms`, `fade`, `max_concurrent`, `trigger` | 操作入力（パッド押下）を起点に輪が広がる。render(show, t)の純粋性の例外：`render_input(t, beat, Option<&PadInput>)` で入力状態を受ける |

colorは`#RRGGBB`のsRGB、alphaはfadeとopacityで表す。Chaseの`period_us`は帯が1周する時間、widthは1〜8セル。PulseはMVPで0.25/0.5/1拍周期だけに限定し、不明な小節位置から長周期を推定しない。depth 0は常時最大、depth 1は周期内でゼロまで下がる。PulseはRGB色を固定しalphaを変化させる規約とし、位相0で最大となる。

Presetはshow内へコピーして保存する。ユーザーのライブラリーのpresetを後から編集しても既存showは変わらない。show内のpresetを複数clipが共有するとき、clip単独の編集は新IDのpresetへ複製して差し替える。明示的な共有編集だけが複数clipへ波及する。

Preset単体のexportは別ファイルにschema versionとpreset本体を持たせる案とする。今回添付したSchemaはshow全体用であり、presetライブラリー用Schemaは実装タスクで別途確定する。

## LayerとClip

layerはtarget、enabled、opacity、blend、contentを持つ。配列順が下から上の合成順となる。`content.kind`で2種類を区別する。

- `timeline`：clipsを持ち、現在時刻に有効なclipだけを評価する。
- `continuous`：preset IDを1つ持ち、show時刻が非負である限り有効。常時の拍反応レイヤーに使用する。

clipはid、preset ID、start/duration、fade in/outを持つ。同じレイヤー内で重なる場合は配列順にover合成する。clip終端を過ぎた値やseek前の値を残さない。continuousのlocal timeはshow time、timelineのlocal timeはshow timeからstartを引いた値とする。

MVPは全パッドを対象とする。パッドmask、パラメーターautomation、ノードグラフ、拍単位clipは後続Schemaで追加し、未定の汎用`config`へ押し込まない。

## 実装で確定した評価規約（M2、2026-10-28）

- Chase：帯は先頭セルから後ろへ`width`セル（先頭、先頭−1、…）。8セルを1周`period_us`で進み、端で折り返す。`axis`がxなら列全体、yなら行全体を点灯する。
- Pulse：`phase = frac(beat / beats_per_cycle)`。sineは`0.5(1+cos 2πφ)`、triangleは`|1−2φ|`、`alpha = 1 − depth(1−v)`。拍が不明（BeatStateなし、または位相無効）なら透明。
- 合成：layer内のclipはpremultiplied alphaでover。layer間はover `F(1−a·o)+L·o`、add `min(1, F+L·o)`、max `max(F, L·o)`（oはlayer opacity）。最後に`master_brightness`を掛ける。色はsRGB値を0..1でそのまま扱い、ガンマと機器校正はデバイス層（M4）で行う。
- IDの名前空間は種別ごと（target、preset、layer）。clip IDはshow全体で一意。layer IDとclip IDが同じ値でもエラーにしない。
- `stop_behavior`、`loss`、`recovery_fade_ms`、`input_offset_us`、`render.fps`、`seed`はCoreでは検証のみ。意味づけはRuntime（M3 / M4）。
- 拡張の余地（U04、2026-10-28）：Effectのparamsは内部では `Param<T>`（今は値のみ）で、Schema 0.2でキーフレーム列を足す。Effectの `type` は `builtin.*` に加えて将来 `graph`（グラフ参照）を足す。clip単位のparams上書きも0.2で `clip.params` として追加する。いずれも0.1.xの保存形式は変えない。
- Preset単体ファイルは`{"schema_version": "0.1.0", "kind": "djsc.preset", "preset": {...}}`。現在はshow Schemaの`$defs/preset`で検証し、専用の`schemas/preset.schema.json`はM3のSchema改訂（`sync.timecode.source`に`live_bridge`追加、oneOfをkind分岐のif/thenにしてエラーのPointerを深くする）と一緒に作る。

## 端末設定との分離

アプリのユーザー設定ディレクトリーに、以下を別保存する。showと一緒にGit管理しない。

- 論理target IDと物理ポートのbinding、ポート役割、backend選択。
- 機器ごとの遅延補正と色校正、最後の接続結果。
- ウィンドウ状態、最近使ったファイル、ログ設定。

同期方式・原点は作品に必要なのでshowへ保存する。実際のポート選択は環境に依存するので端末設定へ分ける。showを別OSへ移しても接続設定を再指定すれば同じ演出を開ける。

## 二段階の検証

Schemaは型、範囲、必須項目、Effectごとのparams、未知キーを検証する。次の制約はRustの意味検証で行う。

1. show内の対象種別ごとのID一意性、clip IDはshow全体で一意。
2. target/preset参照が存在し、必要な機器能力に対応する。
3. `start + duration`が安全な整数範囲内、fade inとoutの合計がduration以下。
4. fpsごとのframe値域とDFラベル、offset換算のoverflow。
5. holdoverよりblackout期限が長いこと。
6. 全体サイズ、レイヤー数、clip数、文字列長等の資源上限。Schema案ではtarget 1、layer最大8、preset最大256、各timeline最大4096 clips。

無効ファイルを開いても現在のshowを置き換えない。エラーにはJSON Pointerと理由を付ける。将来版は読取拒否または明示的なread-only表示とし、自動で古い版として保存し直さない。

## Git差分と移行

2スペースのインデント、LF、末尾改行、固定キー順で保存する。IDは生成後に維持し、配列順が意味を持つlayers/clipsは保持する。編集日時等を毎回書き換えず、無変更saveは同じ内容になるようにする。

読込→version判定→明示migration→現行Schemaでの構造検証→意味検証→実行モデル構築の順に処理する（2026-10-28改訂。旧版の文書は版を書き換えれば現行Schemaで有効なため、旧Schemaを同梱せずに済む。移行できない版は構造検証の前に拒否する）。Schema 0.1.1（`$id` は `https://omoi-iro.invalid/schemas/show.schema.json`。`.invalid` は取得されない予約TLDで、preset Schemaからの参照解決の鍵）：`sync.timecode.source` は `live_bridge`（既定）/`mtc`、`sync.mtc_output.enabled`、content/effectはkind/typeで分岐するif/then、`schemas/preset.schema.json`（`kind` は `omoiiro.preset`）。0.1.0は版の書き換えだけで移行する（`source` は維持、`mtc_output` は追加しない）。移行は元ファイルを保全し、利用者が保存したときに新形式へ書く。saveは同一ディレクトリーの一時ファイルへ書き、flush後にOS対応の置換を行う。失敗時は元ファイルを残す。

## 将来の再現性

タイムラインの秒・固定seedからの再現性と、生演奏の再現性は別契約。Link/MIDI Clock、手動override、外部triggerに依存した演奏を完全に再演する場合は、時刻付き入力ログとテンポアンカーを別assetとして追加する。現行Schemaだけで未記録の過去入力を再現できるとは扱わない。
