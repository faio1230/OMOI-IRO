# Codex への依頼：データ側の並行制作（2026-10-28）

あなたは OMOI-IRO（https://github.com/faio1230/OMOI-IRO）の素材制作担当です。アプリのコード（`crates/`、`apps/`、`schemas/`、`live-bridge/`）は別の担当が同時に変更しているので、**触らないでください**。あなたの成果物はデータと独立したスクリプトだけです。置き場所は下記のとおり。作業ブランチは切らず、ファイルを置くだけでよい（commit は設計担当が行う）。

最初に読むもの：`docs/13-preset-authoring.md`（プリセット制作ガイド）、`docs/14-effects-roadmap.md`（60案の対応表）、`fixtures/shows/*.show.json`（雛形）、`schemas/show.schema.json`（0.1.2）。

## 依頼1：テーマ別パレット用ショー（最優先）

`docs/14-effects-roadmap.md` §2 で「今すぐ」に分類された26案を使い、次の8テーマのパレット用ショーを `shows/palette-<theme>.show.json` に作る。

| theme | 配色 |
| --- | --- |
| seabed（水底） | 深青 `#0D2346`、シアン `#20C8E8`、白 |
| furnace（溶鉱炉） | 赤 `#FF2A1A`、橙 `#FF8A00`、琥珀 `#FFC040` |
| neon-city（ネオン都市） | シアン、ピンク `#FF2AA0`、紫 `#8A2AFF` |
| moon（月面） | 藍 `#1A2060`、薄紫 `#B0A0E0`、白 |
| machine-room（機械室） | 緑 `#00C040`、白、琥珀 |
| monochrome（無彩色） | 白、グレー 2段階 |
| flower-ink（花とインク） | 桃 `#FF7AB0`、紫、青 |
| omoi-iro（OMOI-IRO） | 赤を主役に白、橙 |

各テーマに最低：背景 2（noise / rain / wave / orb のいずれか）、タップ 2（ripple の変種。常時レイヤー）、アクセント 2（strobe / pulse / chase）、終了演出 1（31 白い一撃 → 32 色の着地）。プリセット id は `<theme>-<案番号>-<短い名>`（例 `seabed-01-drop`）。レイヤーは「比較用」に、4秒ずつ順番に並べる（ripple は常時レイヤーで、説明欄がないので `name` に「パッドを押す」と書く）。

確認：`dist-win\omoiiro-probe.exe render --show <file> --at <µs> --beat <拍>`（`dist-win` はこのPCにある 0.1.2 対応版。なければ `cargo build --release -p omoiiro-probe` で作る）で、各プリセットのクリップ区間の始まり・中・終わりを見る。読み込みエラーは JSON Pointer つきで出る。すべてのショーがエラーなしで読めること。

## 依頼2：8×8 ビットマップとフォントの素材（データのみ）

本体のフレーム列・文字機能はこれから実装される。先にデータを用意する。形式は暫定で、実装担当がこの形式を読む。

- 置き場所：`fixtures/bitmaps/`。
- 形式（1ファイル1素材）：

```json
{
  "kind": "omoiiro.bitmap",
  "version": 1,
  "id": "heart",
  "name": "ハート",
  "width": 8,
  "height": 8,
  "rows": [
    "........",
    ".XX..XX.",
    "XXXXXXXX",
    "XXXXXXXX",
    ".XXXXXX.",
    "..XXXX..",
    "...XX...",
    "........"
  ]
}
```

`rows` は上の行から下の行へ、`X` が点灯、`.` が消灯。色は持たない（表示側で色を付ける）。

- 作るもの：記号（heart、star、arrow_up / down / left / right、circle、cross、check、note）と、数字 0〜9、英大文字 A〜Z を 1文字ずつ。数字と英字は 8×8 に収まる太めの字形（5×7 を中央寄せでよい。その場合も `rows` は 8 行で書く）。
- 一覧 `fixtures/bitmaps/index.json`：`[{ "id": ..., "file": ..., "name": ... }]`。

## 依頼3：動画・GIF → 8×8 フレーム列の変換スクリプト

- 置き場所：`tools/frame-convert/`（`convert.py`、`README.md`、`requirements.txt`）。Python 3.11、依存は Pillow と、動画入力用に ffmpeg（PATH にあること。なければ GIF / 連番 PNG のみ対応）。
- 出力形式（暫定、実装担当がこの形式を読む）：

```json
{
  "kind": "omoiiro.frames",
  "version": 1,
  "id": "sample-loop",
  "name": "サンプル",
  "width": 8,
  "height": 8,
  "fps": 12,
  "loop": true,
  "frames": [
    ["#000000", "#102030", "... 64 個、上の行から下の行へ、左から右へ"],
    ["..."]
  ]
}
```

- オプション：`--fps`（既定 12、上限 30）、`--max-frames`（既定 240）、`--crop`（`center` / `x,y,w,h`）、`--fit`（`cover` / `contain`、余白は黒）、`--gamma`（既定 1.0）、`--threshold`（任意、これ未満の明るさを黒に）。
- 制約：出力 1 ファイルは 2 MB 以下。フレーム数と fps を README に書く。
- 動作例：再配布可能な素材（自作の単純なアニメーション GIF を Pillow で生成してよい）を `fixtures/frames/sample-loop.frames.json` として同梱。

## 禁止事項

- `crates/`、`apps/`、`schemas/`、`live-bridge/` を変更しない。
- `schema_version` は 0.1.2 のまま。Schema にないキーを show に入れない。
- 個人の絶対パス、ホスト名、メールアドレスをファイルに書かない（公開リポジトリ）。

## 報告

作ったファイルの一覧、確認に使ったコマンド、読み込みエラーが出て直した点、未完了の項目を箇条書きで。
