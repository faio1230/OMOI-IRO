# P0 環境調査（D01）

確認日：2026年10月27日（Live 12 / LiveMTC / 音声ドライバー / ポート対応の項目は2026年10月28日に更新）。対象はこの資料を作成したWindows PC。Macの情報は実機試験の立ち会い時に取得する。取得値は読取専用の方法で得たものだけを記載し、読めなかった値は「未確認」とする。

## 判定の要約

| 項目 | 状態 | 影響 |
| --- | --- | --- |
| Rust / MSVCビルド | 利用可能。hello world と midir 0.11.1 のビルド・実行を確認 | P0ツールはこのPCでbuild/testできる |
| Ableton Live 12 Suite | 2026-10-28に導入。12.4.7（Build 2026-10-16_cf213561fb）、Licensing Variant=Suite。音声はGeneric Low Latency ASIO Driverに変更済み | Windows側でS02〜S04を実施済み（[p0-results.md](p0-results.md)）。S07はlibclang待ち |
| Max for Live / Max | Live同梱のMax 9.1.5（`Resources\Max\Max.exe`）を確認。LiveMTC 1.1.0（Bridge + MTC.amxd）を2026-10-28に導入 | LiveMTCは動作確認済み（MTC 30 fps出力をアプリで受信） |
| Ableton Link binding | LLVM 23.1.4（libclang）を2026-10-28に導入。`--features link` はVS 2026開発者シェル + `CMAKE_GENERATOR=NMake Makefiles` でビルド成功 | Link観測（S07）の実装に進める。観測機能自体は未実装 |
| 仮想MIDIバス | loopMIDI 1.0.16.27 導入済み。`loopMIDI Port`、`loopMIDI Port 1` に加え、2026-10-28に `DJSC Control`、`DJSC Timecode`、`DJSC Clock` を作成。teVirtualMIDI、Bome Virtual MIDI、rtpMIDI も導入済み | Live 12.4はこれらを `<PC名> (Port N)` と表示する（下記）。起動順はloopMIDI→Bridge→Live |
| Windows MIDI Services | `midisrv` 稼働中（10.0.26100.8875）。MIDI 2.0 Loop / Virtual Devices のtransportが存在 | 新MIDI Servicesの仮想endpointも候補。P0ではWinMM経路を基本にする |
| Launchpad X | 未接続 | S01はLPX接続後 |
| Git | リポジトリーではない（`.git` なし） | 変更履歴は手動管理 |

## OSとハードウェア

| 項目 | 取得値 | 確認方法 | 未確認事項 |
| --- | --- | --- | --- |
| OS | Windows 11 Pro 25H2、10.0.26200 build 26200.9457、64 bit | `Get-CimInstance Win32_OperatingSystem`、`HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion` の DisplayVersion / UBR | なし |
| CPU | 12th Gen Intel Core i9-12900、16コア / 24論理、2.4 GHz基準 | `Get-CimInstance Win32_Processor` | P/Eコア構成の詳細 |
| RAM | 63.7 GB | `Win32_ComputerSystem.TotalPhysicalMemory` | なし |
| ホスト名 | <PC名> | midirのrtpMIDI session port名 | なし |
| ネットワーク | Realtek Gaming 2.5GbE（Up）、Tailscale Tunnel（Up）、Hyper-V Default Switch（Up） | `Get-NetAdapter` | Linkのmulticast（UDP 20808）がどのadapterで到達するか。Tailscale / Hyper-V 側に漏れないか |
| Firewall | Domain / Private / Public すべて有効 | `Get-NetFirewallProfile` | Link初回起動時のUDP受信許可ダイアログの扱い |

## ビルド環境

| 項目 | 取得値 | 確認方法 | 未確認事項 |
| --- | --- | --- | --- |
| rustc / cargo | 1.94.1（2026-03-25）、stable-x86_64-pc-windows-msvc | `rustc --version`、`rustup toolchain list` | なし |
| rustup | 1.29.0。導入済みtargetは x86_64-pc-windows-msvc のみ | `rustup target list --installed` | aarch64-apple-darwin はMac側で別途確認 |
| MSVC | VS Build Tools 2026 18.5.0（VC Tools 14.50.35717）、VS 2022 Community 17.11.2、VS 2022 Build Tools 17.4.5、VS 2019 Community 16.11.23 | `vswhere -all -products *`、`VC\Tools\MSVC` の列挙 | Community 2022 側にVC Toolsディレクトリーが見当たらない。rustcは18 Build Toolsのlinkを使っていると推定 |
| Windows SDK | 10.0.26100.0 | `Windows Kits\10\Include` の列挙 | なし |
| cmake | 3.26.4 | `cmake --version` | rusty_link の要件（3.14以上）を満たす |
| libclang（bindgen用） | 2026-10-28に利用者の依頼で `winget install LLVM.LLVM`（23.1.4）を導入。`C:\Program Files\LLVM\bin\libclang.dll`。ユーザー環境変数 `LIBCLANG_PATH=C:\Program Files\LLVM\bin` を設定済み（新しいシェルから有効） | `clang --version`、パス存在確認 | なし |
| loopMIDIの折り返し停止（2026-10-28 14時台、同日16時台にmidisrv再起動で解消） | M6準備中、`OMOI-IRO Control / Timecode / Clock` の3ポートがWinMMの出力からの送信を1バイトも受け取らなくなった（loopMIDI画面のTotal data 0、Liveを介さない `gen-mtc` → `monitor` の折り返しも0件、別の受信器でも0件）。Liveを閉じてloopMIDIを再起動しても、DetectFeedback=0にしても変わらず（レジストリーは元に戻した）。WinMMの `OMOI-IRO …` はmidisrv（Windows MIDI Services）が作る `SWD\MMDEVAPI\MIDIU_KSA_…` になっており、WinMMとloopMIDIの間にmidisrvの変換層が入っている（P0の03:04時点では通っていた。差分は未確認）。OmoiIroBridgeのUDP受信は正常（`artifacts/m6/win-m6-prep-01.jsonl`）。**復旧手順**：Liveを閉じる → 利用者が管理者権限で `sc stop midisrv` / `sc start midisrv`（SSH経由でも可。sshdログインは昇格済み） → loopMIDIを終了して起動し直す → `omoiiro-probe ports` で入力4・出力5を確認 → 各ポートで `monitor` 5秒 + `gen-mtc` 2秒。3ポートとも quarter frame 240件とフルフレーム1件、lock 1、discontinuities 0、dropped 0 を受信して復旧を確認（合成データ、Liveなし、16:11）。原因はmidisrvの状態と推定するが、再起動で直ったこと以外は未特定。再発時は同じ手順を先に試し、直らなければPC再起動 | 実装担当の実測 | 復旧済み。原因未特定、再発監視 |
| ビルド出力の容量（2026-10-28） | `target\debug` が約28 GB（incremental約12 GB含む）に達しC:の空きが100 MBまで減った。対策：`[profile.dev] debug = "line-tables-only"` と `[build] incremental = false` を固定。`cargo clean` からの全段（test → Link付きprobe → Tauri debug → Tauri release + NSIS）で `target` は4.77 GB（debug 3.19 / release 1.57）。行番号付きのバックトレースは出る。別ドライブへの移動は不要（任意手順としてBUILD.mdに残す） | 実装担当の実測、設計担当がCargo.tomlと容量を確認 | release の `omoiiro.exe` が18.0→17.2 MBに変わった原因は未確認 |
| `--features link` のビルド | 成功（rusty_link 0.4.9、Link SDKをcmakeでビルド、約20 s）。M1以降はルートで `cargo build -p p0-probe --features link`（`.cargo/config.toml` に `CMAKE_GENERATOR=NMake Makefiles` を設定済み）。ただし条件つき：cmake 3.26.4は cc crate が選ぶ generator「Visual Studio 18 2026」を知らず失敗し、VS 2022 Community / Build Tools にはC++ツールが無い。回避策は VS 2026 Build Tools の開発者シェル（`Launch-VsDevShell.ps1 -Arch amd64 -HostArch amd64`）で `cl.exe` をPATHに入れ、`CMAKE_GENERATOR=NMake Makefiles` でビルドする。generatorを変えたときは `cargo clean -p rusty_link` が必要 | `cargo build --features link` | 恒久策はcmake 4.x（winget Kitware.CMake 4.4.4、VS 2026対応）への更新。cmake 4は古いCMakeLists（3.5未満の互換）を拒否するため、他プロジェクトへの影響は利用者判断 |
| cl / clang on PATH | いずれもPATH上にない | `Get-Command` | cargoはVSを自動検出するため影響なし |
| cargo build 動作 | hello world をbuild・実行（9.9 s） | scratchpadで `cargo new` → `cargo build` | なし |
| midir 0.11.1 | buildと `MidiInput::ports()` / `MidiOutput::ports()` の列挙に成功（WinMM backend） | scratchpadの最小binary | WinRT backend（`winrt` feature）は未評価 |
| Node / npm | v24.22.1 / 11.19.1（nvm4w） | `node --version` | Tauri GUIはP0対象外 |
| Python | 3.11.2 | `python --version` | なし |
| Git | 2.39.0.windows.2 | `git --version` | プロジェクトフォルダーはgit管理されていない |

## MIDIと音声

| 項目 | 取得値 | 確認方法 | 未確認事項 |
| --- | --- | --- | --- |
| midirから見えるポート（取得時） | 入力：`<PC名>`（rtpMIDI session）。出力：`Microsoft GS Wavetable Synth`、`<PC名>` | scratchpadのmidir列挙 | loopMIDI停止中のためloopMIDIポートは未表示。起動後に再列挙する |
| loopMIDI | 1.0.16.27、`C:\Program Files (x86)\Tobias Erichsen\loopMIDI\loopMIDI.exe`。レジストリーに `loopMIDI Port`、`loopMIDI Port 1` の定義。プロセスは停止中 | Uninstallレジストリー、`HKCU:\Software\Tobias Erichsen\loopMIDI\Ports`、`Get-Process` | Windows 11 25H2上での安定性。配布元の対応表記はWindows 10まで |
| teVirtualMIDI | 1.3.0.43、driverはロード済み（MEDIA / MidiEndpoint） | Uninstallレジストリー、`Get-PnpDevice` | なし |
| rtpMIDI | 1.1.14.247、サービス稼働中 | `Get-Service rtpMIDIService` | P0では使用しない |
| Bome Virtual MIDI | 2.1.0.44 | Uninstallレジストリー | P0では使用しない |
| Pocket MIDI | 1.7.0（MIDIモニター） | Uninstallレジストリー | 比較用の第二観測器として使える |
| Windows MIDI Services | `midisrv` 稼働中、10.0.26100.8875。`MIDI 2.0 Loop Devices`、`MIDI 2.0 Virtual Devices`、`MIDI 2.0 Service Tests` のSoftwareDeviceあり | `Get-Service midisrv`、`Get-PnpDevice` | WinMM経由でのmulti-client挙動、Loop transportの作成手順 |
| Launchpad X | 未接続。Novation / Focusrite のPnPデバイスなし | `Get-PnpDevice -PresentOnly` | firmware版、USB接続時のポート名（Windowsでは `LPX MIDI` と `MIDIIN2 (LPX MIDI)` 系の2系統が予想されるが未確認） |
| 音声デバイス（実体） | Realtek(R) Audio、NVIDIA High Definition Audio | `Get-PnpDevice -Class MEDIA` | なし |
| ASIOドライバー登録 | Realtek ASIO、Generic Low Latency ASIO（Steinberg 1.0.30）、Yamaha Steinberg USB ASIO、RME Fireface / MADIface USB、ZOOM L-12 / U-44、Blackmagic、Dante Virtual Soundcard、Voicemeeter系、VB-Matrix系、JackRouter | `HKLM:\SOFTWARE\ASIO` | 登録のみで実機未接続のものを含む。Live 12でLinkを使う際のdriver選択は未確認 |
| 仮想音声 | VoiceMeeter VAIO / AUX、VB-Audio Matrix VAIO、VB-Audio Virtual Cable、Dante Virtual Soundcard、NDI Audio | `Get-PnpDevice -Class MEDIA` | なし |

## Ableton LiveとMax

| 項目 | 取得値 | 確認方法 | 未確認事項 |
| --- | --- | --- | --- |
| Live 12 Suite | 12.4.7、Build 2026-10-16_cf213561fb、64-bit、Variant=Suite。`C:\ProgramData\Ableton\Live 12 Suite\Program\Ableton Live 12 Suite.exe`。2026-10-28 00:24 に初回起動（デモソング表示） | exeのVersionInfo、`%APPDATA%\Ableton\Live 12.4.7\Preferences\Log.txt` | Mac側のLive版は未確認 |
| Live 11 の痕跡 | `%APPDATA%\Ableton\Live 11.3.10 / 11.3.11 / 11.3.12` と `HKLM:\SOFTWARE\Ableton\LiveSuite.11`。Log.txt の最終起動は 2023-11-10、`Live 11.3.12 Build: 2023-10-05_4a81dc007f` | ファイルとレジストリーの列挙 | 本体はアンインストール済みか別ドライブ。P0の対象ではない |
| Max | Live同梱 Max 9.1.5（`Resources\Max\Max.exe` のProductVersion 9.1.5.3db35fa476d）。単体のMaxは未導入 | exeのVersionInfo | LiveMTCの公表要件（Max 8.3以降）は満たす |
| Linkの利用状態 | 初回起動時 `Link: Disabled`。Audio driverは `MME/DirectX`、Output `VoiceMeeter Input`。2026-10-28にASIO / Generic Low Latency ASIO Driver（441 samples、44.1 kHz、in 10 ms / out 20 ms）へ変更。Realtek ASIOは「オーディオエンジン無効」、Voicemeeter Virtual ASIOはVoicemeeter未起動のためトランスポートが約56%の速度になった | Preferences画面、`monitor` のClock BPM、Live位置表示 | Link本体は未検証。Linkボタンはトランスポートに表示されている |
| LiveMTC | 1.1.0（2024-11-12版、zip 68,548,448 bytes、SHA-256 `EBFED9FE…62A2`）。Bridge：`%LOCALAPPDATA%\Programs\LiveMTC Bridge\LiveMTC Bridge.exe`（未署名、showsync公式URLから取得）。デバイス：`%USERPROFILE%\OneDrive\ドキュメント\Ableton\User Library\LiveMTC\MTC.amxd`。Bridge設定：MIDI Port `DJSC Timecode`、Timecode Rate 30 FPS、Offset 0 ms、OSC 127.0.0.1:4587（未使用） | 配布zipの展開、Bridge画面 | Bridgeは起動後にMIDIポートを再列挙しない。Mac版は未導入 |
| LiveのUser Library | `%USERPROFILE%\OneDrive\ドキュメント\Ableton\User Library`（Library.cfgの `ProjectPath`） | `%APPDATA%\Ableton\Live 12.4.7\Preferences\Library.cfg` | `Documents\Ableton\User Library` はLiveが参照しない旧フォルダー |
| 改名後の状態（2026-10-28、M5冒頭） | コードは `omoiiro-*` / `omoiiro-probe` / `OmoiIroBridge` / `OMOI-IRO Control・Timecode・Clock` に改名済み。loopMIDIには利用者が `OMOI-IRO Control / Timecode / Clock` を作成済み（同日、`omoiiro-probe ports` で入力#1〜#3、出力#2〜#4として確認。旧 `DJSC` ポートは未確認）。LiveのUser Libraryの `OmoiIroBridge` への入れ替えとLiveのMIDI設定の付け替えは未確認。手順は [p0-runbook.md](p0-runbook.md) 冒頭 | `omoiiro-probe ports`、実装担当の報告 | Live側の移行は利用者が実施 |
| Python Remote Script `DJSCBridgeProbe` | User Library の `Remote Scripts\DJSCBridgeProbe\`（`tools/live-bridge-probe/` のコピー）。Control Surface 1 に割当、Input/Output None。Live起動時に読み込まれ、割当直後から `%USERPROFILE%\DJSCBridgeProbe\bridge-<日時>.jsonl` とUDP 9007へ送る | Log.txt の `(DJSCBridgeProbe) Initializing...`、記録ファイル | `%APPDATA%\...\Preferences\User Remote Scripts\` はインスタントマッピング用で、ここに置いても一覧に出ない |
| テンポ自動化の試験Set | `artifacts/p0/sets/djsc-tempo-automation-test.als`。Live 12.1の既定テンプレート（`Resources\Builtin\Templates\DefaultLiveSet.als`）を展開し、MainTrack の Tempo（AutomationTarget Id 8）の envelope に FloatEvent（120 → 8拍目140 → 22拍目90 → 40拍目120）を書いて再圧縮。Liveで開いて自動化が効くことを確認 | Liveのテンポ表示（6 s 140、14 s 90、26 s 120） | 手動のテンポ変更をArrangement録音で自動化として記録する操作は、この環境では記録されなかった（原因未確認） |
| Live 12のMIDIポート名 | 入力：`<PC名>`、`<PC名> (Port 2)`〜`(Port 6)`、`Computer Keyboard`。出力：`Microsoft GS Wavetable Synth`、`<PC名>`、`(Port 2)`〜`(Port 6)`。WinMM列挙順（`ports`）と同じ並びで、`(Port 6)` = `DJSC Clock` をClock出力で実測確認 | Log.txtの `AMidiIO: Midi Devices`、`win-portmap-02` | `(Port 4)` = `DJSC Control`、`(Port 5)` = `DJSC Timecode` は並び順からの推定 |
| LiveのMIDI設定（2026-10-28 03:20時点） | 入力 `<PC名>`：Track ON（rtpMIDI、未使用）。入力 `(Port 2)`〜`(Port 6)`：Track/Sync/Remote OFF。出力 `(Port 6)`：Sync ON（Song、Resync "Stop and Start"）。Control Surface 7枠すべてNone | Log.txt | Remote ONは `route-check` 実装時に設定 |

## 依存crateの一次確認

| crate | 版 | 確認内容 |
| --- | --- | --- |
| midir | 0.11.1（crates.io最新、調査日） | WindowsはWinMMが既定、`winrt` featureでWinRT。Windowsでの仮想ポート作成API非対応は設計資料どおり。buildと列挙を確認 |
| rusty_link | 0.4.9（crates.io最新） | GPL-2.0-or-later。build.rsが `cmake` crateでLink SDKをビルドし、`bindgen 0.72` でabl_link.hからbindingを生成する。したがってcmakeとlibclangが必要。README記載のテスト済み環境はmacOS M1、Windows 11、Ubuntu 22 |
| midi-msg | 0.9.0 | P0ツールでは採用しない。MTC Quarter Frame / Full Frame / Clockの解析は少量なので自前実装にする |

## 未確認で残る項目

- Macの機種、macOS版、Xcode Command Line Tools、rustup target、Live 12 Suite版、Max版、Launchpad Xのfirmware。立ち会い枠で取得する。
- Link featureをWindowsでビルドするためのlibclang導入可否。
- Windows MIDI Services経由でのmulti-client挙動（Liveとアプリが同じloopMIDI入力を同時に開く構成は未検証）。
- Launchpad X接続時のWindows側ポート名と2系統（DAW / MIDI）の識別。
- Live 12.4がloopMIDIポートを `<PC名> (Port N)` と表示する理由（Windows MIDI Services経由の親デバイス名と推定、未確認）。

## Live 12初回起動時のMIDI認識（Log.txt、2026-10-28 00:24）

Liveが列挙したMIDIデバイスは rtpMIDI の `<PC名>`（Port 2、Port 3を含む）と `Microsoft GS Wavetable Synth` だけ。loopMIDIが停止していたため、loopMIDIポートは認識されていない。LiveはMIDIデバイスを起動時に列挙するので、仮想バスはLiveより先に起動する。Control Surfaceは7枠すべて `None`。

## 確認に使ったコマンド

PowerShell 7.6.9で実行。すべて読取専用。

```powershell
Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber
Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' | Select-Object DisplayVersion, UBR
Get-CimInstance Win32_Processor | Select-Object Name, NumberOfCores, NumberOfLogicalProcessors
rustc --version; cargo --version; rustup toolchain list; rustup target list --installed; cmake --version
& "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -all -products * -format json
Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*' | Where-Object DisplayName -match 'Ableton|Max|loopMIDI|Novation|ASIO|MIDI'
Get-ChildItem 'HKLM:\SOFTWARE\ASIO'
Get-PnpDevice -PresentOnly | Where-Object { $_.FriendlyName -match 'MIDI|Launchpad|Novation' }
Get-Service midisrv, rtpMIDIService
Get-NetAdapter | Where-Object Status -eq 'Up'; Get-NetFirewallProfile
```

midirのポート列挙は、scratchpadに置いた最小binary（`MidiInput::ports()` と `MidiOutput::ports()` を表示するだけ）で行った。機器への送信は行っていない。
