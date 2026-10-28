# OMOI-IRO デスクトップアプリ（M5）

Tauri 2 + React / TypeScript / Vite。製品名・ウィンドウ題名は OMOI-IRO、bundle identifier は `jp.faio.omoiiro`。ビルド手順と固定した版は [BUILD.md](../../BUILD.md)。

## 構成

| 場所 | 内容 |
| --- | --- |
| `src-tauri/src/lib.rs` | Runtime の起動・停止（`omoiiro_runtime::session::FollowSession` を1つ持つ）と IPC command。入力は Rust 側で検証する |
| `src-tauri/src/dto.rs` | IPC の型（JSONのキーは snake_case） |
| `src-tauri/src/settings.rs` | 接続・同期設定：端末設定の読込・確認・保存、入力の受信確認（何も送らない） |
| `src-tauri/src/editor.rs` | タイムライン編集：編集の履歴（Rust 側）、command の適用と検証、Runtime とローカルプレビューへの受け渡し |
| `src-tauri/src/files.rs` | 選ばれたパスの検証（拡張子 `.show.json` / `.preset.json`、サイズ上限、書き込み先のフォルダー）、読込エラーの JSON Pointer |
| `src-tauri/src/preview.rs` | LEDプレビュー：Tauri Channel への送信スレッド（最大30 fps、容量1）、ローカルプレビューの command |
| `src-tauri/build.rs` | command の一覧（app manifest）。command ごとに `allow-<command>` の permission ができる |
| `src-tauri/capabilities/default.json` | main ウィンドウに許可するのは上の command だけ。ファイル・シェル・HTTP 等の plugin は使わない |
| `src/ipc/types.ts` / `src/ipc/commands.ts` | IPC の型（dto.rs の手書きの写し。StatusDto のキーの一致は Rust のテストで検査）と呼び出し |
| `src/App.tsx` | 画面。タブ「演奏」（ショーと開始・停止、タイムコード、Blackout と明るさ、同期と接続の状態、診断、イベント、右に LEDプレビュー）と「接続設定」 |
| `src/ConnectionSettings.tsx` | 接続・同期設定の画面（役割ごとのポート、状態と理由、受信確認、保存） |
| `src/ShowInspectors.tsx` | レイヤーの設定、プリセットの一覧・新規・編集（範囲と単位つきの入力）・削除・読み込み・書き出し |
| `src/TimelineEditor.tsx` | タイムライン編集の画面（レイヤーの行、クリップのドラッグ、数値入力、Undo / Redo） |
| `src/LedPreview.tsx` | LEDプレビューの表示とローカル時計の操作 |

## 約束

- 画面は状態を表示するだけで、演出の時計を進めない。描画・LED出力・操作経路は Runtime のスレッドで動き、画面が重いときや最小化中も止まらない。状態は 200 ms ごとに `runtime_status` で取得する（イベント行はその間に溜まった分）。
- command は Rust 側で検証する（ショーファイルの読込と意味検証、UDPアドレス、ポート指定、明るさ 0〜1）。エラーは `{ code, message }`。
- 停止とアプリ終了では Runtime を止める（LED消灯、鳴っている note の Note Off）。
- LED出力に Launchpad らしいポートは指定できない（実機は M6）。既定は Runtime 内の仮想 Launchpad X。
- LEDプレビューに表示するのは Rust が計算したフレームだけ（画面側でエフェクトを計算しない）。Tauri Channel で送り、画面が描画して `preview_ack` を返すまで次を送らない（容量1）。その間に作られたフレームは置き換わり、件数を「間引き」として表示する。送信は最大30 fps。ack が1秒来ないとき（画面の再読込など）は枠を空ける。
- プレビューの表示元は2つ。「Runtime出力」は render スレッドが LED writer に渡したフレーム（transport の gain、Blackout、明るさを適用し、機器と同じ RGB7 に量子化した値）。「ローカルプレビュー」は画面専用の時計とテンポで描画したもの（`omoiiro_runtime::preview`）。
- ポート、ブリッジ受信アドレス、Link の使用は端末設定（`%APPDATA%\jp.faio.omoiiro\terminal-settings.json`）に保存し、show には書かない（docs/04「端末設定との分離」）。`runtime_start` はショーファイルのパスだけを受け取り、保存済みの端末設定を確認してから使う。
- 役割は5つ：操作入力 `control_in`、Live操作出力 `control_out`、MIDI Clock入力 `clock_in`、LED出力 `led_out`（空欄 = 仮想 Launchpad X）、タイムコード出力 `timecode_out`（show の `sync.mtc_output.enabled` のときだけ MTC を送る）。ポートは完全一致名か `#番号` で指定し、部分一致は使わない。`#番号` は起動順で変わるので警告を出す。
- 確認（`settings_check`）はポートの列挙と照合だけで、何も送らない。理由つきで拒否するもの：同じ名前のポートが複数（Fault、`#番号` で選び直す）、逆流（`control_out` / `timecode_out` が `control_in` / `clock_in` と同じポート）、同じ向きの二重割当、Launchpad らしい名前の `led_out`（M6まで）・`control_out`・`timecode_out`、IP:ポートでないブリッジアドレス。今ないポートは警告だけで保存・開始でき、現れたら接続する。拒否がある間は保存も開始もできない。
- 受信確認（`settings_listen`）は入力の役割を3秒だけ開いて届いたものを種類別に数える（Clock、Start/Stop/SPP、MTC QF、SysEx、Note、CC）。何も送らない。Runtime がポートを持っている間は使えない。
- 保存は同じディレクトリーの一時ファイルに書いてから置き換える。読めない設定ファイルは書き換えず、理由を表示して既定値で開く（保存したときに置き換わる）。最近使ったショー（最大10件）は開始時に記録する。
- 演奏タブの状態は、時刻源（ブリッジ）、拍源、LED出力、操作経路、タイムコード出力を別々のタイルで示す（接続、再接続待ち、Fault、未使用）。接続世代、最後のエラー、操作経路の fault の原因（入力 queue の overflow、大きすぎる入力、送信失敗、出力の消失）と保留中の Note Off を表示する。fault は「fault を解除」で戻す。
- 「全ポートを再接続」は、鳴っている note に Note Off を送ってから、LED出力、操作経路、タイムコード出力を閉じて開き直す（新しい接続世代）。ポートの抜き差しは押さなくても自動で再接続する。タイムコード出力は再接続すると MTC の Full Frame から送り直す。
- 診断は Runtime の各スレッドが約1秒ごとに公開する累計：frame の遅れ（jitter）、render 時間、LED 送信時間、render → LED 送信完了、受信 callback → Live へ送信の p50 / p95 / p99 / max と、frame・飛ばした frame・位置の飛び・mailbox 上書き・LED / Live / MTC の送信と失敗・各 queue の破棄・プレビュー間引きの件数。
- 画面が重くても演出と Live への転送は止まらない。`omoiiro-runtime` の `heavy_gui_does_not_stop_render_or_forwarding`（fake backend）で、状態とプレビューを4スレッドで休みなく取得し、2スレッドで CPU を回しながら 100 件の Note を流して、frame の速度（30 fps の8割以上）と 100 件すべての転送を検査している。
- タイムライン編集（タブ「タイムライン」）：クリップの追加・移動（時間、別のレイヤーへ）・長さ変更・複製・削除、数値入力（開始・長さ・フェードを秒で。フレーム数も表示）、レイヤーの追加・削除・並び替え。スナップは「なし / 1 秒 / フレーム（30 fps）」。キーは Ctrl+Z、Ctrl+Y（Ctrl+Shift+Z）、Delete、Ctrl+D。
- 編集は `omoiiro_core::edit::EditCommand` として Rust に送り、コピーに適用して意味検証を通ったものだけを採用する（不正な編集は何も変えず、理由を表示）。履歴は Rust 側で show 全体のスナップショットを最大200件持つ。保存（U07）までは「未保存の変更あり」と表示する。
- Runtime が同じショーファイルを追従中なら、編集・元に戻す・やり直すのたびに通った版を `submit_show` で渡し、次の frame 境界で差し替える（画面に反映済みの revision を表示）。開始時にタイムラインで編集中なら、ファイルではなく編集中の版で開始する。ローカルプレビューが同じファイルを表示していれば、時計の位置を保ったまま差し替える。
- レイヤーとプリセット（タイムラインタブ）：レイヤーの名前・有効・不透明度（0〜1）・合成（over / add / max）・出力先、常時レイヤーの追加とプリセットの割り当て。プリセットの一覧（色、種類、使用箇所数）、新規（単色 / 流れ / 拍の明滅、既定値つき）、名前とパラメーターの編集（色 #RRGGBB、周期 0.001〜3600 秒、帯の幅 1〜8、方向、1周期の拍数 0.25 / 0.5 / 1、深さ 0〜1、波形）、削除（使用中は拒否し、使っているクリップとレイヤーを表示）。
- クリップのプリセットは差し替えられる。「このクリップだけ効果を編集」は、プリセットが他でも使われていれば新しい ID へ複製してこのクリップだけを差し替える（docs/04）。プリセット一覧から開いた編集は共有の編集で、使用中の全箇所が変わる（画面に箇所を表示）。
- プリセットの読み込み・書き出しは 0.1.1 のプリセットファイル（kind `omoiiro.preset`）。読み込みは ID が衝突したら別名（`<id>-2`…）かエラーを選ぶ。書き出しは上書きの確認つき（一時ファイルに書いてから置き換え）。どちらも当面はパスの入力（ファイル選択ダイアログは U07）。
- 安全上限（端末設定の `safety`、show には書かない）：`max_brightness`（0.05〜1、既定1）は render と上書き（transport の gain、明るさ）の後、量子化の前に各色チャンネルの天井として切る（全体を暗くする係数ではない）。明るさスライダーは gain のままで、上限を横に表示する。`max_flash_hz`（0.5〜30、既定3）を超える点滅は show の静的解析で判定する：Pulse は tempo / 60 / 1周期の拍数（追従中は実測の tempo、拍源がないときと編集中は show の `preview.bpm`）、Chase は (8 / 帯の幅) / 周期。超える preset を使うレイヤーとクリップに、演奏タブ（「安全」タイル）とタイムラインで理由と値を示す。`limit_flash`（既定 false）が true のときだけ、そのレイヤーを不透明度 0 として描画する（show は書き換えない）。
- 保存と復旧（U07）：保存は同じフォルダーの一時ファイルに書いて flush・sync してから置き換える（Windows では置換つきの rename）。失敗したら元のファイルを残し、理由を表示して未保存のままにする。無変更の保存はバイト単位で同じ内容になる（安定整形）。ヘッダーに「● 未保存」と保存・名前を付けて保存（Ctrl+S / Ctrl+Shift+S）。未保存のままウィンドウを閉じると、閉じる前に「保存して終了 / 保存せずに終了 / キャンセル」を聞く（保存に失敗したら閉じない）。
- 読込に失敗したとき（JSON、Schema、意味検証、未対応の版、大きすぎる、拡張子違い）は今のショーをそのまま残し、理由と JSON Pointer の一覧を表示する。編集中に未保存の変更があるときは、別のショーを開く前に保存を求める。
- ファイル選択は dialog plugin（`@tauri-apps/plugin-dialog` 2.8.4 / `tauri-plugin-dialog` 2.8.4）。権限は `dialog:allow-open` と `dialog:allow-save` だけ。plugin はパスを返すだけでファイルへのアクセス権を与えないので、選ばれたパスは Rust 側で検証してから読み書きする。「最近使ったショー」（端末設定、最大10件）から選んで開ける。
- 機器への LED 出力（M6）：端末設定 `devices.allow_launchpad_led`（既定 false、接続設定タブの「Launchpad X へ LED を送る」）が有効なときだけ、LED 出力に Launchpad らしいポートを指定できる（確認と Runtime の両方）。有効にしたときと、それで追従を始めたときに「Launchpad X へ直接 LED を送ります。Live の Control Surface に Launchpad を割り当てないでください」を表示する。Live 操作出力とタイムコード出力に Launchpad は引き続き指定できない。機器は接続ごとに確認してから送るので、操作入力にも同じ Launchpad を指定する（しないと警告）。
- ローカル時計の再生・一時停止・位置（scrub）は、ローカルプレビューだけを動かす。Live、Link、機器へは何も送らず、Runtime の Transport にも触れない。Runtime の追従中に scrub すると表示はローカルプレビューに切り替わり、「独立プレビュー」と表示する。プレビューのテンポ（初期値は show の `preview.bpm`）は、Runtime が止まっているときだけ変えられる。

## 進捗

| 項目 | 状態 |
| --- | --- |
| U01 Tauri shell と IPC | 2026-10-28 完了。command 8個（app_info、list_ports、runtime_start、runtime_stop、runtime_status、set_blackout、set_brightness、reset_control_fault）。合成ブリッジ（`fake_bridge.py`）で起動 → 開始 → 追従 → 停止 → 終了（exit 0）を確認。画面は `screenshots/u01-runtime.png` |
| U03 LEDプレビューとローカル時計 | 2026-10-28 完了。command 7個を追加（preview_subscribe、preview_ack、preview_open、preview_mode、preview_play、preview_seek、preview_set_bpm）。合成ブリッジでの Runtime出力の表示（30.8 s の sweep）と、Runtime 停止中のローカル再生（120 bpm、10 秒で seq 312、間引き 0）を確認。画面は `screenshots/u03-preview-follow.png`、`screenshots/u03-preview-local.png` |
| U02 接続・同期設定 | 2026-10-28 完了。command 4個を追加（settings_get、settings_check、settings_save、settings_listen。合計19個）。`runtime_start` の要求はショーファイルだけになった。セッションに MTC 出力（タイムコード出力の役割）を接続。誤設定の拒否理由と、rtpMIDI 入力での3秒の受信確認（0件）を確認。画面は `screenshots/u02-connections.png` |
| U06 演奏画面と診断 | 2026-10-28 完了。command 1個を追加（reconnect_ports。合計20個）。タイムコード出力の再接続（`output::reconnect`）、Runtime の診断の常時公開（`LiveDiag`）、全ポート再接続の要求。合成ブリッジで locked・follow・30.7 s の sweep と診断値の表示を確認。画面は `screenshots/u06-perform.png` |
| U04 Timeline 編集と Undo / Redo | 2026-10-28 完了。command 5個を追加（editor_open、editor_view、editor_apply、editor_undo、editor_redo。合計25個）。`omoiiro-core` に `Param<T>`（効果パラメーターのタグ付き内部型、保存形式は 0.1.x のまま）と `edit`（command、スナップ、スナップショット履歴）。合成ブリッジで追従中にレイヤーとクリップを追加し、Runtime が rev 3 に差し替えたことを確認。画面は `screenshots/u04-timeline.png` |
| U05 Layer と Preset の編集 | 2026-10-28 完了。command 2個を追加（preset_import、preset_export。合計27個）。EditCommand に9種を追加（set_layer、add_continuous_layer、set_layer_preset、add_preset、set_preset、delete_preset、insert_preset、set_clip_preset、edit_clip_effect）。編集はすべて履歴と Runtime への反映の経路に乗る。画面は `screenshots/u05-presets.png` |
| 安全上限（輝度・点滅） | 2026-10-28 完了。command と capability の増減なし（27個）。`omoiiro_core::safety`（上限値、点滅周波数、レイヤーごとの点滅プロファイル、警告、天井クリップ）、Renderer のレイヤー消灯、Engine での天井と警告。画面は `screenshots/safety-perform.png` |
| U07 保存とエラー復旧 | 2026-10-28 完了。command 3個を追加（editor_save、editor_save_as、close_decision。合計30個）。capability は 27 → 32（command 3個と dialog:allow-open / dialog:allow-save）。画面は `screenshots/u07-load-error.png`、`screenshots/u07-unsaved-close.png` |
| M6 準備（機器なしでできる分） | 2026-10-28。LED 出力が機器のとき、接続ごとに Device Inquiry → Programmer Mode の確認 → Ready まで LED を送らない（`omoiiro_runtime::output::identify`、fake で検証）。release ビルドと NSIS インストーラー（BUILD.md）。手順書案は [docs/m6-runbook-draft.md](docs/m6-runbook-draft.md) |
| Q01 機器なしの通し試験 | 2026-10-28。報告は [docs/q01-report.md](docs/q01-report.md)（合成源）。headless 23項目と GUI 6項目が PASS、Windows の保存ダイアログへの入力（G1.2）は要手動確認。試験のために command 1個を追加（editor_new。合計31個、capability 33個）、LEDプレビューに「位置を指定（秒）」。手順は `omoiiro-probe q01` と `scripts/q01-gui.ps1` |
