# プリセット制作ガイド（Codex や他の制作者向け）

OMOI-IRO のショーデータ（`.show.json`）に入れる演出プリセットを、アプリの GUI を使わずに JSON で量産するための手引き。制作物はデータだけで、アプリのコードには触れない。複数人や複数の AI が並行して作っても衝突しない。

対象の Schema は 0.1.3（`schemas/show.schema.json`、2026-10-28 夜時点）。0.1.2 以前のファイルもそのまま読める。この文書と食い違ったら Schema が正しい。

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

## 3. 演出の種類と params（Schema 0.1.3）

すべて `effect: { "type": ..., "version": 1, "params": { ... } }`。`params` は必須項目をすべて書き、知らないキーは入れない。

| type | 名前 | 必須 params | 意味 |
| --- | --- | --- | --- |
| `builtin.solid` | 単色 | `color` | 全面を1色 |
| `builtin.chase` | 流れ | `color`, `period_us`（1周の時間、µs）, `width`（1〜8）, `axis`（`x` 横 / `y` 縦） | 幅 width の帯が1周する。任意 `reverse: true` で逆向き（右→左、上→下） |
| `builtin.pulse` | 拍の明滅 | `color`, `beats_per_cycle`（0.25 / 0.5 / 1）, `depth`（0〜1）, `waveform`（`sine` / `triangle`） | 拍の位相で明るさが変わる。拍の頭で最大。拍が取れないと透明 |
| `builtin.strobe` | 点滅 | `color`, `mode`（`hz` / `beat`）, `rate_hz`（0.5〜30）, `beats_per_cycle`（0.25 / 0.5 / 1 / 2）, `duty`（0.05〜0.95）, `phase_offset`（0〜1） | 矩形波。`hz` はクリップのローカル時刻、`beat` は拍の位相。使わない方の値も必須なので書く |
| `builtin.noise` | ノイズ | `base_color`, `highlight_color`, `density`（0〜1）, `rate_hz`（0.5〜30）, `seed`（0〜2147483647）, `smooth`（bool）, `depth`（0〜1） | 地の色の上に粒が散る。density は点灯割合、rate_hz は模様の更新頻度、depth は粒の明るさの揺らぎ。時刻だけで決まる |
| `builtin.ripple` | 波紋 | `color`, `speed`（セル/秒 0.5〜16）, `ring_width`（0.5〜4）, `lifetime_ms`（200〜4000）, `fade`（`linear` / `exp`）, `max_concurrent`（1〜16）, `trigger`（`any_pad` / `interval`）。任意 `shape`（`circle` / `diamond` / `square`）。`interval` のとき `origin_x` / `origin_y`（0〜7）と `interval_ms`（100〜10000）が必須 | パッドを押した位置から輪が広がる（最新16件、4秒で破棄、重なりは最大値）。入力駆動なので CLI の `render` では見えない。GUI のプレビュー盤をクリックすると試せる |

| `builtin.rain` | 雨 | `color`, `direction`（`down` / `up` / `left` / `right`）, `speed`（セル/秒 0.5〜16）, `density`（0〜1）, `tail`（0〜8）, `seed`, `flicker`（0〜1）。任意 `head_color` | 一方向に粒が流れ、尾を引く。0.5秒ごとの枠に確率 density で粒が出る。先頭は head_color（省略時は color）、尾は線形に減衰。flicker は尾を 8 Hz で揺らす。時刻だけで決まる。Matrix 風は緑 `#00C040` + 先頭 `#C8FFD8`、tail 4〜6、flicker 0.5 |
| `builtin.wave` | 波形 | `color`, `axis`（`x` 横に流れる / `y` 縦）, `wavelength`（2〜16）, `amplitude`（0.5〜3.5）, `center`（0〜7）, `speed`（周期/秒 −4〜4、負で逆向き）, `thickness`（0.5〜3）, `beat_amplitude`（bool）。任意 `phase`（0〜1、2枚重ねて交差させるときに 0.5） | 正弦波の曲線に近いセルが光る。beat_amplitude を true にすると拍の頭で振幅が最大、裏で 0.3 倍（拍不明なら固定） |
| `builtin.orb` | 球 | `color`, `center_x` / `center_y`（0〜7、小数可）, `radius`（0.5〜6）, `edge`（0〜2）, `light_angle`（0〜360、90 = 上から）, `shading`（0〜1）, `dither`（0〜1）, `dither_rate_hz`（0.5〜15）, `seed`, `motion`（`none` / `orbit` / `bounce`）, `speed`（orbit は 0.05〜2 周/秒、bounce は 0.5〜12 セル/秒、none は 0 でよい）, `orbit_radius`（0〜4）, `bounce_angle`（0〜360）。任意 `orbit_phase`（0〜1、2つの球を対称に回すときに 0.5） | 陰影つきの円。dither で粒状のざらつき。orbit は中心の周りを回り、bounce は盤面の端で反射。すべて時刻だけで決まる |

| `builtin.regions` | 領域ステップ | `pattern`（`halves_lr` 左右半面 / `halves_tb` 上下半面 / `checker` 市松 / `quadrants` 4象限 / `columns` 8列 / `rows` 8行 / `perimeter` 外周28セル / `fan` 中央から外へ4段）, `color`, `step_mode`（`beat` / `seconds`）, `step_beats`（0.25 / 0.5 / 1 / 2 / 4）, `step_ms`（50〜5000）, `order`（`forward` / `reverse` / `pingpong`）, `soft`（0〜1）, `duty`（0.05〜1）。任意 `color_b`（点いていない群の色） | 照明卓のチェイス。群が順に点く。soft 1 で明るさの山が滑らかに伝わる（8灯ディマー）。beat は拍不明なら透明。使わない方の step 値も必須なので書く |
| `builtin.tap_glow` | 余熱 | `color_start`, `color_mid`, `color_end`, `lifetime_ms`（100〜5000）, `radius`（0〜3）, `repeats`（1〜8）, `repeat_interval_ms`（50〜2000）, `repeat_decay`（0〜1） | 押した位置が start → mid → end と色を変えて消える。repeats でエコー。入力駆動（CLI の render では見えない） |
| `builtin.cross` | 十字 | `color`, `speed`（セル/秒 1〜32）, `length`（尾 0.5〜8）, `lifetime_ms`（100〜3000） | 押した位置から上下左右へ光が走る。入力駆動 |

| `builtin.path` | 経路 | `color`, `path`（`line` / `perimeter` / `orbit` / `figure8` / `points`）, `speed`, `size`（光点の半径 0.5〜3）, `tail`（0〜8）, `loop`（bool）, `phase`（0〜1）, `mirror`（`none` / `x` / `y` / `xy`）。`line` は `from_x` / `from_y` / `to_x` / `to_y`、`orbit` と `figure8` は `center_x` / `center_y` / `orbit_radius`、`points` は `points`（[[x,y],…] 2〜32点） | 光点が経路に沿って動く。閉じた経路（perimeter / orbit / figure8）の speed は周/秒（0.05〜4）、line / points はセル/秒（0.5〜16）。loop false は終点で止まり、true は往復か周回。mirror で対称の分身（ムービングヘッドのペア） |
| `builtin.shape` | 図形 | `shape`（`line` / `circle` / `rect` / `diamond` / `wipe`）, `color`, `fill`（bool、false で輪郭）, `stroke`（0.5〜3）, `center_x` / `center_y`（−4〜11、盤の外も可）, `size`（半径または半辺 0〜6）, `angle`（0〜360）, `edge`（0〜2）。任意 `size_y`（rect の縦）、`invert`（bool、図形の外側を塗る） | 静止した形。拡大・移動・回転はキーフレームで作る。`wipe` は半平面（angle 0 で center から右側）。**下を隠す遷移は黒い図形を `over` で重ねる**：アイリス = 黒 circle + invert + size 0→6、シャッター = 黒 wipe 2枚（angle 0 / 180、center を中央から外へ）または黒 rect の size 4→0 |

レイヤーには任意の `mask` を付けられる（そのレイヤー自身の描画だけを切り抜く）：`{ "shape": "circle" | "rect" | "diamond" | "wipe", "center_x", "center_y", "size", "size_y"?, "angle"?, "edge", "invert" }`。全項目キーフレーム可で、時間の基準はそのレイヤーの content と同じ。例：雨を円の中だけに出す（`fixtures/shows/transitions-0.1.3.show.json` の冒頭）。

`builtin.regions` の `pattern` に `custom` を指定すると `groups`（[[セル番号,…],…] 1〜32群、セル番号 = 行×8+列）が必須になり、任意の `offset`（群ごとの位相差 0〜1、群数と同じ長さ）が使える。

すべての種類の試験用ショー：`fixtures/shows/effects-0.1.2.show.json`（点滅・ノイズ）、`ripple-0.1.2.show.json`（波紋）、`rain-wave-orb-0.1.2.show.json`（雨・波形・球、10秒ずつ）、`d-effects-0.1.2.show.json`（領域ステップ・余熱・十字・形つき波紋）、`triggered-0.1.3.show.json`（発火レイヤー）、`keyframes-0.1.3.show.json`（粒から豪雨へ、加速する走査）、`paths-shapes-0.1.3.show.json`（外周ランナー2種、ムービングヘッド対、ひし形の拡大、自由グループ）、`transitions-0.1.3.show.json`（円の中の雨、アイリス、シャッター2種）。

## 3b. 発火レイヤーとキーフレーム（0.1.3）

**発火レイヤー**：`content` の `kind` を `triggered` にすると、クリップの区間内だけ入力や時刻に反応する。

```json
"content": {
  "kind": "triggered",
  "trigger": { "on": "press" },
  "clips": [ { "id": "c1", "preset_id": "drop-blue", "start_us": 0, "duration_us": 600000000, "fade_in_us": 0, "fade_out_us": 0 } ]
}
```

- `on`：`press`（押した瞬間）、`release`（離した瞬間）、`hold`（押している間だけ）、`clip_start`（クリップの開始時刻）、`interval`（クリップ内で `interval_ms` ごと。`origin_x` / `origin_y` で位置、既定 3.5）。
- 位置を受ける演出（ripple / tap_glow / cross）は押した場所から出る。それ以外の演出は `hold` のときだけ使え、押している間だけ全面に出る（ブラインダー）。
- 押下は巻き戻し（seek）では再現しない。消えもしない（寿命まで残る）。
- 常時反応させたいときは 0 から十分長いクリップを1つ置く。

**キーフレーム**：数値と色の params は、値の代わりに次の形を書ける。

```json
"density": { "keyframes": [ { "t_us": 0, "v": 0.05 }, { "t_us": 16000000, "v": 1, "ease": "linear" } ] }
```

- `t_us` は昇順、1〜64 点。最初の前は最初の値、最後の後は最後の値。
- `ease` はその点から次の点への補間：`linear`（既定）、`step`、`ease_in`、`ease_out`、`ease_in_out`。
- 時間の基準：timeline / triggered のクリップはクリップのローカル時刻、continuous は show 時刻、押下から出る演出（ripple など）は発火からの経過。
- 個数の整数（seed、repeats、max_concurrent、chase の width）と enum と bool は `step` だけ。長さの整数（period_us、lifetime_ms など）は補間される。
- 安全解析はキーフレームの最悪値（最大の rate など）で判定する。

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
- 「Matrix」：Rain 下向き、緑 `#00C040`、先頭 `#C8FFD8`、speed 6〜10、density 0.3〜0.5、tail 4〜6、flicker 0.5。

## 7. やってはいけないこと

- `schema_version` は雛形のまま（0.1.2 か 0.1.3。どちらも読める）。
- `sync`、`render`、`targets` はテンプレートのまま。
- プリセットの `params` に Schema にないキーを足さない（読み込みで拒否される）。
- 点滅の速いプリセットを「常時レイヤー」に置かない（本番で開いた瞬間から点滅する）。
