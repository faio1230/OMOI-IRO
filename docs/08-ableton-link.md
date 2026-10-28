# OMOI-IRO Ableton Linkの採用案

## 判断

利用者から「Liveの同期機構が使えるなら利用してよい」との方針を受け、Ableton Linkを拍・テンポ同期の第一候補にする。確認日：2026年10月27日。前稿の「Linkは将来候補」を変更した。実装は未着手。

Linkはテンポ・拍位相・任意の開始停止共有に利用できる。一方、各アプリの曲中タイムラインは独立している。共通タイムコードと途中位置へのseek追従という既存要件を維持するため、MTCの時刻源と併用する。[Ableton Link開発資料](https://ableton.github.io/link/)

| 目的 | 推奨入力 |
| --- | --- |
| BPM・四分音符に反応するLED | Ableton Link |
| セット全体の何分何秒かに対応する演出 | Liveブリッジの曲中時刻サンプル（[10-live-bridge.md](10-live-bridge.md)、2026-10-28改訂）。MTCはアプリが外部機器向けに生成する |
| 再生・停止通知 | Liveブリッジの `is_playing`（代替はMIDI ClockのStart/Stop/Continue）。LinkのStart Stop Syncはアプリ側で有効にしない（S07でLive 12.4.7のトランスポートが進まなくなる問題） |
| パッド入力をLiveへ転送 | 従来どおりMIDI |
| Linkを使えない環境の拍同期 | 利用者がMIDI Clock方式を選択 |

Linkだけの拍演出モードは将来追加できるが、Linkだけで共通タイムコードの要件を満たしたとは扱わない。MTCを外す場合は、任意の途中位置からの復帰やseek追従をどの方法で補うか改めて決める。

## Liveを再生マスターとして使う運用

Linkは対等な参加者で構成され、プロトコルに「Liveだけが変更できるマスター」を指定する機能はない。そこで本アプリからtempo・拍位置・transportの変更を送らず、Liveと本アプリだけのsessionを基本とする。これはアプリの運用上の方針であり、Linkの排他的な受信専用権限ではない。[Ableton Link FAQ](https://help.ableton.com/hc/en-us/articles/209776125-Link-features-and-functions-FAQ)

起動はLiveのLinkを先に有効にし、その後アプリが参加する。アプリを先に起動した場合はLinkへの参加を明示操作まで待つ。S07（2026-10-28）の実測では、アプリが先にセッションを作っているとLive参加時にアプリの既定tempo（120）がLiveへ採用された。順序の規則は必須であり、本番アプリでは「peerを検出するまでLinkを有効にしない」等の仕組みを設計項目にする。別のLinkアプリからのtempo変更はLiveにも影響し得るため、本番では他peerを混在させない。peer数は表示するが、その数だけで相手がLiveだと断定しない。

## 再生途中の参加と停止

Linkは参加時に拍とtempoを同期できるが、既に再生している相手のtransport状態へ自動追従する契約ではない。開始停止の共有は参加後のユーザー操作が対象であり、Linkの拍時計は停止中でも進む。[Link C APIの状態規約](https://github.com/Ableton/link/blob/master/extensions/abl_link/include/abl_link.h)

したがって、初期の`is_playing=false`をLiveの停止通知とみなさない。MTCが進行していれば共通演出をその位置へ追従させる。Link接続・拍位相の有効性と、transportの有効性を別々に持つ。MTCが失われてもLinkが動いていることを理由に共通時刻を自走させない。

Stopを受けた後、送信経路の遅延で数個のMTCが届いても即座に再生へ戻さない。受信時刻・event世代・送信元ごとの遅延を考慮して調停する。継続したMTC進行とStopが矛盾する場合は同期競合を表示し、最後に有効だった位置で保持して既定の同期断処理へ進む。逆にPlayだけ届きMTCがない場合も、曲中位置をゼロと推測しない。

## Rustからの利用

候補は公式Link SDKのC wrapperをRustから利用する方法。`rusty_link`はそのbindingを提供している。SDKのclock、session snapshot、tempo、phaseをRuntimeの型へ変換し、CoreにはSDK型を公開しない。[rusty_linkの公式API資料](https://docs.rs/rusty_link/latest/rusty_link/)

`show-runtime::link`をoptional featureにしてC/C++ビルド依存を隔離する。まず拍同期だけを使用し、Link Audioは導入しない。SDK版とwrapper版を固定してWindowsとmacOSで検証する。SDKはGPLv2+と独自ライセンスの選択肢を公表しているため、配布形態に適合する条件を依存採用時に確認する。[Ableton公式SDK](https://github.com/Ableton/link)

## OSと接続の検証

MIDI Clock用の仮想バスは不要になる。ControlとTimecodeの仮想MIDI経路は残る。同一PCでまず検証し、ネットワーク越しの演奏は後続にする。

WindowsではLiveのオーディオdriverがLinkの利用条件に関係する。AbletonのFAQはASIO使用を案内しているため、対象環境のLive 12でLinkを有効化できるdriverをD01へ記録する。アプリ側で音声を処理するためのASIOエンジンを新規実装するという意味ではない。[Ableton Link FAQ](https://help.ableton.com/hc/en-us/articles/209776125-Link-features-and-functions-FAQ)

S07では両OSで、検出、tempo変更、位相、Start Stop Syncの有効/無効、再生途中の参加、Live終了、ネットワーク遮断、起動順変更、追加peerの参加を確認する。接続を失ってもSDKのローカル時計は進み得るので、値が更新されることを同期正常と判定しない。peer喪失の検出遅延とその後のholdoverを別計測する。

## 採用ゲート

S07でLinkの利用可否と安定性を確認し、S03/S04でMTCと組み合わせた両ビューの挙動を確認する。Linkが合格すれば標準の拍入力とし、不適合な環境ではMIDI Clockを明示選択する。自動切替は位相の飛びを生むためMVPに含めない。
