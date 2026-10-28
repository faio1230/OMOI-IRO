//! Local preview (M5, U03): a clock and renderer that belong to the UI's
//! preview only.
//!
//! * `LocalClock` is a play / pause / seek clock on the session monotonic
//!   time. It never feeds the `Transport`, never sends anything to Live,
//!   Link or a device, and never moves an external position.
//! * `LocalPreview` renders a show at the local clock with a beat derived
//!   from a preview tempo (`show.preview.bpm` unless changed), so Pulse can
//!   be seen without Live. While the runtime is following, the UI shows this
//!   as an independent preview next to the runtime output.
//! * Frames are quantized like the device output (`DeviceFrame`), so the
//!   preview shows what the device would receive.

use omoiiro_core::model::{Content, Show};
use omoiiro_core::render::{PadHit, PadInput, Renderer};
use omoiiro_core::time::BeatState;
use omoiiro_core::validation::Issue;
use omoiiro_devices::adapter::DeviceFrame;

/// Play / pause / seek clock. Times are microseconds; `now_us` comes from
/// the caller's monotonic clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalClock {
    /// Position at `anchor_us`.
    position_us: i64,
    anchor_us: i64,
    playing: bool,
    /// Position wraps to 0 at this length when playing (None: no wrap).
    length_us: Option<i64>,
}

impl LocalClock {
    pub fn new(length_us: Option<i64>) -> LocalClock {
        LocalClock { position_us: 0, anchor_us: 0, playing: false, length_us: length_us.filter(|l| *l > 0) }
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn length_us(&self) -> Option<i64> {
        self.length_us
    }

    pub fn position(&self, now_us: i64) -> i64 {
        let p = if self.playing { self.position_us + (now_us - self.anchor_us).max(0) } else { self.position_us };
        match self.length_us {
            Some(l) => p.rem_euclid(l),
            None => p.max(0),
        }
    }

    pub fn play(&mut self, now_us: i64) {
        if !self.playing {
            self.anchor_us = now_us;
            self.playing = true;
        }
    }

    pub fn pause(&mut self, now_us: i64) {
        if self.playing {
            self.position_us = self.position(now_us);
            self.playing = false;
        }
    }

    /// New show length (after an edit); the position is kept.
    pub fn set_length(&mut self, length_us: Option<i64>) {
        self.length_us = length_us.filter(|l| *l > 0);
    }

    /// Jump (scrub). Clamped to `0..length`.
    pub fn seek(&mut self, now_us: i64, to_us: i64) {
        let max = self.length_us.map(|l| l - 1).unwrap_or(i64::MAX);
        self.position_us = to_us.clamp(0, max);
        self.anchor_us = now_us;
    }
}

/// End of the last timeline clip (None if the show has only continuous layers).
pub fn show_length_us(show: &Show) -> Option<i64> {
    show.layers
        .iter()
        .filter_map(|l| match &l.content {
            Content::Timeline { clips } => clips.iter().map(|c| c.end_us()).max(),
            Content::Continuous { .. } => None,
        })
        .max()
        .map(|e| e.min(i64::MAX as u64) as i64)
}

pub struct LocalPreview {
    renderer: Renderer,
    pub clock: LocalClock,
    bpm: f64,
    show_bpm: f64,
    /// Pad clicks on the preview grid (Ripple check; not device input).
    pads: PadInput,
}

impl LocalPreview {
    /// Validate and compile `show` (on the caller's thread).
    pub fn new(show: &Show) -> Result<LocalPreview, Vec<Issue>> {
        let renderer = Renderer::new(show)?;
        Ok(LocalPreview { renderer, clock: LocalClock::new(show_length_us(show)), bpm: show.preview.bpm, show_bpm: show.preview.bpm, pads: PadInput::default() })
    }

    /// Replace the show (after an edit), keeping the clock and the preview tempo.
    pub fn replace_show(&mut self, show: &Show) -> Result<(), Vec<Issue>> {
        self.renderer = Renderer::new(show)?;
        self.clock.set_length(show_length_us(show));
        self.show_bpm = show.preview.bpm;
        Ok(())
    }

    pub fn bpm(&self) -> f64 {
        self.bpm
    }

    pub fn show_bpm(&self) -> f64 {
        self.show_bpm
    }

    /// Preview tempo (does not change the show). Rejects non-positive values.
    pub fn set_bpm(&mut self, bpm: f64) -> Result<(), String> {
        if !bpm.is_finite() || !(20.0..=999.0).contains(&bpm) {
            return Err(format!("preview tempo must be 20..999 bpm, got {bpm}"));
        }
        self.bpm = bpm;
        Ok(())
    }

    /// Beat at show time `t` for the preview tempo (beat 0 at song start).
    pub fn beat_at(&self, t: i64) -> BeatState {
        BeatState { beat: t as f64 / 1e6 * self.bpm / 60.0, tempo_bpm: self.bpm, phase_valid: true }
    }

    pub fn render_at(&mut self, t: i64) -> DeviceFrame {
        let beat = self.beat_at(t);
        DeviceFrame::from_frame(&self.renderer.render(t, Some(beat)), 1.0)
    }

    /// A click on preview pad (x, y) at monotonic `now_us` (bottom-left origin).
    pub fn press(&mut self, x: u8, y: u8, now_us: i64) {
        self.pads.push(PadHit { x: x.min(7), y: y.min(7), velocity: 127, t_us: now_us });
    }

    /// `render_at` with the preview pad clicks (Ripple) at monotonic `now_us`.
    pub fn render_at_now(&mut self, t: i64, now_us: i64) -> DeviceFrame {
        let beat = self.beat_at(t);
        self.pads.advance(now_us);
        DeviceFrame::from_frame(&self.renderer.render_input(t, Some(beat), Some(&self.pads)), 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_clock_play_pause_seek_wrap() {
        let mut c = LocalClock::new(Some(10_000_000));
        assert_eq!(c.position(5_000), 0);
        c.play(1_000_000);
        assert_eq!(c.position(3_500_000), 2_500_000);
        c.pause(4_000_000);
        assert_eq!(c.position(9_000_000), 3_000_000);
        c.seek(9_000_000, 9_500_000);
        c.play(9_000_000);
        assert_eq!(c.position(10_000_000), 500_000, "wraps at the show length");
        c.seek(10_000_000, -5);
        assert_eq!(c.position(10_000_000), 0);
        c.seek(10_000_000, 99_000_000);
        assert_eq!(c.position(10_000_000), 9_999_999);
        assert_eq!(LocalClock::new(None).length_us(), None);
    }

    #[test]
    fn local_preview_renders_with_the_preview_tempo() {
        let show = omoiiro_store::load_str(include_str!("../../../examples/demo.show.json")).unwrap();
        let mut p = LocalPreview::new(&show).unwrap();
        assert_eq!(p.clock.length_us(), show_length_us(&show));
        assert_eq!(p.bpm(), show.preview.bpm);
        assert!(p.set_bpm(0.0).is_err());
        p.set_bpm(120.0).unwrap();
        assert_eq!(p.beat_at(1_000_000).beat, 2.0);
        // Same time, same frame (render is a function of time and beat).
        assert_eq!(p.render_at(12_000_000), p.render_at(12_000_000));
        assert_eq!(p.render_at(12_000_000).lit(), 64);
    }
}
