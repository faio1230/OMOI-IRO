# ビルドとテスト（Cargo workspace）

M1（2026-10-28）で作成したworkspaceのビルド手順。構成と移植先は [docs/11-implementation-plan.md](docs/11-implementation-plan.md)。

## 構成

| パス | 内容 |
| --- | --- |
| `Cargo.toml` | workspace（共通依存、`rusty_link` のprofile設定） |
| `crates/omoiiro-core` | show型（serde DTO）、時間（ShowTimeUs、有理数fps、DFラベル）、surface / Frame、timeline、Solid / Chase / Pulse、合成、意味検証、headless renderer。MIDI・OS時計・I/O・threadに依存しない（依存は serde のみ） |
| `crates/omoiiro-devices` | デバイス：`launchpad_x`（SysExの組み立てと応答解析、8 x 8 gridのadapter）、`adapter`（`DeviceAdapter` 契約、`DeviceFrame`）、`color`（RGB7量子化）、`simulated`（仮想Launchpad X） |
| `crates/omoiiro-midi` | ポートの列挙と選択、`MidiBackend` 契約と `Ingress`（role tag、接続世代、サイズ上限）、midir backend、fake backend、`MidiSend`、Note/CC routing と操作 `Router`（active note、fault）、単調時計（fake clock付き） |
| `crates/omoiiro-store` | show読込（Schema Draft 2020-12検証、version判定、意味検証）、安定整形と一時ファイル置換による保存、presetのexport / import |
| `crates/omoiiro-runtime` | `sync`（ブリッジ、MTC、MIDI Clock、MTC生成、ブリッジUDP受信、`transport`）、`scheduler`（render周期）、`engine`（show差し替え、override）、`output`（MTC送信、LED writer、mailbox）、`connections`（再接続）、`control`（Trigger / Action、操作経路）、`follow`（headless追従モード）、`link`（feature `link`）、`diagnostics`（JSONL記録、replay、解析、時間の分布、NFR03） |
| `apps/probe` | 検証用CLI `omoiiro-probe`（旧 `tools/p0-probe`）。[apps/probe/README.md](apps/probe/README.md) |
| `live-bridge/` | Live用Remote Script `OmoiIroBridge`（Python）。[live-bridge/README.md](live-bridge/README.md) |
| `apps/desktop` | デスクトップアプリ（M5）。Tauri 2 の shell（`src-tauri`、パッケージ `omoiiro-desktop`、実行ファイル `omoiiro.exe`）と React / TypeScript / Vite の画面（`src`）。[apps/desktop/README.md](apps/desktop/README.md) |
| `fixtures/` | replayテスト用の記録（`artifacts/p0` から抜いたもの、合成fixture） |
| `tools/` | 実験用（`live-bridge-probe`、`link-sss-probe`）。本番コードから参照しない |

## 通常のビルドとテスト

```powershell
# リポジトリーのルートで実行する
cargo build
cargo test
.\target\debug\omoiiro-probe.exe ports
```

確認済み環境：Windows 11 Pro 25H2、Rust 1.94.1 stable-msvc、midir 0.11.1（WinMM backend）。2026-10-28（M6 準備、機器出力の許可）時点で212件pass（Link付きは214件）。workspaceのビルドには `apps/desktop/src-tauri` も含まれる（画面を未ビルドのときは `build.rs` が `apps/desktop/dist` に仮の index.html を置く）。

Live用スクリプトのテスト（Python 3.11）：

```powershell
cd live-bridge
python -m unittest discover -s tests
```

## デスクトップアプリ（M5）

固定した版（2026-10-28 に採用。上げるときは動作確認のうえでこの表とロックファイルを更新する）：

| 部品 | 版 | 固定場所 |
| --- | --- | --- |
| tauri（Rust） | =2.12.5 | ルート `Cargo.toml` の workspace.dependencies、`Cargo.lock` |
| tauri-build | =2.7.4 | 同上 |
| tauri-plugin-dialog | =2.8.4 | 同上（ファイル選択。権限は open / save のみ） |
| @tauri-apps/api | 2.12.4 | `apps/desktop/package.json`（完全一致）、`package-lock.json` |
| @tauri-apps/plugin-dialog | 2.8.4 | `apps/desktop/package.json`（完全一致）、`package-lock.json` |
| @tauri-apps/cli | 2.12.4 | 同上（`npx tauri` で使う。グローバルの tauri-cli 2.10.1 は使わない） |
| react / react-dom | 19.3.1 | 同上 |
| vite | 8.3.5 | 同上 |
| @vitejs/plugin-react | 6.1.2 | 同上 |
| typescript | 5.9.3 | 同上（7.x系は採用しない。型検査のみに使う） |
| Node.js / npm | 24.22.1 / 11.19.1 | このPCの確認環境 |
| WebView2 Runtime | 155 / 156 | Windows 11に同梱 |

```powershell
cd apps\desktop
npm ci                                  # 初回とロックファイル更新後
npx tauri dev                           # 開発（Vite 127.0.0.1:1420 + Rust）
npx tauri build --debug --no-bundle     # 画面を埋め込んだ実行ファイル（target\debug\omoiiro.exe）
npm run typecheck
```

- `omoiiro.exe <showファイル>` で起動すると、起動設定のショーファイル欄に入る（自動では開始しない）。
- Link を拍源に使う場合は、上の Link と同じ開発者シェルで `npx tauri build --debug --no-bundle --features link`。
- 素の `cargo build -p omoiiro-desktop` の debug 実行ファイルは `devUrl`（Vite）から画面を読むので、単体では画面が出ない。単体で動かすときは `npx tauri build` を使う。
- アイコンは `apps/desktop/icon-src/omoiiro-icon.png` から `npx tauri icon` で生成した（`src-tauri/icons`）。

## リリースビルド（Windows、署名なし）

```powershell
cd apps\desktop
npx tauri build                         # release の omoiiro.exe と NSIS インストーラー
cd ..\..
cargo build --release -p omoiiro-probe   # CLI（Link が要るときは開発者シェルで --features link）
```

- `tauri.conf.json` の `bundle.targets` は `["nsis"]`（WiX の MSI は作らない）。署名と自動更新は設定していない。NSIS と WiX の道具は Tauri CLI が `%LOCALAPPDATA%\tauri` に置く（このPCでは既にあった。2026-10-28 のビルドで CLI が「NSIS.exe is outdated. Recreating it」と表示して作り直した）。
- 生成物（2026-10-28）：`target\release\omoiiro.exe` 17.2 MB（debug は line-tables-only で 22.7 MB）、`target\release\bundle\nsis\OMOI-IRO_0.1.0_x64-setup.exe` 3.70 MiB、`target\release\omoiiro-probe.exe` 7.7 MB（`cargo clean` より前に測った値。上の実測の段階には含めていない）。release のビルド時間は debug のビルド済みの状態で 119 s。
- release の確認：アプリは起動して画面を読み込み、working set 30.2 MB、閉じて exit 0。インストーラーはこのPCでは実行していない（利用者の環境にインストールするため）。
- NFR03（`omoiiro-probe bench-render --frames 9000`）：release p50 1 µs / p95 2 µs / max 37 µs、debug p95 12 µs / max 209 µs。headless の Q01（release）：jitter p95 0.60 ms、render p95 18 µs、LED 送信 p95 5 µs（合成源）。

## ディスク容量

出力を小さく保つため、次の2つを固定している（2026-10-28 から）。

- ルート `Cargo.toml` の `[profile.dev] debug = "line-tables-only"`（test プロファイルは dev を継承）。完全なデバッグ情報は持たないが、パニック時のバックトレースにはファイル名と行番号が出る（確認済み：`RUST_BACKTRACE=1` で自分の関数が `tests\…rs:4`、std の関数も行番号つき）。デバッガーで変数を見たいときは、そのときだけ `debug = true` に戻す。
- ルート `.cargo/config.toml` の `[build] incremental = false`。環境変数 `CARGO_INCREMENTAL` を設定するとそちらが優先される。

`cargo clean` からの実測（2026-10-28、C: のまま。サイズは `target` 全体）：

| 段階 | target | 所要時間 |
| --- | --- | --- |
| `cargo test --workspace`（212件 pass） | 2.0 GB | 77 s |
| ＋ `cargo build -p omoiiro-probe --features link`（開発者シェル） | 2.65 GB | 53 s |
| ＋ `npx tauri build --debug --no-bundle` | 3.19 GB | 28 s |
| ＋ `npx tauri build`（release と NSIS） | 4.77 GB（debug 3.19 / release 1.57） | 119 s |

以前（完全なデバッグ情報＋incremental）は `target\debug` だけで約28 GB（うち incremental の cache が約12 GB）になり、C: の空きが無くなって「os error 112」でビルドが止まった。今の設定でも空きが少なくなったら `cargo clean` で消してよい（すべて作り直せる）。

### 出力先を別のドライブへ移す（任意）

今の設定なら全段そろえても 5 GB 前後なので C: のままで足りる。C: を空けたいときは、`CARGO_TARGET_DIR` を別ドライブに向けると、ソースは今の場所のまま、ビルド出力（`target`）だけを移せる。空きが 10 GB 以上あるドライブを選ぶ（2026-10-28 時点で D: の空きは 3.2 GB なので向かない）。

```powershell
# 1. 出力先を作る（例：E: の外付け SSD）
New-Item -ItemType Directory E:\omoiiro-target
# 2. ユーザー環境変数にする（新しく開いたシェルから効く）
[Environment]::SetEnvironmentVariable("CARGO_TARGET_DIR", "E:\omoiiro-target", "User")
# 3. 新しいシェル（Link を使うなら開発者シェル）でビルドし直す
cargo build --workspace
cd apps\desktop; npx tauri build --debug --no-bundle
# 4. 新しい場所で動くことを確かめてから、古い出力を消す（cargo が作り直せるものだけ）
Remove-Item -Recurse -Force <repo>\target
```

- 実行ファイルの場所が `E:\omoiiro-target\debug\omoiiro.exe` 等に変わる。この文書や README の `.\target\…` は読み替える。`apps/desktop/scripts/q01-gui.ps1` は `CARGO_TARGET_DIR` を見る。
- Tauri CLI（`npx tauri build`）は cargo の出力先に従い、インストーラーは `<出力先>\release\bundle\nsis\` にできる。
- rusty_link（Link）は新しい出力先で cmake から作り直す（初回は数分）。
- 元に戻すときは環境変数を消す：`[Environment]::SetEnvironmentVariable("CARGO_TARGET_DIR", $null, "User")`。
- 外付けドライブを外した状態でビルドすると失敗する（出力先が無い）。演奏用の PC では、インストーラーで入れた release 版を使えばビルド出力は要らない。

## Link付きのビルド（Windows、このPC）

`omoiiro-runtime` の feature `link`（`apps/probe` では `omoiiro-probe/link`）は rusty_link 0.4.9 を使う。rusty_link はLink SDKをcmakeでビルドし、bindgen（libclang）でbindingを作る。

```powershell
& 'C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\Common7\Tools\Launch-VsDevShell.ps1' -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
$env:LIBCLANG_PATH = 'C:\Program Files\LLVM\bin'   # ユーザー環境変数に設定済みなら不要
# リポジトリーのルートで実行する
cargo build -p omoiiro-probe --features link
cargo test --workspace --features omoiiro-probe/link
```

- ルートの `.cargo/config.toml` で `CMAKE_GENERATOR = "NMake Makefiles"` を設定している（cmake 3.26.4 は「Visual Studio 18 2026」generatorを知らない）。NMakeとcl.exeが必要なので、VS 2026 Build Toolsの開発者シェルで実行する。Launch-VsDevShell実行時の「vswhere.exe が見つからない」という表示はビルドに影響しなかった。
- ルートの `Cargo.toml` の `[profile.dev.package.rusty_link] opt-level = 2` は必須。opt-level 0 だとcmakeがLink SDKをDebug構成（MSVCRTD）でビルドし、Rust側（MSVCRT）とのリンクで `__imp__calloc_dbg` / `__imp__CrtDbgReport` が未解決になる。
- generatorや上記設定を変えた後は `cargo clean -p rusty_link`。
- link なしで `cargo build` すると `target\debug\omoiiro-probe.exe` が link なしのものに上書きされる。`monitor --link` を使う前に、開発者シェルで link 付きを再ビルドする。
- rusty_link 0.4.9 の `set_*_callback` は、値で受け取ったclosureのスタック上のアドレスをC側へ渡す。何かを捕捉したclosureは、Linkのスレッドで解放済みメモリーとして読まれる（初回実機で `misaligned pointer dereference` で異常終了した）。`omoiiro-runtime::link` では何も捕捉しない（サイズ0の）closureだけを渡し、送り先はプロセス内の共有スロットに置いている。closureがサイズ0であることはテストで検査している。
- rusty_link の `abl_link_create` は `ableton::LinkAudio` を生成する。S07の切り分けでは、Start Stop Sync による Live の停止は素の `ableton::Link` でも再現したので、LinkAudio が原因ではない。アプリは Start Stop Sync を有効にしない。
- rusty_link は GPL-2.0-or-later。配布形態は docs/07-decisions.md O09 を参照。
