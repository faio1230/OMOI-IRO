# tools/p0-probe（移動済み）

P0 の検証用CLI `p0-probe` は、M1（2026-10-28）で Cargo workspace に移った。

| 旧 | 新 |
| --- | --- |
| `tools/p0-probe/src`（CLI部分） | [apps/probe](../../apps/probe/README.md) |
| デコーダー、記録、replay、Link観測 | `crates/show-runtime`（`sync`、`diagnostics`、`link`） |
| ポート、ingress、routing | `crates/show-midi` |
| Launchpad X の SysEx | `crates/show-devices`（`launchpad_x`） |
| `tools/p0-probe/fixtures` | ルートの [fixtures](../../fixtures) |

コマンド、表示、JSONL形式は変わらない。ビルドはリポジトリーのルートで行い、実行ファイルは `target\debug\p0-probe.exe` になる（手順は [BUILD.md](../../BUILD.md)）。

P0 の実測記録 `artifacts/p0/` は移動していない。過去の文書に出てくる `tools\p0-probe` からの相対パス（`.\target\debug\p0-probe.exe`、`..\..\artifacts\p0\...`）は、ルートからの `.\target\debug\p0-probe.exe`、`artifacts\p0\...` に読み替える。
