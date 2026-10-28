//! Frame evaluation for the render loop (T06): show replacement at frame
//! boundaries and overrides.
//!
//! * `ShowExchange::submit` validates and compiles a show on the caller's
//!   thread (never the render thread). Only a show that passes becomes a new
//!   revision; the render thread swaps it in at the start of a frame.
//! * `Overrides` (Blackout, brightness) act on the final composite, after
//!   the show's master brightness and the transport output gain. Blackout
//!   affects LEDs only; the operation route to Live is independent (docs/01).
//! * `Engine::evaluate` follows `Frame = render(show_revision, show_time,
//!   beat_state, override_state)` using the transport snapshot's output
//!   state: follow / hold render the snapshot's show time, black is black.

use omoiiro_core::model::Show;
use omoiiro_core::render::{PadInput, Renderer};
use omoiiro_core::time::ShowTimeUs;
use omoiiro_core::validation::Issue;
use omoiiro_devices::adapter::DeviceFrame;
use crate::sync::transport::{OutputState, TransportSnapshot};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use omoiiro_core::safety::{cap_frame, FlashProfile, FlashWarning, SafetyLimits};
use std::sync::Mutex;

/// Shared override state (written by commands / triggers, read per frame).
#[derive(Debug)]
pub struct Overrides {
    blackout: AtomicBool,
    brightness_bits: AtomicU32,
    /// Terminal safety limits (docs/12): brightness ceiling, flashing limit.
    max_brightness_bits: AtomicU32,
    max_flash_hz_bits: AtomicU64,
    limit_flash: AtomicBool,
}

impl Default for Overrides {
    fn default() -> Self {
        let d = SafetyLimits::default();
        Overrides {
            blackout: AtomicBool::new(false),
            brightness_bits: AtomicU32::new(1.0f32.to_bits()),
            max_brightness_bits: AtomicU32::new((d.max_brightness as f32).to_bits()),
            max_flash_hz_bits: AtomicU64::new(d.max_flash_hz.to_bits()),
            limit_flash: AtomicBool::new(d.limit_flash),
        }
    }
}

impl Overrides {
    pub fn set_blackout(&self, on: bool) {
        self.blackout.store(on, Ordering::SeqCst);
    }

    pub fn blackout(&self) -> bool {
        self.blackout.load(Ordering::SeqCst)
    }

    /// Output brightness gain 0..=1 (clamped), applied after compositing.
    /// The safety ceiling is separate: it clips channels after this gain, so
    /// a 20 % ceiling does not also scale the whole frame by 0.2.
    pub fn set_brightness(&self, b: f32) {
        let b = if b.is_nan() { 0.0 } else { b.clamp(0.0, 1.0) };
        self.brightness_bits.store(b.to_bits(), Ordering::SeqCst);
    }

    /// Install the terminal's safety limits.
    pub fn set_limits(&self, l: &SafetyLimits) {
        self.max_brightness_bits.store((l.max_brightness.clamp(0.0, 1.0) as f32).to_bits(), Ordering::SeqCst);
        self.max_flash_hz_bits.store(l.max_flash_hz.to_bits(), Ordering::SeqCst);
        self.limit_flash.store(l.limit_flash, Ordering::SeqCst);
    }

    pub fn limits(&self) -> SafetyLimits {
        SafetyLimits { max_brightness: self.max_brightness() as f64, max_flash_hz: f64::from_bits(self.max_flash_hz_bits.load(Ordering::SeqCst)), limit_flash: self.limit_flash.load(Ordering::SeqCst) }
    }

    pub fn max_brightness(&self) -> f32 {
        f32::from_bits(self.max_brightness_bits.load(Ordering::SeqCst))
    }

    pub fn brightness(&self) -> f32 {
        f32::from_bits(self.brightness_bits.load(Ordering::SeqCst))
    }
}

/// Hand-over of validated shows to the render thread.
#[derive(Default)]
pub struct ShowExchange {
    pending: Mutex<Option<Staged>>,
    last_revision: AtomicU64,
}

impl ShowExchange {
    pub fn new() -> ShowExchange {
        ShowExchange::default()
    }

    /// Validate and compile; on success the show becomes the next revision.
    pub fn submit(&self, show: &Show) -> Result<u64, Vec<Issue>> {
        let renderer = Box::new(Renderer::new(show)?);
        let rev = self.last_revision.fetch_add(1, Ordering::SeqCst) + 1;
        *self.pending.lock().unwrap() = Some(Staged { rev, renderer, flash: FlashProfile::of(show), preview_bpm: show.preview.bpm });
        Ok(rev)
    }

    fn take(&self) -> Option<Staged> {
        self.pending.lock().unwrap().take()
    }
}

/// A validated show waiting for the next frame boundary.
struct Staged {
    rev: u64,
    renderer: Box<Renderer>,
    flash: FlashProfile,
    preview_bpm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DarkReason {
    NoShow,
    NoTime,
    Transport,
    Blackout,
}

impl DarkReason {
    pub fn name(self) -> &'static str {
        match self {
            DarkReason::NoShow => "no_show",
            DarkReason::NoTime => "no_time",
            DarkReason::Transport => "transport",
            DarkReason::Blackout => "blackout",
        }
    }
}

/// One evaluated frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Evaluated {
    pub frame: DeviceFrame,
    pub revision: u64,
    pub show_time_us: Option<ShowTimeUs>,
    pub gain: f32,
    pub dark: Option<DarkReason>,
    /// True when the show was rendered (false for black shortcuts).
    pub rendered: bool,
}

/// Owned by the render thread.
pub struct Engine {
    renderer: Option<Box<Renderer>>,
    revision: u64,
    pub swaps: u64,
    flash: FlashProfile,
    preview_bpm: f64,
    /// Inputs of the last safety evaluation (revision, tempo, limits).
    safety_key: Option<(u64, f64, SafetyLimits)>,
    /// Current flashing warnings and muted layers (layer IDs).
    pub warnings: Vec<FlashWarning>,
    pub muted_layers: Vec<String>,
    /// Set when `warnings` / `muted_layers` changed (the caller publishes and clears it).
    pub safety_changed: bool,
    /// Pad hits for input-driven effects (Ripple); the render loop feeds and
    /// advances it before `evaluate`.
    pub pads: PadInput,
    /// Bridge meters for this frame, and whether they are fresh (0.1.3).
    pub audio: omoiiro_core::render::AudioInput,
    pub audio_fresh: bool,
    /// Frames whose rendering panicked (an effect bug): the previous frame
    /// was held instead, and the LED output, the operation route and the
    /// clocks kept running. `render_fault` is the last panic message.
    pub render_panics: u64,
    pub render_fault: Option<String>,
    last_frame: omoiiro_core::surface::Frame,
    /// Tests: make the next frames' rendering panic.
    #[cfg(test)]
    pub inject_panics: u32,
}

impl Default for Engine {
    fn default() -> Self {
        Engine::new()
    }
}

impl Engine {
    pub fn new() -> Engine {
        Engine {
            renderer: None,
            revision: 0,
            swaps: 0,
            flash: FlashProfile::default(),
            preview_bpm: 120.0,
            safety_key: None,
            warnings: Vec::new(),
            muted_layers: Vec::new(),
            safety_changed: false,
            pads: PadInput { now_us: 0, hits: Vec::with_capacity(PadInput::CAPACITY), history: Vec::with_capacity(PadInput::HISTORY_CAPACITY) },
            audio: Default::default(),
            audio_fresh: false,
            render_panics: 0,
            render_fault: None,
            last_frame: Default::default(),
            #[cfg(test)]
            inject_panics: 0,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Frame boundary: adopt the newest validated show, if any.
    pub fn adopt(&mut self, exchange: &ShowExchange) -> bool {
        match exchange.take() {
            Some(st) => {
                self.renderer = Some(st.renderer);
                self.revision = st.rev;
                self.flash = st.flash;
                self.preview_bpm = st.preview_bpm;
                self.safety_key = None;
                self.swaps += 1;
                true
            }
            None => false,
        }
    }

    /// Flashing warnings at `tempo_bpm` and muting when `limit_flash` is on.
    /// Recomputed only when the revision, the tempo (by > 0.05 bpm) or the
    /// limits change, so the render path does not allocate per frame.
    fn update_safety(&mut self, tempo_bpm: f64, limits: SafetyLimits) {
        let key = (self.revision, tempo_bpm, limits);
        if self.safety_key.is_some_and(|(r, t, l)| r == key.0 && (t - tempo_bpm).abs() <= 0.05 && l == limits) {
            return;
        }
        self.safety_key = Some(key);
        self.warnings = self.flash.warnings(tempo_bpm, limits.max_flash_hz);
        let mut muted = Vec::new();
        if let Some(r) = self.renderer.as_mut() {
            for i in 0..r.layer_count() {
                let m = limits.limit_flash && self.flash.layer_hz(i, tempo_bpm) > limits.max_flash_hz;
                r.set_muted(i, m);
                if m {
                    muted.push(self.flash.layers[i].0.clone());
                }
            }
        }
        self.muted_layers = muted;
        self.safety_changed = true;
    }

    pub fn evaluate(&mut self, snap: &TransportSnapshot, ov: &Overrides) -> Evaluated {
        // Pulse speed depends on the tempo: measured while following, else the show's preview tempo.
        let tempo = snap.beat.map(|b| b.tempo_bpm).filter(|t| t.is_finite() && *t > 0.0).unwrap_or(self.preview_bpm);
        if self.renderer.is_some() {
            self.update_safety(tempo, ov.limits());
        }
        let dark = |reason, t| Evaluated { frame: DeviceFrame::black(), revision: self.revision, show_time_us: t, gain: 0.0, dark: Some(reason), rendered: false };
        if ov.blackout() {
            return dark(DarkReason::Blackout, snap.show_time_us);
        }
        let Some(renderer) = self.renderer.as_mut() else {
            return dark(DarkReason::NoShow, snap.show_time_us);
        };
        if snap.output == OutputState::Black {
            return dark(DarkReason::Transport, snap.show_time_us);
        }
        let Some(t) = snap.show_time_us else {
            return dark(DarkReason::NoTime, None);
        };
        let gain = snap.output_gain.clamp(0.0, 1.0) * ov.brightness();
        // A panic inside an effect must not stop the show: hold the previous
        // frame, count it and keep the message (reported as a fault).
        let (pads, audio) = (&self.pads, self.audio_fresh.then_some(&self.audio));
        #[cfg(test)]
        let inject = {
            let k = self.inject_panics > 0;
            self.inject_panics = self.inject_panics.saturating_sub(1);
            k
        };
        #[cfg(not(test))]
        let inject = false;
        let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if inject {
                panic!("injected render panic (test)");
            }
            renderer.render_all(t, snap.beat, Some(pads), audio)
        }));
        let mut frame = match rendered {
            Ok(f) => {
                self.last_frame = f;
                f
            }
            Err(p) => {
                self.render_panics += 1;
                let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown panic".into());
                self.render_fault = Some(format!("render panic at {t} us: {msg}"));
                self.last_frame
            }
        };
        // Overrides (transport gain, brightness), then the safety ceiling per
        // channel, then device quantization.
        if gain != 1.0 {
            for c in frame.leds.iter_mut() {
                *c = c.scale(gain);
            }
        }
        cap_frame(&mut frame, ov.max_brightness());
        Evaluated { frame: DeviceFrame::from_frame(&frame, 1.0), revision: self.revision, show_time_us: Some(t), gain, dark: None, rendered: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::bridge::BridgeHealth;
    use crate::sync::transport::{BeatHealth, Generations, TransportState};
    use omoiiro_core::time::BeatState;

    fn snap(t: i64, tempo: f64) -> TransportSnapshot {
        TransportSnapshot {
            now_us: t,
            show_time_us: Some(t),
            advancing: true,
            transport: TransportState::Playing,
            stalled: false,
            time_health: BridgeHealth::Locked,
            output: OutputState::Follow,
            output_gain: 1.0,
            beat: Some(BeatState { beat: t as f64 / 1e6 * tempo / 60.0, tempo_bpm: tempo, phase_valid: true }),
            beat_health: BeatHealth::Locked,
            beat_seeded: false,
            clock_seeded: 0,
            mtc_label: None,
            generations: Generations::default(),
        }
    }

    #[test]
    fn a_panicking_render_holds_the_last_frame_and_keeps_going() {
        let mut e = engine();
        let ov = Overrides::default();
        let good = e.evaluate(&snap(9_000_000, 120.0), &ov);
        assert!(good.rendered && good.frame.lit() > 0);
        e.inject_panics = 3;
        for k in 1..=3 {
            let held = e.evaluate(&snap(9_000_000 + k * 33_333, 120.0), &ov);
            assert_eq!(held.frame, good.frame, "the previous frame is held");
            assert!(held.rendered);
        }
        assert_eq!(e.render_panics, 3);
        assert!(e.render_fault.as_deref().is_some_and(|m| m.contains("injected render panic")), "{:?}", e.render_fault);
        // The next frame renders again.
        let next = e.evaluate(&snap(30_000_000, 120.0), &ov);
        assert!(next.rendered && next.frame != DeviceFrame::black());
        assert_eq!(e.render_panics, 3);
    }

    fn engine() -> Engine {
        let show = omoiiro_store::load_str(include_str!("../../../examples/demo.show.json")).unwrap();
        let ex = ShowExchange::new();
        ex.submit(&show).unwrap();
        let mut e = Engine::new();
        e.adopt(&ex);
        e
    }

    #[test]
    fn brightness_ceiling_clips_channels_before_quantization() {
        let mut e = engine();
        let ov = Overrides::default();
        // 10 s: blue base (0.6 master) and the cyan sweep; 0.5 s past a beat so the pulse is dark.
        let t = 10_250_000;
        let free = e.evaluate(&snap(t, 120.0), &ov).frame;
        ov.set_limits(&SafetyLimits { max_brightness: 0.2, ..SafetyLimits::default() });
        let capped = e.evaluate(&snap(t, 120.0), &ov).frame;
        let cap7 = (0.2f32 * 127.0).round() as u8;
        assert!(capped.rgb7.iter().flatten().all(|v| *v <= cap7), "no channel above the ceiling");
        let mut kept = 0;
        for (a, b) in free.rgb7.iter().flatten().zip(capped.rgb7.iter().flatten()) {
            if *a <= cap7 {
                assert_eq!(a, b, "channels under the ceiling are unchanged (not a gain)");
                kept += 1;
            }
        }
        assert!(kept > 0);
        // The brightness gain stays independent of the ceiling.
        assert_eq!(ov.brightness(), 1.0);
        ov.set_brightness(0.5);
        let dimmed = e.evaluate(&snap(t, 120.0), &ov).frame;
        assert!(dimmed.rgb7.iter().flatten().all(|v| *v <= cap7));
        assert!(dimmed.rgb7.iter().flatten().zip(capped.rgb7.iter().flatten()).all(|(a, b)| a <= b));
    }

    #[test]
    fn flashing_warning_follows_tempo_and_limit_flash_mutes() {
        let mut e = engine();
        let ov = Overrides::default();
        // On the beat: the demo's quarter-note pulse (continuous layer) is fully lit.
        let t = 30_000_000;
        let lit = e.evaluate(&snap(t, 200.0), &ov).frame;
        assert_eq!(e.warnings.len(), 1, "1 beat per cycle at 200 bpm = 3.33 Hz > 3 Hz");
        assert!(e.muted_layers.is_empty(), "warning only while limit_flash is off");
        assert!(e.safety_changed);
        e.evaluate(&snap(t, 120.0), &ov);
        assert!(e.warnings.is_empty(), "2 Hz at 120 bpm");
        ov.set_limits(&SafetyLimits { limit_flash: true, ..SafetyLimits::default() });
        let muted = e.evaluate(&snap(t, 200.0), &ov).frame;
        assert_eq!(e.muted_layers, vec!["beat".to_string()]);
        assert_ne!(lit, muted, "the flashing layer is not rendered");
        // Back under the limit: unmuted again.
        let again = e.evaluate(&snap(t, 120.0), &ov).frame;
        assert!(e.muted_layers.is_empty());
        assert_eq!(again, lit);
    }
}
