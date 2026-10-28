# Codex への依頼：演出プリセットの追加（ライブラリー用ショー、2026-10-29）

あなたは OMOI-IRO（https://github.com/faio1230/OMOI-IRO）の素材制作担当です。アプリのコード（`crates/`、`apps/`、`schemas/`、`live-bridge/`）は別の担当が同時に変更しているので、**触らないでください**。成果物はショーファイル（JSON データ）だけです。

最初に読むもの：`docs/13-preset-authoring.md`（制作ガイド。Schema 0.1.4、分類 `metadata` の書き方を含む）、`schemas/show.schema.json`、雛形 `shows/omoi-iro-live-template.show.json`（31 プリセット）、比較用 `shows/omoi-iro-palette.show.json`。

## 納品の形

- `shows/omoi-iro-library-<テーマ>.show.json` を 1 テーマ 1 ファイルで作る。アプリはこのファイルを「ライブラリー」として登録し、必要な素材だけを本番ショーへ取り込む（ファイルは読むだけで書き換えない）。
- 作り方は雛形を複製して `id`、`name`、`presets`、`layers` を書き換える。`sync`、`render`、`targets` は雛形のまま。`schema_version` は `0.1.4`。
- レイヤーは「比較用」に、プリセットを 4 秒ずつ順番に並べる。タップ系（ripple、tap_glow、pads、cycle、constellation）は常時レイヤーに置き、`name` に「パッドを押す」と書く。
- 各プリセットに必ず `metadata` を付ける：`category` は「タップ」「背景」「アクセント」「トランジション」「照明」「グラフィック」のどれか、`tags` は色・動き・強さ・用途（例 `["暖色", "ゆっくり", "サビ"]`）。
- プリセット id は `<テーマ>-<連番2桁>-<短い英名>`（例 `seabed-01-drop`）、`name` は日本語で画面に出る名前（例 `水滴（青）`）。

## 今回のテーマと最低数

テーマ（配色）は `prompts/codex-assets.md` の 8 テーマ表を使う。1 テーマあたり最低：背景 3、タップ 3、アクセント 3、トランジション 2、グラフィック 1（text か frames か shape）。使える演出は 20 種すべて（solid, chase, pulse, strobe, noise, ripple, rain, wave, orb, regions, tap_glow, cross, path, shape, particles, frames, text, pads, cycle, constellation, meter）。キーフレーム、マスク、発火レイヤー（triggered）も可。

点滅が速いもの（3 Hz 超）は `tags` に「高速点滅」を入れ、常時レイヤーには置かない。

## 確認（必須）

```
dist-win\omoiiro-probe.exe check-shows --write shows
dist-win\omoiiro-probe.exe render --show shows\omoi-iro-library-<テーマ>.show.json --at <µs> --beat <拍>
```

`check-shows` が全ファイル PASS であること（正規形に整え、読込・保存・描画を検査する。`--write` で整形まで行う）。`render` で各プリセットの区間の始まり・中・終わりを見て、真っ黒や全面同色のまま終わるプリセットが無いこと。読み込みエラーは JSON Pointer つきで出る。

## 禁止事項

- `crates/`、`apps/`、`schemas/`、`live-bridge/` を変更しない。
- Schema にないキーを入れない。
- 個人の絶対パス、ホスト名、メールアドレスを書かない（公開リポジトリ）。

## 報告

作ったファイル、テーマごとのプリセット数、`check-shows` の結果、直した読み込みエラー、未完了を箇条書きで。
