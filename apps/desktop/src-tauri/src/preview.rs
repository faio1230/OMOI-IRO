//! LED preview (U03): frames computed in Rust, pushed to the UI over a Tauri
//! Channel. The UI never computes effects.
//!
//! * Sources: `runtime` = the frame the render thread evaluated for the LED
//!   writer (after transport gain, Blackout and brightness, quantized like
//!   the device); `local` = the local preview (`omoiiro_runtime::preview`),
//!   its own clock and tempo, never connected to Live, Link or a device.
//! * Capacity 1, at most 30 fps: a new frame is sent only after the UI
//!   acknowledged the previous one (`preview_ack`); frames produced while
//!   one is in flight replace each other and are counted as skipped. An ack
//!   missing for 1 s (reloaded page) frees the slot.
//! * Local scrub and the preview tempo only change the local preview. While
//!   the runtime follows, the local preview is an independent view and the
//!   preview tempo cannot be changed (the show's tempo stays in effect).

use crate::dto::CommandError;
use crate::AppState;
use omoiiro_midi::clock::MonotonicClock;
use omoiiro_runtime::preview::LocalPreview;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

pub const PREVIEW_FPS: i64 = 30;
const ACK_TIMEOUT_US: i64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewSource {
    Runtime,
    Local,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreviewFrame {
    pub seq: u64,
    pub source: PreviewSource,
    pub show_time_us: Option<i64>,
    pub timecode: Option<String>,
    pub playing: bool,
    pub dark: Option<&'static str>,
    /// 64 x RGB, 0..255, index = y * 8 + x with y = 0 at the bottom row.
    pub rgb: Vec<u8>,
    /// Frames replaced while the previous one was not yet acknowledged.
    pub skipped: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreviewInfo {
    pub show_path: String,
    pub length_us: Option<i64>,
    pub bpm: f64,
    pub show_bpm: f64,
    pub position_us: i64,
    pub playing: bool,
    pub mode: PreviewSource,
}

struct Inner {
    mode: PreviewSource,
    local: Option<LocalPreview>,
    local_path: Option<String>,
    channel: Option<Channel<PreviewFrame>>,
}

pub struct PreviewState {
    inner: Mutex<Inner>,
    clock: MonotonicClock,
    in_flight: AtomicBool,
    in_flight_since: AtomicI64,
    seq: AtomicU64,
    skipped: AtomicU64,
    started: AtomicBool,
}

impl PreviewState {
    /// After an edit: if the local preview shows `path`, swap in the new show
    /// (clock position, play state and preview tempo are kept).
    pub fn replace_local_if(&self, path: &std::path::Path, show: &omoiiro_core::model::Show) {
        let mut inner = self.inner.lock().unwrap();
        let same = inner.local_path.as_deref().is_some_and(|p| {
            let p = std::path::Path::new(p);
            match (p.canonicalize(), path.canonicalize()) {
                (Ok(a), Ok(b)) => a == b,
                _ => p == path,
            }
        });
        if same {
            if let Some(local) = inner.local.as_mut() {
                let _ = local.replace_show(show);
            }
        }
    }

    pub fn skipped(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }

    pub fn new() -> PreviewState {
        PreviewState {
            inner: Mutex::new(Inner { mode: PreviewSource::Runtime, local: None, local_path: None, channel: None }),
            clock: MonotonicClock::new(),
            in_flight: AtomicBool::new(false),
            in_flight_since: AtomicI64::new(0),
            seq: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            started: AtomicBool::new(false),
        }
    }

    fn info(&self, inner: &Inner) -> Result<PreviewInfo, CommandError> {
        let p = inner.local.as_ref().ok_or_else(|| CommandError::new("no_preview", "open a show for the local preview first"))?;
        let now = self.clock.now_us();
        Ok(PreviewInfo {
            show_path: inner.local_path.clone().unwrap_or_default(),
            length_us: p.clock.length_us(),
            bpm: p.bpm(),
            show_bpm: p.show_bpm(),
            position_us: p.clock.position(now),
            playing: p.clock.playing(),
            mode: inner.mode,
        })
    }
}

fn rgb7_to_rgb8(frame: &omoiiro_devices::adapter::DeviceFrame) -> Vec<u8> {
    let mut v = Vec::with_capacity(64 * 3);
    for c in frame.rgb7.iter() {
        for ch in c {
            v.push(((*ch as u16 * 255 + 63) / 127) as u8);
        }
    }
    v
}

/// One preview tick: pick the source, build the frame (None: nothing to show).
fn produce(app: &AppHandle, ps: &PreviewState) -> Option<PreviewFrame> {
    let app_state = app.state::<AppState>();
    let mut inner = ps.inner.lock().unwrap();
    let session = app_state.session.lock().unwrap();
    let now = ps.clock.now_us();
    if inner.mode == PreviewSource::Runtime {
        if let Some(s) = session.as_ref() {
            let msg = s.take_preview_frame()?;
            let tc = s.shared().transport.lock().unwrap().snapshot(s.now_us()).mtc_label;
            return Some(PreviewFrame {
                seq: 0,
                source: PreviewSource::Runtime,
                show_time_us: msg.eval.show_time_us,
                timecode: tc,
                playing: true,
                dark: msg.eval.dark.map(|d| d.name()),
                rgb: rgb7_to_rgb8(&msg.eval.frame),
                skipped: 0,
            });
        }
    }
    drop(session);
    let p = inner.local.as_mut()?;
    let t = p.clock.position(now);
    let frame = p.render_at_now(t, now);
    Some(PreviewFrame { seq: 0, source: PreviewSource::Local, show_time_us: Some(t), timecode: None, playing: p.clock.playing(), dark: None, rgb: rgb7_to_rgb8(&frame), skipped: 0 })
}

fn spawn_thread(app: AppHandle) {
    std::thread::Builder::new()
        .name("omoiiro-preview".into())
        .spawn(move || {
            let period = 1_000_000 / PREVIEW_FPS;
            let ps = app.state::<Arc<PreviewState>>().inner().clone();
            let mut next = ps.clock.now_us();
            loop {
                let now = ps.clock.now_us();
                if now < next {
                    std::thread::sleep(Duration::from_micros((next - now) as u64));
                    continue;
                }
                next += period;
                if next < now {
                    next = now + period; // fell behind: skip, never catch up
                }
                let Some(mut frame) = produce(&app, &ps) else { continue };
                let busy = ps.in_flight.load(Ordering::SeqCst);
                if busy && now - ps.in_flight_since.load(Ordering::SeqCst) < ACK_TIMEOUT_US {
                    ps.skipped.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let channel = ps.inner.lock().unwrap().channel.clone();
                let Some(channel) = channel else { continue };
                frame.seq = ps.seq.fetch_add(1, Ordering::SeqCst) + 1;
                frame.skipped = ps.skipped.load(Ordering::Relaxed);
                ps.in_flight.store(true, Ordering::SeqCst);
                ps.in_flight_since.store(now, Ordering::SeqCst);
                if channel.send(frame).is_err() {
                    ps.in_flight.store(false, Ordering::SeqCst);
                }
            }
        })
        .expect("spawn preview thread");
}

type CmdResult<T> = Result<T, CommandError>;

#[tauri::command]
pub fn preview_subscribe(app: AppHandle, ps: State<'_, Arc<PreviewState>>, on_frame: Channel<PreviewFrame>) {
    ps.inner.lock().unwrap().channel = Some(on_frame);
    ps.in_flight.store(false, Ordering::SeqCst);
    if !ps.started.swap(true, Ordering::SeqCst) {
        spawn_thread(app);
    }
}

#[tauri::command]
pub fn preview_ack(ps: State<'_, Arc<PreviewState>>, seq: u64) {
    if seq == ps.seq.load(Ordering::SeqCst) {
        ps.in_flight.store(false, Ordering::SeqCst);
    }
}

#[tauri::command(async)]
/// The edited show is used when it is this file (unsaved edits included);
/// an empty path is the edited new show that has no file yet.
pub fn preview_open(ps: State<'_, Arc<PreviewState>>, ed: State<'_, crate::editor::EditorState>, show_path: String) -> CmdResult<PreviewInfo> {
    let path = show_path.trim().to_string();
    let show = match crate::editor::current_show_for(&ed, std::path::Path::new(&path)) {
        Some(s) => s,
        None if path.is_empty() => return Err(CommandError::new("invalid", "choose a show file")),
        None => omoiiro_store::load_path(std::path::Path::new(&path)).map_err(|e| CommandError::new("show", e.to_string()))?,
    };
    let local = LocalPreview::new(&show).map_err(|i| CommandError::new("show", format!("{i:?}")))?;
    let mut inner = ps.inner.lock().unwrap();
    inner.local = Some(local);
    inner.local_path = Some(path);
    ps.info(&inner)
}

#[tauri::command]
pub fn preview_mode(ps: State<'_, Arc<PreviewState>>, mode: PreviewSource) -> CmdResult<PreviewInfo> {
    let mut inner = ps.inner.lock().unwrap();
    inner.mode = mode;
    ps.info(&inner).or_else(|e| if mode == PreviewSource::Runtime { Ok(PreviewInfo { show_path: String::new(), length_us: None, bpm: 0.0, show_bpm: 0.0, position_us: 0, playing: false, mode }) } else { Err(e) })
}

#[tauri::command]
pub fn preview_play(ps: State<'_, Arc<PreviewState>>, playing: bool) -> CmdResult<PreviewInfo> {
    let now = ps.clock.now_us();
    let mut inner = ps.inner.lock().unwrap();
    let p = inner.local.as_mut().ok_or_else(|| CommandError::new("no_preview", "open a show for the local preview first"))?;
    if playing {
        p.clock.play(now)
    } else {
        p.clock.pause(now)
    }
    ps.info(&inner)
}

/// Scrub the local preview. Only the local clock moves (never Live or a device).
#[tauri::command]
pub fn preview_seek(ps: State<'_, Arc<PreviewState>>, position_us: i64) -> CmdResult<PreviewInfo> {
    let now = ps.clock.now_us();
    let mut inner = ps.inner.lock().unwrap();
    let p = inner.local.as_mut().ok_or_else(|| CommandError::new("no_preview", "open a show for the local preview first"))?;
    p.clock.seek(now, position_us);
    ps.info(&inner)
}

/// Click on preview pad (x, y), bottom-left origin: a pad press for Ripple
/// in the source being shown (the runtime while it follows, else the local
/// preview). For checking only; it is not device input and nothing goes to Live.
#[tauri::command]
pub fn preview_pad(app_state: State<'_, AppState>, ps: State<'_, Arc<PreviewState>>, x: u8, y: u8, pressed: Option<bool>) -> CmdResult<()> {
    let pressed = pressed.unwrap_or(true);
    let mut inner = ps.inner.lock().unwrap();
    if inner.mode == PreviewSource::Runtime {
        if let Some(s) = app_state.session.lock().unwrap().as_ref() {
            s.press_pad(x, y, pressed);
            return Ok(());
        }
    }
    let now = ps.clock.now_us();
    let p = inner.local.as_mut().ok_or_else(|| CommandError::new("no_preview", "open a show for the local preview first"))?;
    p.press(x, y, pressed, now);
    Ok(())
}

/// Preview tempo: only while the runtime is not following.
#[tauri::command]
pub fn preview_set_bpm(app_state: State<'_, AppState>, ps: State<'_, Arc<PreviewState>>, bpm: f64) -> CmdResult<PreviewInfo> {
    if app_state.session.lock().unwrap().is_some() {
        return Err(CommandError::new("following", "the preview tempo can be changed only while the runtime is stopped"));
    }
    let mut inner = ps.inner.lock().unwrap();
    let p = inner.local.as_mut().ok_or_else(|| CommandError::new("no_preview", "open a show for the local preview first"))?;
    p.set_bpm(bpm).map_err(|e| CommandError::new("invalid", e))?;
    ps.info(&inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb7_maps_to_full_8bit_range() {
        let mut f = omoiiro_devices::adapter::DeviceFrame::black();
        f.rgb7[0] = [127, 64, 1];
        let v = rgb7_to_rgb8(&f);
        assert_eq!(v.len(), 192);
        assert_eq!(&v[..3], &[255, 129, 2]);
    }
}
