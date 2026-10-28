# 2026-10-28 夜の実装のまとめ（実装者向け・引き継ぎ用）

docs/14 §4 の段階1〜9 と、その後の検証・本番用の雛形・堅牢性の監査をまとめる。仕様の正本は docs/（設計側が管理）で、ここは実装の地図。パスはすべてリポジトリ相対。

- 作業の進め方：設計セッションの仕様 → 実装 → `cargo test --workspace`・`npx tsc --noEmit`（apps/desktop）・`npx tauri build`（release）→ 報告 → 設計が commit・push。
- 最終状態：commit 5cc4384 の時点で、`cargo test --workspace` 312 件、live-bridge の Python 8 件がすべて通る。release ビルド成功。
- 実測（Windows の Live）と合成の区別：Live で測ったのは段階9の meter の受信だけ（下記）。ほかの数値はすべて合成データ（fake MIDI backend、fake_bridge、仮想 Launchpad X）による。Launchpad X（LPX）の実機は使っていない。

## 共通の土台

- Schema は **0.1.3**（`omoiiro_core::model::SCHEMA_VERSION`）。0.1.0 / 0.1.1 / 0.1.2 は読込時に版番号だけを書き換える（`omoiiro-store` の `migrate`）。段階1〜9 の追加はすべて 0.1.3 の中の任意項目、または新しい Effect 型で、既存ファイルの描画は変えない。
- スキーマ本体：`schemas/show.schema.json`。値は全 Effect 共通で `anyOf [定義, $defs/keyframes]`（キーフレーム不可の項目を除く）。
- 正規形の保存：`omoiiro_store::to_canonical_string`（整数値の f64 は整数で書く：`ser_num`、`ser_opt_num`、`ser_opt_nums`、`ser_opt_points`）。
- 描画：`omoiiro_core::render::Renderer`。入口は `render(t, beat)`、`render_input(t, beat, pads)`、`render_all(t, beat, pads, audio)`。`CompiledEffect` は段階7 から `Copy` ではなく `Clone`（frames と text のデータを `Arc` で共有するため）。
- 編集：`omoiiro_core::edit::{EditCommand, apply, History}`。すべての変更は `apply` が検証（`validation::validate`）を通してから反映する。
- 安全判定：`omoiiro_core::safety::{effect_flash, FlashProfile, FlashSource}`。

## 段階1：発火モデル

- 入ったもの：triggered レイヤー `Content::Triggered { clips, trigger: TriggerSpec }`。`TriggerSpec.on` は press / release / hold / clip_start / interval（`interval_ms`、`origin_x/y`）。クリップは発火を受け付ける時間の窓。
- 入力：`render::{PadEvent, PadHit, PadInput}`。`PadHit` は `release_us`、`show_us`（届いた時点の曲の時刻）を持つ。長押しは `MAX_HOLD_US` = 10 s で打ち切り、`MAX_AGE_US` = 20 s を過ぎた押下は捨てる。
- instance は `(x, y, age)` で最大 64 個（`MAX_INSTANCES`）。instance を受ける効果は ripple、tap_glow、cross（後に particles、frames も）。それ以外の効果は `hold` のときだけ描く（ブラインダー）。
- 関数：`triggered_instances`、`CompiledEffect::paint_instances`、`takes_instances`。

## 段階2：キーフレーム

- `model::Param<T>` = `Value(T)` または `Keyframes(Vec<Keyframe { t_us, v, ease }>)`。ease は linear（既定）/ step / ease_in / ease_out / ease_in_out。
- `Sample` trait：f64・u32・u64 は補間（整数は丸め）、色（String）は描画で補間、bool と enum は step だけ（`step_only!` マクロ）。
- 読込時の拒否：空、64 点超、時刻の逆順、step だけの型に補間の ease。
- `CompiledEffect::compile_at(effect, t)` で時刻 t の値を取る。キーフレームを持つプリセットは `Renderer.keyed` に元の Effect を持ち、フレームごとに取り直す（`has_keyframes` は serde_json で判定）。
- 時間軸：クリップならクリップのローカル時刻、continuous なら曲の時刻、instance 効果なら発火からの経過時間。
- 安全判定と検証は全キーフレーム値の最悪値を見る。個数を表す整数（seed、repeats、max_concurrent、width など）は `Ctx::step_only`。

## 段階3：領域・経路・図形

- `core/shape.rs`（新規）：`Metric {Circle, Diamond, Square}`、`metric`、`unrotate`、`sd_rect`、`segment_distance`、`fill_alpha`、`stroke_alpha`（edge 0 でも線幅は最低1セル）。ripple の形もこの関数を使う。
- regions：`pattern: custom` と `groups`（セル番号 0〜63 の組、1〜32 組）、`offset`（組ごとに 0〜1 段遅らせる）。
- `builtin.path`（`PathParams`）：line / perimeter / orbit / figure8 / points、`loop`、`phase`、`mirror`（none/x/y/xy）。figure8 の横の半幅は `orbit_radius` を流用。速さの単位は、閉じた経路は周/秒、line・points はセル/秒。
- `builtin.shape`（`ShapeParams`）：line / circle / rect / diamond（段階4で wipe と invert を追加）。
- 安全判定：Path は Wave と同じく警告だけ（消灯の対象外）。

## 段階4：マスクとトランジション

- shape に `wipe`（半平面。angle 0 で +x 側を塗る、size は使わない）と `invert` を追加。37 アイリス・38 シャッターは「黒い図形を over で重ねる」形で作る（合成器は変えない）。
- レイヤーに任意の `mask`（`model::LayerMask`、自分の描画だけを切り抜く）。時間軸はそのレイヤーと同じで、重なったクリップがあれば先頭のもの。関数は `render::apply_mask`。
- 範囲の拡張：shape と mask の center は -4〜11、shape の size の最小は 0。
- 編集コマンド：`SetLayerMask`。

## 段階5：粒子

- `core/particles.rs`（新規）：`builtin.particles`（`ParticlesParams`）。mode は burst / fountain / rocket / implode。`implode_radius` は確定後に追加した項目。
- 決定性：粒子の値は `hash01(seed, event_id, j)`。位置は抵抗と重力の解析解だけで求める（積分しない）ので、seek しても同じ絵になる。bounce は軸ごとの三角波で折り返す。vanish は一度でも盤の外に出たら戻らない。
- 発生源：timeline はクリップの開始に1回、continuous は寿命ごとに繰り返す、triggered は押した位置から（rocket は押した列の下端から、fountain は hold の間）。
- 上限：イベントは 16、1レイヤーの粒子は 256、ショー全体の粒子は 768（`MAX_SHOW_PARTICLES`、段階7の最初に追加）。古いものから落とす。
- 関数：`ParticleLook::compile`、`paint_event`、`continuous_particle_events`、`triggered_particle_events`、`paint_particles`（ショー全体の残り数を引数で受ける）。
- 安全判定の対象外。

## 段階6：複合素材

- `Show.compounds: Vec<Compound { id, name, duration_us?, layers }>`。内部レイヤーは `Layer` と同じ形で、`target_id` は空（置いた側のものを継承）、content は timeline か continuous。最大 4 枚、入れ子は不可。
- クリップに `compound_id`（`preset_id` とどちらか一方。preset_id は空文字で「無し」）、`tint`、`time_scale`（0.25〜4）、`loop`。
- 描画：置き方ごとに `CompoundUse` を作る。tint は `render::tinted`（主の色を tint にし、ほかの色は明るさの比で合わせる。白も tint の色に変わる）。内部時刻 = ローカル時刻 × time_scale。内部レイヤーは premultiplied のバッファで合成する（`blend_into`、`paint_compound`）。
- レイヤー ID とクリップ ID はショー全体で一意。そのため既存の編集コマンドは素材の中にもそのまま効く（`all_layers`、`layer_mut`、`find_clip` は (layer_id, index) を返す）。
- 編集コマンド：`AddCompound`、`SetCompound`、`DeleteCompound`（使用中は拒否）、`AddCompoundLayer`、`AddCompoundClip`、`SetCompoundClip`、`DetachCompoundClip`（time_scale が 1 以外なら拒否）。
- 安全判定：素材の中の点滅は、置いたレイヤーに数え、時間で決まる周波数には time_scale を掛ける。
- GUI：`apps/desktop/src/Compounds.tsx`、タイムラインの「編集対象」の切り替え。

## 段階7：フレーム列・文字・取り込み

- `builtin.frames`（`FramesParams`）：384 桁の hex（上の行から、左から右）を 1〜240 個、fps 1〜30、loop、hold_last、hold_ms、anchor（board / pad：フレームの (3, 3) を押したパッドに合わせる）、color、opacity_key（黒を透明にする、既定 true）。キーフレームは使えない。`render::FramesLook`。
- `builtin.text`（`TextParams`）：`core/font.rs` に 5×7（ASCII 95 字）と 3×5（数字・大文字・記号）を内蔵。mode は static / scroll_left / scroll_up / sequential、beat_step（クリップの開始を拍の頭とみなして数える）。`render::{TextLook, paint_text}`。
- 取り込み：`core/assets.rs` の `FramesAsset`（`omoiiro.frames` と `omoiiro.bitmap`）→ `EditCommand::ImportFrames`。ファイルの読み込みと 8 MB の上限は `omoiiro_store::assets`。Tauri コマンドは `frames_import`。ショーファイル全体の上限は 16 MB のまま。
- 安全判定：frames は fps 6 以上で警告だけ、text は scroll の速さ × 2 で警告だけ。

## 段階8：入力状態と記録

- `PadInput.history`（最後のリセット以降、1,024 件・60 s）。既存の `hits`（16 件・20 s）は変えていないので、既存の効果は今までどおり。`PadEvent::Reset` で両方を空にする。
- `core/history.rs`（新規）：`builtin.pads`（heat / plant / paint）、`builtin.cycle`、`builtin.constellation`、tap_glow の `charge`。どれも状態を持たず、毎フレーム履歴から計算する。数えるのはクリップの区間内の押下だけ。素材の中では描かない。
- 記録：クリップに `recording { events (最大 1,024), length_us? }`。`history::clip_input` がクリップのローカル時計で、実際の入力と記録を合わせた `PadInput` を作り、既存の発火・粒子・frames・新しい効果に渡す。記録の部分は seek しても同じ絵になる。
- 変換：`history::recording_from(events, start_us)`。編集コマンドは `AddRecordingClip`、`SetClipRecording`。
- Runtime：`follow::PadLog`（録音の記録と JSONL 出力）、`FollowSession::{reset_input, start_recording, stop_recording, recording_since}`。Tauri コマンドは `reset_input`、`recording_start`、`recording_stop`。GUI は `apps/desktop/src/InputPanel.tsx`。

## 段階9：音反応

- ブリッジ（`live-bridge/OmoiIroBridge`）：時刻のレコード（v2）は変えずに、別の流れとして `{"v":3,"kind":"meter","t","master":[l,r],"tracks":{name:[l,r]}}` を送る。マスターの output_meter_left のリスナーで送り、33 ms 未満の間隔は間引く。対象トラックは名前が `[LED]` で始まるもの最大 4 本（1 秒ごとに選び直す）。meter が 100 ms 途絶えたら update_display から補って送る。例外は場所ごとにログへ1回だけ出す。
- アプリ：`render::AudioInput::from_meter_record`。sync スレッドで meter と時刻を振り分け（`FollowShared.audio`）、受信から 500 ms 超は途絶として扱う。`Engine.audio` / `audio_fresh` を経て `render_all` に渡す。
- `builtin.meter`（`core/meter.rs`）：source（master / track）、channel、mode（bar / glow / orb）。値 = min(1, clamp((raw − floor) / (1 − floor), 0, 1) × gain)（floor 0〜0.9）。平滑化（attack / release）はレイヤーごとの状態で持ち、クリップが変わったら 0 から始める。orb は値 0 では描かない。
- 互換：段階9より前のアプリは v3 を受けると fault になる。新しいブリッジは新しいアプリと組み合わせる。

## 検証と本番用の雛形

- 回帰：`omoiiro-probe q01`（23 項目）と `apps/desktop/scripts/q01-gui.ps1`（G1〜G6）がすべて PASS。結果は `apps/desktop/docs/q01-report-2.md`。
  - G1.2（名前を付けて保存）は、WM_SETTEXT と BM_CLICK を使う形で無人でも通るようにした。
  - スクリプトのオプション：`-Release`、`-BridgePort`（ほかの送り手がいるとき）、`-Meter`。
- 10分 soak（重いショー `fixtures/shows/soak-heavy-0.1.3.show.json`）：本体 43.4 → 44.3 MB、子孫プロセス込み +6.3%。
- Windows の Live での meter の実測：再生中 24.6 Hz（p50 33.5 ms、p95 66 ms）、曲中時刻のリスナーは 60 Hz のまま。キックの値は平滑化後に 0.75〜0.92。
- 本番用の雛形：`shows/omoi-iro-live-template.show.json`（8 レイヤー、31 プリセット）、`shows/omoi-iro-palette.show.json`（パレット比較）。
- 堅牢性
  - `Engine::evaluate` の描画を catch_unwind で包み、panic したフレームは前のフレームを保持する（`render_panics`、`render_fault`。演奏タブの「LED出力」に表示）。
  - fuzz 18,000 件で panic 0。
  - 操作経路の p95 は、軽いショーでも重いショーでも 22 µs で変わらない。

## probe の新しいコマンド

| コマンド | 使い方 | 内容 |
| --- | --- | --- |
| `bench-render --particles` / `--compounds [--particles]` | `omoiiro-probe bench-render --compounds --frames 9000 --json` | 粒子や複合素材の最悪の負荷で描画時間（NFR03）を測る |
| `import-frames` | `--show <show> --file <asset> [--rename] [--out <path>]` | `omoiiro.frames` / `omoiiro.bitmap` を frames のプリセットとして取り込む |
| `make-recording` | `--in <follow の JSONL> --show <show> --layer <triggered> --start-us <曲の時刻> [--preset] [--length-us] [--loop] [--out]` | `follow --out` の pad レコードから記録クリップを作る |
| `check-shows` | `omoiiro-probe check-shows shows fixtures/shows examples [--write]` | 読込 → 保存 → 再読込でバイト一致 → 3 時点で描画。`--write` は正規形で保存し直す |
| `fuzz-shows` | `omoiiro-probe fuzz-shows <files> --iterations 1000 --seed 7` | 損傷を入れたファイルで、拒否か正常で終わり panic しないことを確かめる |
| `route-latency` | `omoiiro-probe route-latency --show <show> --seconds 20 --rate 20` | fake MIDI で追従中の操作経路（callback → send）と描画時間を測る |
| `follow` の stdin コマンド | `reset-input` | 入力履歴を空にする。`--out` の JSONL に pad レコードも書く |

- シミュレーター：`live-bridge/sim/fake_bridge.py --meter`（合成の meter、120 bpm のキック）。

## 既知の簡略化

- 素材（compound）の中では、入力依存の効果（pads・cycle・constellation）と meter は描かない（内部の時計が曲の時刻と違うため）。
- tint はプリセット単位で置き換えるため、白も tint の色になる。
- 同じレイヤーで重なったクリップでは、mask の時間軸と記録の扱いは先頭のクリップに合わせる。
- 粒子のショー全体の上限は、下のレイヤーから順に使う（ショー全体での古い順ではない）。
- DetachCompoundClip の近似：内部の blend は下のレイヤーに直接かかり、外側の opacity は掛け合わせる。
- frames の color と text の速さはキーフレーム不可。
- meter の値は Live のメーター（平滑化済み、約 25 Hz）なので、周波数の帯域は取れない。

## 未確認事項

- Launchpad X の実機での見え方（粒子、frames、text、入力状態の効果）と、実機での録音。
- Mac（Apple Silicon）での新しいブリッジ（v3）の動作と meter の受信頻度。
- 実機の MIDI ドライバーを含む操作経路の遅延（今回の数値は fake MIDI）。
- Codex の素材（`fixtures/bitmaps/`、`fixtures/frames/`、`shows/` の追加分）の取り込み。届いたら `check-shows` と `fuzz-shows` で検査する。
