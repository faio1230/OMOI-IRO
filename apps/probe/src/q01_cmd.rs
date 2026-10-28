//! `q01`: device-less run-through (Q01) of the show workflow, headless, on
//! the same code the desktop app uses (`omoiiro_core::edit`, `omoiiro_store`,
//! `omoiiro_runtime::{preview, session}`):
//!
//! 1. Build a sample show from an empty one (layers, Solid / Chase / Pulse
//!    presets, overlapping clips with fades, a continuous layer, a fast
//!    Pulse that triggers the flashing warning and its removal), save it.
//! 2. Reload it, save unchanged (byte-identical), history starts empty.
//! 3. Preview at arbitrary times equals `omoiiro-probe render` at the same time.
//! 4. Follow a synthetic bridge (UDP, in this process) with a fake MIDI
//!    backend: edit adopted at a frame boundary, Blackout, brightness,
//!    safety ceiling, control fault and reset, reconnect all, stop.
//! 5. Load failures: semantic errors, unsupported versions, too large.
//! 6. Timing of the follow run (jitter, render, LED send p95).
//!
//! Everything here is synthetic: no Live, no device, no MIDI port. The GUI
//! parts (dialogs, pixels on screen, close confirmation, memory) are in
//! `apps/desktop/scripts/q01-gui.ps1`.

use omoiiro_core::edit::{new_show, EditCommand, History, Snap};
use omoiiro_core::model::{Axis, Blend, ChaseParams, Effect, MtcOutput, PulseParams, Show, SolidParams, Waveform};
use omoiiro_core::safety::FlashProfile;
use omoiiro_devices::adapter::DeviceFrame;
use omoiiro_midi::backend::MidiBackend;
use omoiiro_midi::fake::fake_backend;
use omoiiro_midi::ports::names;
use omoiiro_runtime::follow::SharedBackend;
use omoiiro_runtime::preview::LocalPreview;
use omoiiro_runtime::session::{FollowSession, LedOut, SessionConfig};
use serde_json::{json, Value};
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(clap::Args, Debug)]
pub struct Q01Args {
    /// Folder for the sample show, the failure files and the results.
    #[arg(long, default_value = "apps/desktop/docs/q01-files")]
    pub out: PathBuf,
    /// Seconds of following in step 4/6.
    #[arg(long, default_value_t = 12)]
    pub follow_seconds: u64,
}

struct Report {
    rows: Vec<Value>,
    failed: usize,
}

impl Report {
    fn check(&mut self, id: &str, title: &str, expect: &str, ok: bool, result: impl Into<String>) {
        let result = result.into();
        println!("[{}] {id} {title}: {result}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            self.failed += 1;
        }
        self.rows.push(json!({ "id": id, "title": title, "expect": expect, "result": result, "pass": ok }));
    }
}

fn solid(color: &str) -> Effect {
    Effect::Solid { version: 1, params: SolidParams { color: color.to_string().into() } }
}

fn chase(color: &str, period_us: u64, width: u32) -> Effect {
    Effect::Chase { version: 1, params: ChaseParams { color: color.to_string().into(), period_us: period_us.into(), width: width.into(), axis: Axis::X.into(), reverse: None } }
}

fn pulse(color: &str, beats_per_cycle: f64) -> Effect {
    Effect::Pulse { version: 1, params: PulseParams { color: color.to_string().into(), beats_per_cycle: beats_per_cycle.into(), depth: 1.0.into(), waveform: Waveform::Sine.into() } }
}

/// Apply and return the outcome's new ID (panics on rejection: a harness bug).
fn ed(h: &mut History, cmd: EditCommand) -> (Option<String>, Option<String>, Option<String>) {
    let o = h.apply(&cmd, Snap::Frame30).unwrap_or_else(|e| panic!("{} rejected: {e}", cmd.label()));
    (o.clip_id, o.layer_id, o.preset_id)
}

/// Step 1: the sample show from an empty one.
fn build_sample(r: &mut Report) -> History {
    let mut h = History::new_unsaved(new_show("Q01 サンプル"));
    r.check("1.1", "新規ショー", "空のショーが検証を通る、未保存", h.dirty() && h.current().layers.is_empty(), format!("layers {}, presets {}, dirty {}", h.current().layers.len(), h.current().presets.len(), h.dirty()));

    let (_, _, solid_id) = ed(&mut h, EditCommand::AddPreset { name: "青".into(), effect_type: "builtin.solid".into() });
    let solid_id = solid_id.unwrap();
    ed(&mut h, EditCommand::SetPreset { preset_id: solid_id.clone(), name: None, effect: Some(solid("#2040FF")) });
    let (_, _, chase_id) = ed(&mut h, EditCommand::AddPreset { name: "流れ".into(), effect_type: "builtin.chase".into() });
    let chase_id = chase_id.unwrap();
    ed(&mut h, EditCommand::SetPreset { preset_id: chase_id.clone(), name: None, effect: Some(chase("#00DCC8", 4_000_000, 2)) });
    let (_, _, pulse_id) = ed(&mut h, EditCommand::AddPreset { name: "拍".into(), effect_type: "builtin.pulse".into() });
    let pulse_id = pulse_id.unwrap();
    let types: Vec<&str> = h.current().presets.iter().map(|p| p.effect.type_name()).collect();
    r.check("1.2", "プリセット新規（Solid / Chase / Pulse）", "3種が1つずつ追加される", types.contains(&"builtin.solid") && types.contains(&"builtin.chase") && types.contains(&"builtin.pulse") && h.current().presets.len() == 4, format!("{types:?}"));

    let (_, bg, _) = ed(&mut h, EditCommand::AddLayer { name: "背景".into(), index: None });
    let bg = bg.unwrap();
    let (_, mv, _) = ed(&mut h, EditCommand::AddLayer { name: "動き".into(), index: None });
    let mv = mv.unwrap();
    let (_, beat, _) = ed(&mut h, EditCommand::AddContinuousLayer { name: "拍".into(), preset_id: pulse_id.clone(), index: None });
    let beat = beat.unwrap();
    ed(&mut h, EditCommand::SetLayer { layer_id: mv.clone(), name: None, enabled: None, opacity: Some(0.8), blend: Some(Blend::Add), target_id: None });
    ed(&mut h, EditCommand::SetLayer { layer_id: beat.clone(), name: None, enabled: None, opacity: Some(0.3), blend: None, target_id: None });
    // Background 0..20 s with fades, an overlapping clip 10..14 s on the same layer, chase 5..15 s.
    let (c1, _, _) = ed(&mut h, EditCommand::AddClip { layer_id: bg.clone(), preset_id: solid_id.clone(), start_us: 0, duration_us: 20_000_000 });
    ed(&mut h, EditCommand::SetClipTiming { clip_id: c1.clone().unwrap(), start_us: 0, duration_us: 20_000_000, fade_in_us: 1_000_000, fade_out_us: 2_000_000 });
    ed(&mut h, EditCommand::AddClip { layer_id: bg.clone(), preset_id: "preset-white".into(), start_us: 10_000_000, duration_us: 4_000_000 });
    let (c3, _, _) = ed(&mut h, EditCommand::AddClip { layer_id: mv.clone(), preset_id: chase_id.clone(), start_us: 5_000_000, duration_us: 10_000_000 });
    ed(&mut h, EditCommand::SetClipTiming { clip_id: c3.unwrap(), start_us: 5_000_000, duration_us: 10_000_000, fade_in_us: 500_000, fade_out_us: 500_000 });
    let s = h.current();
    r.check(
        "1.3",
        "clip 配置（重なり、フェード、常時レイヤー）",
        "3レイヤー（timeline 2、常時 1）、clip 3、重なりとフェードあり",
        s.layers.len() == 3 && omoiiro_core::validation::validate(s).is_empty(),
        format!("layers {}, undo depth {}", s.layers.len(), h.depth()),
    );

    // Safety: a fast pulse (16th notes at 120 bpm = 8 Hz) is warned, then removed.
    ed(&mut h, EditCommand::SetPreset { preset_id: pulse_id.clone(), name: None, effect: Some(pulse("#FFFFFF", 0.25)) });
    let w = FlashProfile::of(h.current()).warnings(h.current().preview.bpm, 3.0);
    r.check("1.4", "安全警告（高速 Pulse）", "0.25拍周期 × 120 bpm = 8 Hz > 3 Hz で警告1件", w.len() == 1 && (w[0].hz - 8.0).abs() < 1e-9, w.first().map(|x| x.reason.clone()).unwrap_or_default());
    ed(&mut h, EditCommand::SetPreset { preset_id: pulse_id, name: None, effect: Some(pulse("#FFFFFF", 1.0)) });
    let w = FlashProfile::of(h.current()).warnings(h.current().preview.bpm, 3.0);
    r.check("1.5", "安全警告の解除", "1拍周期（2 Hz）で警告0件", w.is_empty(), format!("警告 {} 件", w.len()));
    h
}

fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_default()
}

/// RGB (8 bit) of a device frame as the GUI shows it (RGB7 -> 0..255).
fn gui_rgb(f: &DeviceFrame) -> Vec<[u8; 3]> {
    f.rgb7.iter().map(|c| c.map(|v| ((v as u16 * 255 + 63) / 127) as u8)).collect()
}

fn render_cli(show: &Path, t: i64, beat: f64, tempo: f64) -> Result<Vec<[u8; 3]>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let out = std::process::Command::new(exe)
        .args(["render", "--show", &show.display().to_string(), "--at", &t.to_string(), "--beat", &beat.to_string(), "--tempo", &tempo.to_string(), "--json"])
        .output()
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("render output: {e}"))?;
    let leds = v["leds_bottom_left"].as_array().ok_or("no leds_bottom_left")?;
    leds.iter()
        .map(|h| {
            let s = h.as_str().unwrap_or("#000000");
            let c = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0);
            Ok([c(1), c(3), c(5)])
        })
        .collect()
}

/// Synthetic bridge (like `live-bridge/sim/fake_bridge.py`): plays from 0 at 60 Hz.
fn spawn_bridge(port: u16, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<u64> {
    std::thread::spawn(move || {
        let sock = UdpSocket::bind("127.0.0.1:0").expect("bridge socket");
        let t0 = Instant::now();
        let mut seq = 0u64;
        while !stop.load(Ordering::SeqCst) {
            seq += 1;
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            let rec = json!({"v": 2, "seq": seq, "kind": "listener", "event": "song_time", "song_ms": ms, "beats": ms / 500.0, "tempo": 120.0, "is_playing": true, "errors": []});
            let _ = sock.send_to(rec.to_string().as_bytes(), ("127.0.0.1", port));
            std::thread::sleep(Duration::from_micros(16_667));
        }
        seq
    })
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0").and_then(|s| s.local_addr()).map(|a| a.port()).unwrap_or(9077)
}

fn wait_until(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < limit {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn latest_frame(s: &FollowSession) -> Option<omoiiro_runtime::follow::FrameMsg> {
    std::thread::sleep(Duration::from_millis(80));
    let mut last = None;
    while let Some(f) = s.take_preview_frame() {
        last = Some(f);
    }
    last.or_else(|| {
        std::thread::sleep(Duration::from_millis(50));
        s.take_preview_frame()
    })
}

/// Step 4 and 6: follow with a fake backend and a synthetic bridge.
fn follow(r: &mut Report, show: &Show, seconds: u64) -> Value {
    let (be, ports) = fake_backend();
    ports.add_input("Pads");
    ports.add_output(names::CONTROL_PORT);
    ports.add_output(names::TIMECODE_PORT);
    let backend: SharedBackend = Arc::new(Mutex::new(Box::new(be) as Box<dyn MidiBackend>));
    let mut show = show.clone();
    show.sync.mtc_output = Some(MtcOutput { enabled: true }); // exercise the timecode binding too
    let port = free_udp_port();
    let cfg = SessionConfig {
        time_source: Default::default(),
        show: show.clone(),
        bind: format!("127.0.0.1:{port}"),
        clock_in: None,
        link: false,
        led_out: LedOut::Virtual,
        control: Some(("Pads".into(), names::CONTROL_PORT.into())),
        timecode_out: Some(names::TIMECODE_PORT.into()),
        record: None,
        meta: json!({"command": "q01"}),
        safety: Default::default(),
        identify_led: false,
    };
    let mut s = match FollowSession::start(cfg, backend) {
        Ok(s) => s,
        Err(e) => {
            r.check("4.0", "追従の開始", "開始できる", false, e);
            return json!(null);
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let bridge = spawn_bridge(port, stop.clone());
    let t_start = Instant::now();

    let locked = wait_until(Duration::from_secs(3), || {
        let st = s.status();
        st.transport.time_health.name() == "locked" && st.follow.output == Some("follow")
    });
    r.check("4.1", "合成ブリッジで追従開始", "3秒以内に locked / follow", locked, format!("{:?} / {:?}", s.status().transport.time_health.name(), s.status().follow.output));

    // Edit during follow: adopted at a frame boundary.
    let mut h = History::new(show.clone());
    let bg = show.layers[0].id.clone();
    // Keep the background lit (the later brightness checks read it): change its opacity.
    h.apply(&EditCommand::SetLayer { layer_id: bg, name: None, enabled: None, opacity: Some(0.9), blend: None, target_id: None }, Snap::None).unwrap();
    let f0 = s.status().follow.frame_index;
    let rev = s.submit_show(h.current()).unwrap();
    let mut adopted_at = None;
    wait_until(Duration::from_secs(1), || {
        let st = s.status();
        if st.follow.revision == rev {
            adopted_at = Some(st.follow.frame_index);
            true
        } else {
            false
        }
    });
    let frames = adopted_at.map(|f| f.saturating_sub(f0));
    r.check("4.2", "追従中の編集の反映", "検証済みの版が次の frame 境界で採用（数 frame 以内）", frames.is_some_and(|n| n <= 3), format!("rev {rev}, {frames:?} frame 後"));

    // Blackout: LEDs dark, the route to Live keeps working.
    s.set_blackout(true);
    ports.inject("Pads", &[0x90, 40, 100]);
    let blk = latest_frame(&s);
    let forwarded = wait_until(Duration::from_secs(1), || ports.sent(names::CONTROL_PORT).iter().any(|m| m == &vec![0x90, 40, 100]));
    r.check("4.3", "Blackout", "LED は黒、Live への転送は続く", blk.is_some_and(|f| f.eval.dark.map(|d| d.name()) == Some("blackout") && f.eval.frame == DeviceFrame::black()) && forwarded, format!("dark {:?}, forwarded {forwarded}", blk.map(|f| f.eval.dark.map(|d| d.name()))));
    s.set_blackout(false);

    // Brightness gain and the safety ceiling.
    // Frames come from a moving show time, so check the applied gain and the
    // range instead of comparing two different moments.
    let max = |f: &Option<DeviceFrame>| f.map(|f| f.rgb7.iter().flatten().copied().max().unwrap_or(0)).unwrap_or(0);
    s.set_brightness(0.5);
    let half = latest_frame(&s);
    let half_frame = half.map(|f| f.eval.frame);
    let gain = half.map(|f| f.eval.gain).unwrap_or(-1.0);
    r.check("4.4", "明るさ", "gain 0.5 が掛かり、全チャンネルが半分（64）以下", (gain - 0.5).abs() < 1e-6 && max(&half_frame) <= 64 && max(&half_frame) > 0, format!("gain {gain}, max {}", max(&half_frame)));
    s.set_brightness(1.0);
    s.shared().overrides.set_limits(&omoiiro_core::safety::SafetyLimits { max_brightness: 0.3, ..Default::default() });
    let capped = latest_frame(&s).map(|f| f.eval.frame);
    let cap7 = (0.3f32 * 127.0).round() as u8;
    r.check("4.5", "安全上限（輝度の天井）", "全チャンネルが 30% 以下（各色の天井）", max(&capped) <= cap7 && max(&capped) > 0, format!("max {} (上限 {cap7})", max(&capped)));
    s.shared().overrides.set_limits(&Default::default());

    // Control fault (oversize input) and reset.
    ports.inject("Pads", &vec![0xF0; 2000]);
    let faulted = wait_until(Duration::from_secs(2), || s.status().control_fault.is_some());
    let cause = s.status().control_fault.unwrap_or_default();
    s.reset_control_fault();
    let cleared = wait_until(Duration::from_secs(2), || s.status().control_fault.is_none());
    ports.inject("Pads", &[0x90, 41, 100]);
    let again = wait_until(Duration::from_secs(1), || ports.sent(names::CONTROL_PORT).iter().any(|m| m == &vec![0x90, 41, 100]));
    r.check("4.6", "操作経路の fault と reset", "原因つきで fault、reset で転送が戻る", faulted && cleared && again, format!("fault {cause:?}, reset {cleared}, forward {again}"));

    // Reconnect all: the timecode output gets a new connection generation.
    let gen0 = s.status().follow.timecode_generation;
    s.reconnect_all();
    let regen = wait_until(Duration::from_secs(2), || {
        let g = s.status().follow.timecode_generation;
        g.is_some() && g != gen0
    });
    let ctl_ok = wait_until(Duration::from_secs(2), || s.status().follow.control_out == Some("ready") && s.status().follow.control_in == Some("ready"));
    r.check("4.7", "全ポート再接続", "タイムコード出力の接続世代が変わり、操作経路は ready に戻る", regen && ctl_ok, format!("timecode gen {:?} -> {:?}, control ready {ctl_ok}", gen0, s.status().follow.timecode_generation));

    // Let it run, then stop.
    while t_start.elapsed() < Duration::from_secs(seconds) {
        std::thread::sleep(Duration::from_millis(100));
    }
    let mtc_sent = ports.sent(names::TIMECODE_PORT).len();
    stop.store(true, Ordering::SeqCst);
    let datagrams = bridge.join().unwrap_or(0);
    let end = s.stop().unwrap_or_else(|e| json!({ "error": e }));
    let released = ports.sent(names::CONTROL_PORT).iter().any(|m| m == &vec![0x80, 41, 0]);
    r.check("4.8", "停止", "Note Off を送って停止、要約が出る", released && end["render"]["frames"].as_u64().unwrap_or(0) > 0, format!("frames {}, Note Off {released}, MTC {mtc_sent} 件, datagrams {datagrams}", end["render"]["frames"]));

    // Step 6: timing.
    let p95 = |v: &Value| v["p95_us"].as_i64().unwrap_or(-1);
    let jit = p95(&end["render"]["jitter"]);
    let ren = p95(&end["render"]["render_time"]);
    let snd = p95(&end["led"]["send_time"]);
    r.check("6.1", "計測（追従中、合成源）", "jitter / render / LED 送信の p95 を記録（render p95 ≤ 5 ms は NFR03）", ren >= 0 && ren <= 5_000, format!("jitter p95 {jit} us, render p95 {ren} us, LED 送信 p95 {snd} us, skipped {}", end["render"]["skipped_frames"]));
    end
}

pub fn run(args: Q01Args) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&args.out)?;
    let mut r = Report { rows: Vec::new(), failed: 0 };
    println!("Q01 (synthetic: no Live, no device, no MIDI port). Files in {}", args.out.display());

    // 1. Build and save as.
    let mut h = build_sample(&mut r);
    let sample = args.out.join("q01-sample.show.json");
    omoiiro_store::save_atomic(&sample, h.current())?;
    h.mark_saved();
    r.check("1.6", "名前を付けて保存", "一時ファイル→置換で保存、未保存の印が消える", !h.dirty() && sample.exists(), format!("{} ({} bytes)", sample.display(), read(&sample).len()));

    // 2. Reload, unchanged save, empty history.
    let loaded = omoiiro_store::load_path(&sample)?;
    let same = &loaded == h.current().as_ref();
    let before = read(&sample);
    let h2 = History::new(loaded.clone());
    omoiiro_store::save_atomic(&sample, h2.current())?;
    let identical = read(&sample) == before;
    r.check("2.1", "再読込", "保存したものと同じショーが読める", same, format!("equal {same}"));
    r.check("2.2", "無変更の保存", "バイト単位で一致（差分なし）", identical, format!("{} bytes, identical {identical}", before.len()));
    r.check("2.3", "Undo 履歴", "再読込後は空（元に戻す対象なし）、未保存の印なし", !h2.can_undo() && !h2.dirty(), format!("can_undo {}, dirty {}", h2.can_undo(), h2.dirty()));

    // 3. Arbitrary-position preview = render CLI.
    let mut lp = LocalPreview::new(&loaded).map_err(|e| format!("{e:?}"))?;
    let tempo = loaded.preview.bpm;
    let mut worst = 0i32;
    let mut all_ok = true;
    let times = [0i64, 750_000, 5_250_000, 10_500_000, 12_345_678, 19_000_000];
    for t in times {
        lp.clock.seek(0, t);
        let gui = gui_rgb(&lp.render_at(lp.clock.position(0)));
        let cli = render_cli(&sample, t, t as f64 / 1e6 * tempo / 60.0, tempo)?;
        // The GUI shows the device value (RGB 0..127 scaled to 0..255);
        // `render` prints 8-bit values of the same frame: within one 7-bit step.
        for (a, b) in gui.iter().zip(cli.iter()) {
            for k in 0..3 {
                let d = (a[k] as i32 - b[k] as i32).abs();
                worst = worst.max(d);
                all_ok &= d <= 2;
            }
        }
    }
    r.check("3.1", "任意位置プレビュー = render", "6時刻の64パッドで、プレビュー（機器の7bit値）と render（8bit）の差が1段（±2）以内", all_ok, format!("時刻 {:?} us、最大差 {worst}", times));

    // 4 / 6. Follow.
    let end = follow(&mut r, &loaded, args.follow_seconds);

    // 5. Load failures (the current show is never replaced: loading returns an error only).
    let mut fail_files = Vec::new();
    let mut broken = serde_json::to_value(&loaded)?;
    broken["layers"][0]["content"]["clips"][0]["preset_id"] = json!("missing");
    let p = args.out.join("q01-broken.show.json");
    std::fs::write(&p, serde_json::to_string_pretty(&broken)?)?;
    fail_files.push(p.clone());
    match omoiiro_store::load_path(&p) {
        Err(omoiiro_store::LoadError::Semantic(v)) => r.check("5.1", "壊れたファイル（意味検証）", "拒否、JSON Pointer つきの理由", v.iter().any(|i| i.pointer == "/layers/0/content/clips/0/preset_id"), v.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("; ")),
        other => r.check("5.1", "壊れたファイル（意味検証）", "拒否", false, format!("{other:?}")),
    }
    for (i, ver) in ["0.0.9", "0.2.0"].iter().enumerate() {
        let mut v = serde_json::to_value(&loaded)?;
        v["schema_version"] = json!(ver);
        let p = args.out.join(format!("q01-version-{ver}.show.json"));
        std::fs::write(&p, serde_json::to_string_pretty(&v)?)?;
        fail_files.push(p.clone());
        let res = omoiiro_store::load_path(&p);
        r.check(&format!("5.{}", i + 2), &format!("未対応の版 {ver}"), "読込前に版で拒否", matches!(res, Err(omoiiro_store::LoadError::UnsupportedVersion { .. })), match res {
            Err(e) => e.to_string(),
            Ok(_) => "loaded".into(),
        });
    }
    let big = args.out.join("q01-too-large.show.json");
    {
        let f = std::fs::File::create(&big)?;
        f.set_len(omoiiro_store::MAX_FILE_BYTES as u64 + 1)?;
    }
    let res = omoiiro_store::load_path(&big);
    r.check("5.4", "大きすぎるファイル", "16 MB を超えると拒否", matches!(res, Err(omoiiro_store::LoadError::TooLarge { .. })), res.err().map(|e| e.to_string()).unwrap_or_default());
    // The large file is not kept (it is just zeros).
    let _ = std::fs::remove_file(&big);

    let summary = json!({
        "q01": "synthetic run-through: no Live, no device, no MIDI port (fake backend, in-process UDP bridge)",
        "unix_s": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "build": if cfg!(debug_assertions) { "debug" } else { "release" },
        "rows": r.rows,
        "failed": r.failed,
        "follow_summary": end,
        "files": { "sample": sample, "failures": fail_files },
    });
    let out = args.out.join("q01-headless-result.json");
    std::fs::write(&out, serde_json::to_string_pretty(&summary)? + "\n")?;
    println!("result: {} rows, {} failed -> {}", summary["rows"].as_array().map(|a| a.len()).unwrap_or(0), r.failed, out.display());
    if r.failed > 0 {
        return Err(format!("{} check(s) failed", r.failed).into());
    }
    Ok(())
}
