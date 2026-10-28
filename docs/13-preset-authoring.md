# プリセット制作ガイド（Codex や他の制作者向け）

OMOI-IRO のショーデータ（`.show.json`）に入れる演出プリセットを、アプリの GUI を使わずに JSON で量産するための手引き。制作物はデータだけで、アプリのコードには触れない。複数人や複数の AI が並行して作っても衝突しない。

対象の Schema は 0.1.2（`schemas/show.schema.json`）。この文書と食い違ったら Schema が正しい。

## 1. 納品の形

- 1テーマにつき1つの「パレット用ショー」を作る。ファイル名は `shows/palette-<テーマ>.show.json`（例：`palette-blue-rain.show.json`）。
- パレット用ショーには、プリセットと、それを見るためのクリップを入れる。クリップは4秒ずつ順番に並べ、1つのレイヤーに置く。常時系（Pulse、Noise、Ripple）は continuous レイヤーでもよい。
- 本番側では、このショーを開いてプリセットを選び直すか、プリセット欄の「読み込み」で `.preset.json` を取り込む。`.preset.json` も出すなら `shows/presets/<id>.preset.json`。

テンプレートは `fixtures/shows/effects-0.1.2.show.json`。これを複製して `id`、`name`、`presets`、`layers` を書き換えるのが最短。

## 2. 盤面と色の前提

- 盤面は Launchpad X の 8×8。LED index = y × 8 + x、原点は左下。
- 色は `#RRGGBB` の sRGB。実機は各チャンネル 7 bit に量子化されるので、`#101010` のような暗い色はほぼ消える。地の色は `#0D2346` 程度が下限の目安。
- 明るさの調整は色でなくレイヤーの `opacity` でやる。白を `opacity` 0.3〜0.5 で `add` 合成すると、下の色を保ったまま強調できる。
- 合成は `layers` の配列順。後ろが上。`blend` は `over`（上書き）、`add`（加算）、`max`（明るい方）。
- 点滅の安全上限は端末設定で既定 3 Hz。これを超えるプリセットは警告が出て、本番側が上限を上げない限り消灯されうる。速い点滅は「本番で意図して上限を上げる前提」と名前に書く（例：`strobe-fast-8hz`）。

## 3. 演出の種類と params（Schema 0.1.2）

すべて `effect: { "type": ..., "version": 1, "params": { ... } }`。`params` は必須項目をすべて書き、知らないキーは入れない。

| type | 名前 | 必須 params | 意味 |
| --- | --- | --- | --- |
| `builtin.solid` | 単色 | `color` | 全面を1色 |
| `builtin.chase` | 流れ | `color`, `period_us`（1周の時間、µs）, `width`（1〜8）, `axis`（`x` 横 / `y` 縦） | 幅 width の帯が1周する。任意 `reverse: true` で逆向き（右→左、上→下） |
| `builtin.pulse` | 拍の明滅 | `color`, `beats_per_cycle`（0.25 / 0.5 / 1）, `depth`（0〜1）, `waveform`（`sine` / `triangle`） | 拍の位相で明るさが変わる。拍の頭で最大。拍が取れないと透明 |
| `builtin.strobe` | 点滅 | `color`, `mode`（`hz` / `beat`）, `rate_hz`（0.5〜30）, `beats_per_cycle`（0.25 / 0.5 / 1 / 2）, `duty`（0.05〜0.95）, `phase_offset`（0〜1） | 矩形波。`hz` はクリップのローカル時刻、`beat` は拍の位相。使わない方の値も必須なので書く |
| `builtin.noise` | ノイズ | `base_color`, `highlight_color`, `density`（0〜1）, `rate_hz`（0.5〜30）, `seed`（0〜2147483647）, `smooth`（bool）, `depth`（0〜1） | 地の色の上に粒が散る。density は点灯割合、rate_hz は模様の更新頻度、depth は粒の明るさの揺らぎ。時刻だけで決まる |
| `builtin.ripple` | 波紋 | `color`, `speed`（セル/秒 0.5〜16）, `ring_width`（0.5〜4）, `lifetime_ms`（200〜4000）, `fade`（`linear` / `exp`）, `max_concurrent`（1〜16）, `trigger`（`any_pad`） | パッドを押した位置から輪が広がる（最新16件、4秒で破棄、重なりは最大値）。入力駆動なので CLI の `render` では見えない。GUI のプレビュー盤をクリックすると試せる |

予定（実装後に Schema を見て追記）：`builtin.rain`（雨）、`builtin.wave`（波形）、`builtin.orb`（球）。

## 4. 書き方の規則

- `id` は `^[a-z][a-z0-9_-]{0,63}$`。英小文字で始め、英数字とハイフン・下線。ショー内で一意。プリセットの id は `<種類>-<特徴>`（例：`noise-blue-dense`、`strobe-beat-white`）。
- `name` は日本語でよい（128文字まで）。GUI に出る名前。
- 時間はすべて µs の整数（1秒 = 1000000）。
- `clip` は `id`、`preset_id`、`start_us`、`duration_us`、`fade_in_us`、`fade_out_us` をすべて書く。
- `layer` は `id`、`name`、`target_id`（`main-pad`）、`enabled`、`opacity`、`blend`、`content` をすべて書く。
- 同じ配色の変種は別プリセットにする（GUI で差し替えやすい）。

## 5. 確認の仕方（CLI）

機器なしで描画結果を文字で出せる。Windows の実行ファイルは `target\release\omoiiro-probe.exe`（最新に作り直した場所を確認すること）。

```powershell
# 2.0 秒の時点、拍位置 4.0（拍の頭）で描画。色が 8×8 で出る
.\target\release\omoiiro-probe.exe render --show shows\palette-blue.show.json --at 2000000 --beat 4.0

# 拍の裏（Pulse や beat モードの Strobe の消える側）
.\target\release\omoiiro-probe.exe render --show shows\palette-blue.show.json --at 2000000 --beat 4.5

# JSON で出す（別のツールで画像化するとき）
.\target\release\omoiiro-probe.exe render --show shows\palette-blue.show.json --at 2000000 --beat 4.0 --json
```

- 読み込み時に Schema と整合（ID 参照、時間区間、範囲）を検査する。エラーは JSON Pointer つきで出るので、その場所を直す。
- `--beat` を付けないと拍は不明として扱われ、Pulse と beat モードの Strobe は透明になる。
- 1プリセットにつき、少なくとも 3 時点（クリップの始まり、途中、終わり際）を見る。流れ系は 1 周期の 1/4 ごとに見ると向きが分かる。

## 6. テーマの例（依頼の出し方）

依頼は「テーマ、色、使う種類、何個」で出す。例：

- 「青の雨と水面」：base `#0D2346`、highlight `#9FE8FF`〜`#FFFFFF`。Noise を密度 0.15 / 0.3 / 0.5 で3つ、Chase 縦（上→下、`reverse: true`）を速さ3段階、Pulse 白 depth 1 を1つ。
- 「赤のテーマ（OMOI-IRO）」：`#FF2A2A` 系。Solid 2段階、Strobe beat 1拍 duty 0.25、Noise 赤×橙、Chase 横を2速度。
- 「Matrix」：Rain 実装後。緑 `#00C040`、先頭 `#B0FFB0`。

## 7. やってはいけないこと

- `schema_version` を変えない（0.1.2）。
- `sync`、`render`、`targets` はテンプレートのまま。
- プリセットの `params` に Schema にないキーを足さない（読み込みで拒否される）。
- 点滅の速いプリセットを「常時レイヤー」に置かない（本番で開いた瞬間から点滅する）。
