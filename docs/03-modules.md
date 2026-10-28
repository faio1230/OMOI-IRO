# OMOI-IRO Rustクレートとモジュール構成

## 初期構成

以下は作成予定の構成であり、現時点ではCargo workspaceを生成しない。初期は5つのライブラリーとTauriアプリに留め、Effectごと・Triggerごとの細かなcrate分割は避ける。

```text
OMOI-IRO/
  Cargo.toml                         # workspace、共通依存
  crates/
    show-core/
      src/
        lib.rs
        model/                       # show、preset、clip、ID
        time/                        # ShowTime、BeatState、有理数fps
        timeline/                    # active clip検索、local time
        effects/                     # registry、solid、chase、pulse
        compose/                     # RGBA、blend、brightness
        surface/                     # LED ID、座標、Frame
        control/                     # Trigger/Actionのドメイン型
        validation/                  # ID参照・時間区間・能力の検証
    show-devices/
      src/
        lib.rs
        adapter.rs                   # DeviceAdapter契約
        launchpad_x/                 # layout、inquiry、mode、RGB codec
        simulated/                   # 仮想surface
        color/                       # RGB7・パレット量子化
    show-midi/
      src/
        lib.rs
        backend.rs                   # MidiBackend契約
        midir_backend/               # OS接続の実装
        message/                     # bytesと正規化イベント
        routing/                     # 許可ルール・active notes
        ports/                       # 列挙・役割・接続世代
    show-store/
      src/
        lib.rs
        json/                        # DTO、構造検証
        migration/                   # schema version移行
        presets/                     # import/export、ID解決
        save/                        # 安定整形、一時保存、置換
    show-runtime/
      src/
        lib.rs
        supervisor/                  # start/stop、状態監視
        sync/                        # MTC、Link/Clock、transport統合
        link/                        # optional SDK binding、clock対応、peer状態
        scheduler/                   # 単調時計、render周期
        output/                      # 最新frame、writer、差分
        commands/                    # 編集・接続・override
        snapshots/                   # UI用状態
        settings/                    # OS別port設定
        diagnostics/                 # 時間・送信・欠落の集計
  apps/desktop/
    src/                             # React / TypeScript
      features/connections/
      features/timeline/
      features/presets/
      features/preview/
      features/diagnostics/
      ipc/                           # command、snapshotの型
    src-tauri/
      src/lib.rs                     # Runtime組立、薄いcommand
      capabilities/
      tauri.conf.json
  schemas/
  examples/
  tests/fixtures/                     # 既知MIDI系列と期待値
  docs/
```

## crate間の依存

| crate | 依存先 | 持たせない責務 |
| --- | --- | --- |
| show-core | 最小の数値・シリアライズ関連のみ | Tauri、midir、OS時計、ファイル、thread生成 |
| show-devices | show-core | MIDIポートのopen、timer、ファイル保存 |
| show-midi | show-core、midir | エフェクト評価、GUI、show読込 |
| show-store | show-core、JSON検証関連 | MIDI送受信、render周期 |
| show-runtime | 上記4 crate | Tauri型、React型、WebView依存 |
| desktop shell | show-runtime、Tauri | LED合成の再実装 |

`show-core::time`は値・換算・snapshotを持ち、入ってきたbytesから時計を追跡する処理は`show-runtime::sync`に置く。この中のdecoderも純粋な状態機械として、I/Oなしでテスト可能にする。

## 境界インターフェース案

以下は責務を示す擬似Rustであり、コンパイル対象のAPI確定稿ではない。

```rust
trait Effect: Send + Sync {
    fn descriptor(&self) -> EffectDescriptor;
    fn render(&self, context: &RenderContext, output: &mut LayerFrame);
}

struct RenderContext<'a> {
    show_time_us: i64,
    local_time_us: i64,
    beat: Option<&'a BeatState>,
    surface: &'a Surface,
    seed: u32,
}

trait DeviceAdapter: Send {
    fn capabilities(&self) -> DeviceCapabilities;
    fn surface(&self) -> &Surface;
    fn decode_input(&self, message: &[u8]) -> Result<DeviceInput, DecodeError>;
    fn initialization(&self) -> Vec<MidiPacket>;
    fn encode_frame(&self, frame: &DeviceFrame,
                    last_sent: Option<&DeviceFrame>) -> Vec<MidiPacket>;
    fn shutdown(&self) -> Vec<MidiPacket>;
}

trait MidiBackend {
    fn list_ports(&self) -> Result<Vec<PortDescriptor>, PortError>;
    // 実際のconnection所有権・callback・OS制約はbackend実装で隠す。
    fn open_input(&mut self, port: PortId, ingress: Ingress) -> Result<Input, PortError>;
    fn open_output(&mut self, port: PortId) -> Result<Output, PortError>;
}

trait SyncSource {
    fn ingest(&mut self, event: TimedSyncEvent);
    fn snapshot(&self, now: MonotonicTime) -> TransportSnapshot;
}

trait Trigger {
    fn evaluate(&self, event: &InputEvent) -> Option<Action>;
}
```

Effectは読込時にパラメーター検証・必要な事前計算を済ませる。render内でのJSON解析や無制限なメモリー確保を禁止する。MVPでは組込みenumをregistryで生成してよく、動的dispatchは必要な境界に限定する。

Actionの実行はRuntimeに集約する。DeviceAdapterにOS connectionを渡さず、返したpacketをwriterへ渡す。エラーにはport role、device logical ID、接続generationを付け、GUI表示と復旧判断へ使う。

## 依存ライブラリー候補

| 用途 | 候補 | 方針 |
| --- | --- | --- |
| MIDI I/O | midir 0.11系 | 調査時0.11.1。採用時に両OSとSysExを検証しlockfile固定 |
| 拍同期 | Ableton Link SDK + rusty_link等のbinding | optional featureで隔離。S07で両OSビルド、SDK版、配布条件を確認 |
| データ | serde、serde_json | 共通DTOからの保存・読込。型のversionを明示 |
| Schema | Draft 2020-12対応validator、必要時schemars | 手書き案をreview後、Rust型との二重管理を解消 |
| エラー・診断 | thiserror、tracing | callback内で重いformat/log処理をしない |
| キュー | stdまたはbounded channel実装 | 容量・破棄規則を先に決め、lock-free化は測定後 |
| GUI | Tauri 2、React、TypeScript、Vite | 各採用版を固定。演出時計には使わない |

midirはWinMM/CoreMIDI/ALSA等とSysExを提供する。採用可能性の根拠は[公式crate資料](https://docs.rs/crate/midir/latest)。その他の候補は依存確定時にAPI・互換性・ライセンスを確認し、最新バージョンを無条件には固定しない。

TokioはMVPのrender loopに必須ではない。Tauriの非同期commandは外側の仕事に使い、正確な周期実行をasync timerに依存させない。まず単純な専用threadと有界通信で構成する。

Linkは初期のcrate数を増やさず`show-runtime::link`へ閉じ込める。C/C++依存をCoreへ漏らさない。`rusty_link`はAbletonのC wrapperへのRust bindingでありPure Rust実装ではない。採用条件は[Linkの採用案](08-ableton-link.md)にまとめる。

## 将来の分割タイミング

- F1追加時はまず`show-devices::kontrol_f1`を追加する。
- 複数DAWの専用連携が必要になったら`show-daw`を分離する。
- OSCが独立した入出力になる時点で`show-osc`を追加する。
- 外部プラグインは実行方式とABIを決めてから`show-plugin-host`を設ける。
- LTC変換を採用する場合は別crateまたは別アプリとし、Coreへaudio APIを持ち込まない。

## テストの配置

Coreは任意時刻・境界・合成・再現性を単体検証する。Device Adapterはgolden bytesで検証する。Syncは既知の時刻を持つ記録系列と不正系列をfixtureにする。Runtimeはfake backendとfake clockで切断・負荷・queue overflowを再現する。最後に両OSの実機試験でAPIやUSB・ドライバー由来の差異を確認する。
