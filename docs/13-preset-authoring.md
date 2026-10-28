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

| `builtin.particles` | 粒子 | `mode`（`burst` 一度に count 個 / `fountain` 毎秒 rate 個 / `rocket` 上昇して burst_height で破裂 / `implode` 円周から中心へ集まり pause_ms 後に破裂）, `color`, `speed_min` / `speed_max`（セル/秒 0〜16）, `angle`（度、0 = 右、90 = 上）, `spread`（度 0〜360）, `gravity`（セル/秒² −32〜32、負で下）, `drag`（0〜8）, `lifetime_ms`（100〜6000）, `size`（0.5〜2）, `tail`（0〜4）, `edge_mode`（`vanish` 消える / `bounce` 跳ね返る）, `seed`。任意 `count`（1〜64、既定 16）, `rate`（0.5〜64、既定 8）, `color_end`（寿命で補間）, `origin_x` / `origin_y`（時刻駆動の発生源、既定 3.5）, `rise_speed`（rocket、既定 10）, `burst_height`（rocket、既定 5）, `pause_ms`（implode、既定 200）, `implode_radius`（1〜6、既定 4） | 火花 = press + burst + gravity −12 + drag 2。花火 = press + rocket（押した列の下から）。しずく = burst count 1 + gravity + bounce。吸い込み→爆発 = implode。timeline はクリップ開始で1回（fountain はクリップの間ずっと）、continuous は繰り返し、triggered は押した位置から（hold で fountain は押している間）。1レイヤーの上限は同時 256 粒・16 イベント。発生時刻からの解析解なので seek しても同じ |

| `builtin.frames` | フレーム列 | `frames`（1 フレーム = 64 セル × `RRGGBB` の 384 桁 hex、上の行から左から右、1〜240 枚）, `fps`（1〜30）, `loop`, `hold_last`（loop でないとき最後を保持）, `hold_ms`（0〜60000）, `anchor`（`board` / `pad`：押したパッドにフレームの (3,3) を合わせる）, `opacity_key`（黒を透明に、既定 true）。任意 `color`（2値フレーム用：点灯セルをこの色に） | ドット絵とアニメ。キーフレーム不可。手で書くより、外部形式（`omoiiro.frames` / `omoiiro.bitmap`、[prompts/codex-assets.md](../prompts/codex-assets.md) に定義）を GUI の「フレーム列を読み込み…」か `omoiiro-probe import-frames --show <show> --file <asset>` で取り込む。triggered press + anchor pad でスタンプ（53） |
| `builtin.text` | 文字 | `text`（ASCII 1〜64）, `font`（`5x7` / `3x5`）, `color`, `mode`（`static` / `scroll_left` / `scroll_up` / `sequential`）, `speed`（scroll はセル/秒 0.5〜16、sequential は文字/秒 0.2〜8）, `gap`（0〜2）, `beat_step`（sequential を拍ごとに）, `loop` | 内蔵フォント。3×5 は小文字を大文字で。54 スクロール、55 カウントダウン（"4321" sequential + 白の solid クリップ） |

| `builtin.pads` | パッドの状態 | `mode`（`heat` 叩くほど溜まる / `plant` 押した所が育って消える / `paint` 色を置く）。heat：`color_low` / `color_high`, `gain`（0.05〜1）, `decay_ms`。plant：`color`, `attack_ms` / `sustain_ms` / `release_ms`, `size`。paint：`color`, `palette`（任意 2〜8 色、同じパッドを叩くたびに次の色）, `decay_ms`（0 で残り続ける） | 入力履歴（最後のリセット以降、60 秒・1,024 件）から毎フレーム計算。クリップの区間内の押下だけを数え、クリップ開始で空から。巻き戻しでは再現しない。15 / 57 / 58 |
| `builtin.cycle` | 叩くたびに色 | `palette`（2〜8 色）, `scope`（`global` / `per_pad`）, `lifetime_ms`（0 で次の押下まで）, `radius`（0〜3） | 色 = palette[回数 mod 長さ]。56 |
| `builtin.constellation` | 星座 | `color`, `stroke`（0.5〜2）, `lifetime_ms`, `fade`（`linear` / `exp`）, `max_segments`（1〜16）, `dots`（bool） | 直前の押下と今回の押下を線で結ぶ。14 |

| `builtin.meter` | 音量 | `source`（`master` / `track`）, `track_name`（track のとき必須。Live のトラック名から `[LED]` を除いたもの）, `channel`（`mix` / `left` / `right`）, `mode`（`bar` 縦棒 / `glow` 全面の明るさ / `orb` 中央の球）, `color_low` / `color_high`, `gain`（0.5〜8）, `floor`（0〜0.5）, `attack_ms`（0〜500）, `release_ms`（0〜2000）, `columns`（bar、使う列）, `peak_hold_ms`（bar、0〜2000） | Live 内のブリッジが送る音量メーター（マスターと、名前が `[LED]` で始まるトラック最大4本）に反応。**取れるのは音量だけで周波数帯域は取れない**。キックだけに反応させたいときは Live でそのトラック（やバス）の名前の頭に `[LED]` を付け、`track_name` にその名前を書く。音が届かないときは透明。continuous と timeline で使える（素材の中と triggered では描かない）。59 |

`builtin.tap_glow` に `charge`（bool）を付けると、`hold` の発火で押している間に半径と明るさが育ち、離すと届いた大きさで光って冷める（13 長押しチャージ）。

**記録（60 演奏の残像ループ）**：triggered レイヤーのクリップに `recording` を持たせると、記録した押下を時刻どおりに再生し、実際の押下と重ねる。

```json
{ "id": "rec1", "preset_id": "glow-white", "start_us": 0, "duration_us": 8000000, "fade_in_us": 0, "fade_out_us": 0, "loop": true,
  "recording": { "length_us": 2000000, "events": [ { "t_us": 0, "x": 0, "y": 0, "velocity": 100, "duration_us": 150000 }, { "t_us": 500000, "x": 7, "y": 0, "velocity": 100 } ] } }
```

- `events` は時刻順、最大 1,024 件。`length_us` 省略時は最後のイベントの終わり。`loop` で繰り返す。記録部分は時刻だけで決まるので seek しても同じ。
- 作り方：演奏タブの「● 録音」→「■ 録音を止めて追加」（追従中のみ）、または `omoiiro-probe make-recording --in <follow の JSONL> --show <show> --layer <id> --start-us <µs>`。
- 入力履歴のリセットは演奏タブの「入力をリセット」か、`follow` の stdin コマンド `reset-input`。
- 入力に依存する効果（pads / cycle / constellation / ripple など）は複合素材の中では描かれない（内部の時刻が曲の時刻とずれるため）。

レイヤーには任意の `mask` を付けられる（そのレイヤー自身の描画だけを切り抜く）：`{ "shape": "circle" | "rect" | "diamond" | "wipe", "center_x", "center_y", "size", "size_y"?, "angle"?, "edge", "invert" }`。全項目キーフレーム可で、時間の基準はそのレイヤーの content と同じ。例：雨を円の中だけに出す（`fixtures/shows/transitions-0.1.3.show.json` の冒頭）。

`builtin.regions` の `pattern` に `custom` を指定すると `groups`（[[セル番号,…],…] 1〜32群、セル番号 = 行×8+列）が必須になり、任意の `offset`（群ごとの位相差 0〜1、群数と同じ長さ）が使える。

すべての種類の試験用ショー：`fixtures/shows/effects-0.1.2.show.json`（点滅・ノイズ）、`ripple-0.1.2.show.json`（波紋）、`rain-wave-orb-0.1.2.show.json`（雨・波形・球、10秒ずつ）、`d-effects-0.1.2.show.json`（領域ステップ・余熱・十字・形つき波紋）、`triggered-0.1.3.show.json`（発火レイヤー）、`keyframes-0.1.3.show.json`（粒から豪雨へ、加速する走査）、`paths-shapes-0.1.3.show.json`（外周ランナー2種、ムービングヘッド対、ひし形の拡大、自由グループ）、`transitions-0.1.3.show.json`（円の中の雨、アイリス、シャッター2種）、`particles-0.1.3.show.json`（火花、花火、しずく、吸い込み→爆発、噴水）、`compounds-0.1.3.show.json`（複合素材）、`frames-text-0.1.3.show.json`（星のスタンプ、文字スクロール、カウントダウン、8 フレームのループ）、`input-states-0.1.3.show.json`（星座、熱、色の循環、育つ光、塗る、長押しチャージ、記録ループ）、`audio-meter-0.1.3.show.json`（マスターの glow、Kick の orb、全列 bar。`live-bridge/sim/fake_bridge.py --meter` で模擬入力）。

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

## 3c. 複合素材（compounds、0.1.3）

複数のレイヤーをまとめて1つの素材にし、クリップとして置ける。

```json
"compounds": [
  { "id": "ribbon", "name": "ネオン・リボン", "duration_us": 4000000,
    "layers": [ { "id": "ribbon-shadow", "name": "影", "enabled": true, "opacity": 1, "blend": "over",
                  "content": { "kind": "timeline", "clips": [ { "id": "rs1", "preset_id": "wave-thick", "start_us": 0, "duration_us": 4000000, "fade_in_us": 0, "fade_out_us": 0 } ] } },
                { "id": "ribbon-core", "name": "芯", "enabled": true, "opacity": 1, "blend": "add",
                  "content": { "kind": "continuous", "preset_id": "wave-thin" } } ] }
],
"layers": [ { "id": "main", "name": "本線", "target_id": "main-pad", "enabled": true, "opacity": 1, "blend": "over",
  "content": { "kind": "timeline", "clips": [
    { "id": "c1", "compound_id": "ribbon", "start_us": 0, "duration_us": 8000000, "fade_in_us": 0, "fade_out_us": 0, "tint": "#40C0FF", "time_scale": 0.5, "loop": true } ] } } ]
```

- 内部レイヤーは外側の layer と同じ形だが、`target_id` を書かない（置いた側を継承）。content は timeline か continuous。triggered と素材の入れ子は不可。内部レイヤーは 1〜4 枚、素材は 64 個まで。
- クリップは `preset_id` か `compound_id` のどちらか一方。`tint`（色の差し替え。主の色を tint にし、他の色は明るさの比で派生。白にも効くので、白を残したい素材では使わない）、`time_scale`（0.25〜4。拍に合わせる演出には効かない）、`loop`。
- クリップが素材の長さより長いとき：`loop` が true なら繰り返し、false なら終端以降は何も描かない。短いときは途中で切る。速さを変えたいときは `time_scale` を明示する（勝手に伸縮しない）。
- GUI：「素材（複合）」欄で作成・配置・削除、タイムラインの「編集対象」で素材の中身を編集。「展開して独立」は time_scale が 1 のときだけ。
- 例：`fixtures/shows/compounds-0.1.3.show.json`（リボン、着地、交通を3回ずつ）。

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
- 粒子（particles）を多くのレイヤーで同時に使わない。最悪条件（8 レイヤー全部が噴水、尾つき）で描画 p95 4.0 ms と上限 5 ms に近い。2〜3 レイヤーまでが目安。
