# link-sss-probe

S07で見つかった問題の切り分け用。p0-probe `monitor --link --link-start-stop-sync` を同じセッションで動かすと、Live 12.4.7がPlay後に進まなくなった。その原因がLink本体なのか、rusty_link 0.4.9が生成する `ableton::LinkAudio` なのか、p0-probe固有なのかを分ける。

観測だけを行う最小のLink peer（C++）。p0-probeと同じ使い方でSDKを呼ぶ（enable、任意でStart Stop Sync、app session stateのcapture、commitしない）。tempo、beat、transportは設定しない。

| 実行ファイル | peerの型 | 用途 |
| --- | --- | --- |
| `link-sss-probe.exe` | `ableton::Link`（SDKの公式例LinkHutと同じ） | 素のLinkでも再現するか |
| `link-sss-probe-audio.exe` | `ableton::LinkAudio`（rusty_linkの `abl_link_create` と同じ。peer名は空文字） | LinkAudioだけで再現するか |

`LinkAudio.hpp` は `LINK_AUDIO` を定義し、SDK全体のコンパイル内容を切り替える。そのため、2つの型を1つの実行ファイルにまとめられない。

## ビルド（Windows、このPC）

SDKは、cargo registryにある rusty_link 0.4.9 同梱のもの（p0-probeが使うものと同じ）を指す。ダウンロードは行わない。

```powershell
& 'C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\Common7\Tools\Launch-VsDevShell.ps1' -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
$sdk = "$env:USERPROFILE/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rusty_link-0.4.9/link"
cmake -S tools\link-sss-probe -B <build dir> -G "NMake Makefiles" -DCMAKE_BUILD_TYPE=Release "-DLINK_SDK=$sdk"
cmake --build <build dir>
```

PowerShellでは `-DLINK_SDK=$sdk` を引用符で囲む（囲まないと変数が展開されない）。公式例の LinkHutSilent も同じSDKから `cmake -S $sdk -B <dir> -G "NMake Makefiles"` → `cmake --build <dir> --target LinkHutSilent` でビルドできる。ただしキー入力（`a` でLink有効化、`s` でStart Stop Sync）が必要で、Space（再生）や `w` / `e`（tempo変更）はセッションへ書き込むので押さない。

## 使い方

```powershell
<build dir>\link-sss-probe.exe --sss --seconds 30        # 素のLink、Start Stop Sync ON
<build dir>\link-sss-probe-audio.exe --sss --seconds 30  # LinkAudio、Start Stop Sync ON
<build dir>\link-sss-probe.exe --seconds 30              # 対照：Start Stop Sync OFF
```

出力：`event peers -> N` / `event tempo -> X` / `event is_playing -> true|false` と、500 msごとの `t= peers= tempo= beat= phase= is_playing=`。

## 切り分け手順（未実施。Liveの画面操作が必要）

1. Live 12.4.7でLink ON、Preferencesの開始停止同期 ON。DJSCBridge（またはDJSCBridgeProbe）を割り当て、`p0-probe bridge-clock` でLiveの進行を記録する（`stalled` が出ればLiveが進んでいない）。
2. `link-sss-probe.exe --sss` を起動し、`peers -> 1` を確認してからLiveでPlay 8 s → Stop。
3. Liveが進まなければ、Liveを再起動して 4 へ。進めば `link-sss-probe-audio.exe --sss` で同じ手順。
4. 各回の後に、対照（`--sss` なし）で正常に進むことを確認する。
5. 判定：素のLinkで再現すれば、Live側（またはLinkの仕様）の問題。LinkAudioだけで再現すれば、rusty_linkがLinkAudioを生成していることが原因。どちらでも再現しなければ、p0-probe側を疑う。

Liveが固まった場合の復旧（設計担当の記録）：Ctrl+Q →「いいえ」→ 再起動。起動順は loopMIDI → Live。
