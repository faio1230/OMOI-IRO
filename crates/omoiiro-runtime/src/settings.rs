//! Terminal settings (M5, U02; docs/04 "端末設定との分離"): which physical
//! port plays which role on this machine, the bridge address, Link use and
//! recent files. Saved in the app's user settings directory, never in the
//! show.
//!
//! Port roles and their direction:
//!
//! | role | direction | used for |
//! | --- | --- | --- |
//! | `control_in` | input | pads / operation input forwarded to Live |
//! | `control_out` | output | the Live control bus (`OMOI-IRO Control`) |
//! | `clock_in` | input | MIDI Clock from Live (beat source midi_clock) |
//! | `led_out` | output | LED frames (empty = in-process virtual Launchpad X) |
//! | `timecode_out` | output | MTC for external gear (when the show enables `mtc_output`) |
//!
//! A port is given as its exact name or `#index` (substrings are not
//! accepted). `check_roles` compares the settings with the current port
//! lists and never sends anything. It rejects, with a reason:
//!
//! * a spec matching several ports with the same name (fault: use `#index`);
//! * reverse flow: `control_out` or `timecode_out` on a port the app also
//!   reads (`control_in`, `clock_in`), which would loop back (a loopMIDI port
//!   has the same name on both sides);
//! * the same port in two roles of the same direction;
//! * a Launchpad-looking port as `led_out`, `control_out` or `timecode_out`
//!   (device output is M6; the control and timecode buses must not be a
//!   device).
//!
//! A port that is simply absent is a warning: the binding retries until it
//! appears (I06). `led_out` and `control_in` may share a device name (the
//! Launchpad X has one port name in both directions).

use omoiiro_midi::ports::{device_port_key, is_launchpad_daw_port, looks_like_launchpad, resolve_exact};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const SETTINGS_FILE: &str = "terminal-settings.json";
pub const SETTINGS_VERSION: u32 = 1;
pub const MAX_RECENT: usize = 10;
/// Library shows (A20).
pub const MAX_LIBRARY: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortRoles {
    pub control_in: Option<String>,
    pub control_out: Option<String>,
    pub clock_in: Option<String>,
    /// None = in-process virtual Launchpad X.
    pub led_out: Option<String>,
    pub timecode_out: Option<String>,
}

impl Default for PortRoles {
    fn default() -> Self {
        PortRoles { control_in: None, control_out: Some(omoiiro_midi::ports::names::CONTROL_PORT.into()), clock_in: None, led_out: None, timecode_out: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalSettings {
    pub version: u32,
    /// MIDI backend (only "midir" exists).
    pub backend: String,
    pub bridge_bind: String,
    pub ports: PortRoles,
    /// Use Ableton Link as the beat source when the show selects it.
    pub link: bool,
    pub recent_shows: Vec<String>,
    /// Brightness ceiling and flashing limit of this terminal (docs/12).
    /// Missing in older files: the defaults (1.0, 3 Hz, no muting).
    #[serde(default)]
    pub safety: omoiiro_core::safety::SafetyLimits,
    /// Device output permissions (M6). Missing in older files: all off.
    #[serde(default)]
    pub devices: DeviceSettings,
    /// Shows whose presets and compounds are offered as library materials
    /// (A20; read only, at most `MAX_LIBRARY`). Missing in older files: none.
    #[serde(default)]
    pub library_shows: Vec<String>,
}

/// What the app may send to a device directly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeviceSettings {
    /// Send LED frames straight to a Launchpad X (`led_out` may then be a
    /// Launchpad-looking port). The device is identified first; Live must
    /// not have the Launchpad as a Control Surface.
    pub allow_launchpad_led: bool,
}

/// Shown when the permission is turned on and when following starts with it.
pub const LAUNCHPAD_LED_WARNING: &str = "Launchpad X へ直接 LED を送ります。Live の Control Surface に Launchpad を割り当てないでください";

impl Default for TerminalSettings {
    fn default() -> Self {
        TerminalSettings {
            version: SETTINGS_VERSION,
            backend: "midir".into(),
            bridge_bind: crate::sync::bridge_ingress::DEFAULT_BIND.into(),
            ports: PortRoles::default(),
            link: false,
            recent_shows: Vec::new(),
            safety: Default::default(),
            devices: Default::default(),
            library_shows: Vec::new(),
        }
    }
}

impl TerminalSettings {
    /// Blank strings become None (a cleared field means "not used").
    pub fn normalized(mut self) -> TerminalSettings {
        let n = |o: &mut Option<String>| {
            *o = o.take().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        };
        n(&mut self.ports.control_in);
        n(&mut self.ports.control_out);
        n(&mut self.ports.clock_in);
        n(&mut self.ports.led_out);
        n(&mut self.ports.timecode_out);
        if self.ports.led_out.as_deref() == Some("virtual") {
            self.ports.led_out = None;
        }
        self.bridge_bind = self.bridge_bind.trim().to_string();
        self.recent_shows.truncate(MAX_RECENT);
        let mut library: Vec<String> = Vec::new();
        for p in self.library_shows.drain(..).map(|p| p.trim().to_string()).filter(|p| !p.is_empty()) {
            if !library.contains(&p) {
                library.push(p);
            }
        }
        library.truncate(MAX_LIBRARY);
        self.library_shows = library;
        self
    }

    pub fn remember_show(&mut self, path: &str) {
        self.recent_shows.retain(|p| p != path);
        self.recent_shows.insert(0, path.to_string());
        self.recent_shows.truncate(MAX_RECENT);
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("settings serialize") + "\n"
    }

    pub fn from_json(text: &str) -> Result<TerminalSettings, String> {
        let s: TerminalSettings = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if s.version != SETTINGS_VERSION {
            return Err(format!("unsupported terminal settings version {}", s.version));
        }
        Ok(s.normalized())
    }
}

/// Load; a missing file gives the defaults. A broken file is an error and is
/// left untouched (the caller shows it and keeps the defaults until a save).
pub fn load(dir: &Path) -> Result<TerminalSettings, String> {
    let p = dir.join(SETTINGS_FILE);
    match std::fs::read_to_string(&p) {
        Ok(t) => TerminalSettings::from_json(&t).map_err(|e| format!("{}: {e}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TerminalSettings::default()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

/// Write to a temporary file in the same directory, then replace.
pub fn save(dir: &Path, s: &TerminalSettings) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let p = dir.join(SETTINGS_FILE);
    let tmp = dir.join(format!("{SETTINGS_FILE}.tmp"));
    std::fs::write(&tmp, s.to_json()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(p)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Input,
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleState {
    /// Not used (empty).
    Unused,
    /// In-process virtual Launchpad X (`led_out` only).
    Virtual,
    /// Found exactly once.
    Ok,
    /// Not present now (the binding retries).
    Missing,
    /// Several ports share the name: needs `#index`.
    Fault,
    /// Refused configuration (reason in `message`).
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RoleReport {
    pub role: &'static str,
    pub direction: Direction,
    pub spec: Option<String>,
    pub resolved: Option<String>,
    pub state: RoleState,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CheckReport {
    pub roles: Vec<RoleReport>,
    /// True when nothing is `Fault` / `Invalid` (missing ports are allowed).
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

const ROLES: [(&str, Direction); 5] = [
    ("control_in", Direction::Input),
    ("control_out", Direction::Output),
    ("clock_in", Direction::Input),
    ("led_out", Direction::Output),
    ("timecode_out", Direction::Output),
];

fn spec_of<'a>(p: &'a PortRoles, role: &str) -> Option<&'a str> {
    match role {
        "control_in" => p.control_in.as_deref(),
        "control_out" => p.control_out.as_deref(),
        "clock_in" => p.clock_in.as_deref(),
        "led_out" => p.led_out.as_deref(),
        "timecode_out" => p.timecode_out.as_deref(),
        _ => None,
    }
}

fn label(role: &str) -> &'static str {
    match role {
        "control_in" => "操作入力",
        "control_out" => "Live操作出力",
        "clock_in" => "MIDI Clock入力",
        "led_out" => "LED出力",
        "timecode_out" => "タイムコード出力",
        _ => "?",
    }
}

/// Compare the settings with the current port lists. Sends nothing.
pub fn check_roles(s: &TerminalSettings, inputs: &[String], outputs: &[String]) -> CheckReport {
    let mut roles: Vec<RoleReport> = Vec::new();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for (role, dir) in ROLES {
        let spec = spec_of(&s.ports, role).map(str::to_string);
        let names = if dir == Direction::Input { inputs } else { outputs };
        let mut r = RoleReport { role, direction: dir, spec: spec.clone(), resolved: None, state: RoleState::Unused, message: None };
        match spec.as_deref() {
            None if role == "led_out" => r.state = RoleState::Virtual,
            None => {}
            Some(sp) => match resolve_exact(names, sp) {
                Ok(i) => {
                    r.resolved = Some(names[i].clone());
                    r.state = RoleState::Ok;
                    if sp.starts_with('#') {
                        r.message = Some(format!("#index は起動順やUSBの抜き差しで変わる。{:?} の完全一致名を推奨", names[i]));
                    }
                }
                Err(e) if e.contains("select one with #index") => {
                    r.state = RoleState::Fault;
                    r.message = Some(format!("同じ名前のポートが複数ある：{e}"));
                }
                Err(_) => {
                    // A name that matches nothing: absent now. Substrings never match.
                    r.state = RoleState::Missing;
                    let near: Vec<&String> = names.iter().filter(|n| n.to_lowercase().contains(&sp.to_lowercase())).collect();
                    r.message = Some(if near.is_empty() {
                        "今はポートがない（現れたら接続する）".to_string()
                    } else {
                        format!("完全一致するポートがない（部分一致は使わない。候補：{near:?}）")
                    });
                }
            },
        }
        roles.push(r);
    }

    let name = |roles: &[RoleReport], role: &str| -> Option<String> { roles.iter().find(|r| r.role == role).and_then(|r| r.resolved.clone().or_else(|| r.spec.clone().filter(|s| !s.starts_with('#')))) };
    let mut refuse = |roles: &mut Vec<RoleReport>, role: &str, msg: String| {
        if let Some(r) = roles.iter_mut().find(|r| r.role == role) {
            r.state = RoleState::Invalid;
            r.message = Some(msg.clone());
        }
        errors.push(format!("{}：{msg}", label(role)));
    };

    // Launchpad-looking outputs: LED output only with the device permission;
    // the Live control bus and the timecode output never.
    let mut device_led = None;
    for role in ["led_out", "control_out", "timecode_out"] {
        if let Some(n) = name(&roles, role) {
            if looks_like_launchpad(&n) {
                if role == "led_out" && s.devices.allow_launchpad_led {
                    device_led = Some(n);
                    continue;
                }
                let why = if role == "led_out" {
                    format!("{n:?} は Launchpad らしい名前。機器へ直接送るには「Launchpad X へ LED を送る」を有効にする（空欄にすると仮想 Launchpad X）")
                } else {
                    format!("{n:?} は Launchpad らしい名前。Live / 外部機器向けのバスに機器を指定しない")
                };
                refuse(&mut roles, role, why);
            }
        }
    }
    // The Launchpad's DAW interface carries neither Programmer Mode pads nor LEDs.
    let mut daw = false;
    for role in ["led_out", "control_in"] {
        if let Some(n) = name(&roles, role) {
            if is_launchpad_daw_port(&n) {
                daw |= role == "led_out";
                refuse(&mut roles, role, format!("{n:?} は Launchpad の DAW 用ポート。MIDI 用（名前に「LPX MIDI」を含む方）を指定する"));
            }
        }
    }
    if let Some(n) = device_led.filter(|_| !daw) {
        if let Some(r) = roles.iter_mut().find(|r| r.role == "led_out") {
            r.message = Some(format!("{n:?} へ直接送る。接続ごとに機器を確認（Device Inquiry）してから送る。{LAUNCHPAD_LED_WARNING}"));
        }
        // macOS names the two directions differently ("… MIDI Out" / "… MIDI In").
        let same_device = name(&roles, "control_in").is_some_and(|i| device_port_key(&i) == device_port_key(&n));
        if !same_device {
            warnings.push(format!("LED出力：機器の確認の応答は操作入力から受ける。操作入力にも同じ {n:?} を指定する（指定しないと「応答がない」で止まる）"));
        }
    }
    // Reverse flow: an output to Live / external gear on a port the app reads.
    for out_role in ["control_out", "timecode_out"] {
        for in_role in ["control_in", "clock_in"] {
            if let (Some(o), Some(i)) = (name(&roles, out_role), name(&roles, in_role)) {
                if o == i {
                    refuse(&mut roles, out_role, format!("{o:?} は{}でもある。送ったものが自分に戻る（逆流）", label(in_role)));
                }
            }
        }
    }
    // Same port twice in one direction.
    for (a, b) in [("control_out", "led_out"), ("control_out", "timecode_out"), ("led_out", "timecode_out"), ("control_in", "clock_in")] {
        if let (Some(x), Some(y)) = (name(&roles, a), name(&roles, b)) {
            if x == y {
                refuse(&mut roles, b, format!("{x:?} は{}にも割り当て済み（二重割当）", label(a)));
            }
        }
    }
    for e in s.safety.issues() {
        errors.push(format!("安全：{e}"));
    }
    if s.bridge_bind.parse::<std::net::SocketAddr>().is_err() {
        errors.push(format!("ブリッジ受信：{:?} は IP:ポート の形ではない", s.bridge_bind));
    }
    for r in &roles {
        match r.state {
            RoleState::Fault => errors.push(format!("{}：{}", label(r.role), r.message.clone().unwrap_or_default())),
            RoleState::Missing => warnings.push(format!("{}：{}", label(r.role), r.message.clone().unwrap_or_default())),
            RoleState::Ok if r.message.is_some() => warnings.push(format!("{}：{}", label(r.role), r.message.clone().unwrap_or_default())),
            _ => {}
        }
    }
    errors.dedup();
    CheckReport { ok: errors.is_empty(), roles, errors, warnings }
}

/// Classification of messages seen by an input receive check.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ListenCounts {
    pub total: u64,
    pub midi_clock: u64,
    pub transport: u64,
    pub mtc_quarter_frame: u64,
    pub sysex: u64,
    pub note: u64,
    pub control_change: u64,
    pub other: u64,
}

impl ListenCounts {
    pub fn add(&mut self, bytes: &[u8]) {
        self.total += 1;
        match bytes.first().copied() {
            Some(0xF8) => self.midi_clock += 1,
            Some(0xFA | 0xFB | 0xFC | 0xF2) => self.transport += 1,
            Some(0xF1) => self.mtc_quarter_frame += 1,
            Some(0xF0) => self.sysex += 1,
            Some(s) if (0x80..=0x9F).contains(&s) => self.note += 1,
            Some(s) if (0xB0..=0xBF).contains(&s) => self.control_change += 1,
            _ => self.other += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn settings(f: impl FnOnce(&mut PortRoles)) -> TerminalSettings {
        let mut s = TerminalSettings::default();
        f(&mut s.ports);
        s
    }

    fn state(r: &CheckReport, role: &str) -> RoleState {
        r.roles.iter().find(|x| x.role == role).unwrap().state
    }

    #[test]
    fn normal_setup_with_a_launchpad_on_both_sides() {
        let ins = v(&["LPX MIDI", "OMOI-IRO Clock", "OMOI-IRO Control"]);
        let outs = v(&["LPX MIDI", "OMOI-IRO Clock", "OMOI-IRO Control", "OMOI-IRO Timecode"]);
        let s = settings(|p| {
            p.control_in = Some("LPX MIDI".into());
            p.clock_in = Some("OMOI-IRO Clock".into());
            p.timecode_out = Some("OMOI-IRO Timecode".into());
        });
        let r = check_roles(&s, &ins, &outs);
        assert!(r.ok, "{:?}", r.errors);
        assert_eq!(state(&r, "led_out"), RoleState::Virtual);
        assert_eq!(state(&r, "control_out"), RoleState::Ok);
    }

    #[test]
    fn misconfigurations_are_refused_with_reasons() {
        let ins = v(&["OMOI-IRO Control", "OMOI-IRO Clock", "LPX MIDI"]);
        let outs = v(&["OMOI-IRO Control", "OMOI-IRO Timecode", "LPX MIDI"]);
        // Reverse flow: Control bus is also the operation input.
        let r = check_roles(&settings(|p| p.control_in = Some("OMOI-IRO Control".into())), &ins, &outs);
        assert!(!r.ok);
        assert_eq!(state(&r, "control_out"), RoleState::Invalid);
        assert!(r.errors[0].contains("逆流"), "{:?}", r.errors);
        // LED out on a Launchpad (M6), control out on a device.
        let r = check_roles(&settings(|p| p.led_out = Some("LPX MIDI".into())), &ins, &outs);
        assert_eq!(state(&r, "led_out"), RoleState::Invalid);
        assert!(r.errors[0].contains("Launchpad X へ LED を送る"), "{:?}", r.errors);
        let r = check_roles(&settings(|p| p.control_out = Some("LPX MIDI".into())), &ins, &outs);
        assert_eq!(state(&r, "control_out"), RoleState::Invalid);
        // Same port twice.
        let r = check_roles(&settings(|p| p.timecode_out = Some("OMOI-IRO Control".into())), &ins, &outs);
        assert_eq!(state(&r, "timecode_out"), RoleState::Invalid);
        assert!(r.errors.iter().any(|e| e.contains("二重割当")));
        let r = check_roles(&settings(|p| {
            p.control_in = Some("OMOI-IRO Clock".into());
            p.clock_in = Some("OMOI-IRO Clock".into());
        }), &ins, &outs);
        assert_eq!(state(&r, "clock_in"), RoleState::Invalid);
        // A Launchpad as LED output: refused unless the device permission is on.
        let mut s = settings(|p| {
            p.led_out = Some("LPX MIDI".into());
            p.control_in = Some("LPX MIDI".into());
        });
        assert_eq!(state(&check_roles(&s, &ins, &outs), "led_out"), RoleState::Invalid);
        s.devices.allow_launchpad_led = true;
        let r = check_roles(&s, &ins, &outs);
        assert!(r.ok, "{:?}", r.errors);
        assert_eq!(state(&r, "led_out"), RoleState::Ok);
        assert!(r.roles.iter().find(|x| x.role == "led_out").unwrap().message.as_ref().unwrap().contains("Control Surface"));
        // Without the same device as operation input: a warning (identification needs it).
        s.ports.control_in = None;
        assert!(check_roles(&s, &ins, &outs).warnings.iter().any(|w| w.contains("操作入力にも同じ")));
        // The permission never opens the control bus or the timecode output to a device.
        let mut s2 = settings(|p| p.timecode_out = Some("LPX MIDI".into()));
        s2.devices.allow_launchpad_led = true;
        assert_eq!(state(&check_roles(&s2, &ins, &outs), "timecode_out"), RoleState::Invalid);
        // Bad bridge address.
        let mut s = TerminalSettings::default();
        s.bridge_bind = "localhost".into();
        assert!(!check_roles(&s, &ins, &outs).ok);
    }

    #[test]
    fn macos_coremidi_names() {
        // CoreMIDI display names: IAC buses carry the driver name (here Japanese)
        // and are the same on both sides; the Launchpad has "In"/"Out" endpoints.
        let ins = v(&["IACドライバ OMOI-IRO Control", "IACドライバ OMOI-IRO Clock", "Launchpad X LPX DAW Out", "Launchpad X LPX MIDI Out"]);
        let outs = v(&["IACドライバ OMOI-IRO Control", "IACドライバ OMOI-IRO Clock", "Launchpad X LPX DAW In", "Launchpad X LPX MIDI In"]);
        let mut s = settings(|p| {
            p.control_in = Some("Launchpad X LPX MIDI Out".into());
            p.control_out = Some("IACドライバ OMOI-IRO Control".into());
            p.clock_in = Some("IACドライバ OMOI-IRO Clock".into());
            p.led_out = Some("Launchpad X LPX MIDI In".into());
        });
        s.devices.allow_launchpad_led = true;
        let r = check_roles(&s, &ins, &outs);
        assert!(r.ok, "{:?}", r.errors);
        // Only the device notice, no "same device as the operation input" warning.
        assert!(!r.warnings.iter().any(|w| w.contains("操作入力にも同じ")), "{:?}", r.warnings);
        for role in ["control_in", "control_out", "clock_in", "led_out"] {
            assert_eq!(state(&r, role), RoleState::Ok, "{role}");
        }
        assert_eq!(state(&r, "timecode_out"), RoleState::Unused);
        // The DAW interface is refused for pads and LEDs.
        let mut d = s.clone();
        d.ports.led_out = Some("Launchpad X LPX DAW In".into());
        d.ports.control_in = Some("Launchpad X LPX DAW Out".into());
        let r = check_roles(&d, &ins, &outs);
        assert!(!r.ok);
        assert_eq!(state(&r, "led_out"), RoleState::Invalid);
        assert_eq!(state(&r, "control_in"), RoleState::Invalid);
        assert!(r.errors.iter().all(|e| e.contains("DAW 用")), "{:?}", r.errors);
        // The Mac example file loads and passes with the LPX roles left empty.
        let ex = TerminalSettings::from_json(include_str!("../../../fixtures/terminal-settings.mac.example.json")).unwrap();
        assert!(ex.devices.allow_launchpad_led && ex.ports.timecode_out.is_none());
        assert!(check_roles(&ex, &ins, &outs).ok);
        // Reverse flow still caught with IAC names (same name on both sides).
        let mut f = s.clone();
        f.ports.control_out = Some("IACドライバ OMOI-IRO Clock".into());
        assert_eq!(state(&check_roles(&f, &ins, &outs), "control_out"), RoleState::Invalid);
    }

    #[test]
    fn duplicates_fault_and_missing_ports_only_warn() {
        let ins = v(&["Pads", "Pads"]);
        let outs = v(&["OMOI-IRO Control"]);
        let r = check_roles(&settings(|p| p.control_in = Some("Pads".into())), &ins, &outs);
        assert_eq!(state(&r, "control_in"), RoleState::Fault);
        assert!(!r.ok);
        let r = check_roles(&settings(|p| p.control_in = Some("#1".into())), &ins, &outs);
        assert_eq!(state(&r, "control_in"), RoleState::Ok);
        assert!(r.ok && r.warnings[0].contains("#index"));
        // Absent and substring-only ports: warnings, start is allowed.
        let r = check_roles(&settings(|p| p.clock_in = Some("OMOI".into())), &v(&["OMOI-IRO Clock"]), &outs);
        assert_eq!(state(&r, "clock_in"), RoleState::Missing);
        assert!(r.ok && r.warnings[0].contains("部分一致は使わない"));
    }

    #[test]
    fn save_load_round_trip_and_broken_file() {
        let dir = std::env::temp_dir().join(format!("omoiiro-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir).unwrap(), TerminalSettings::default());
        let mut s = TerminalSettings::default();
        s.ports.led_out = Some(" virtual ".into());
        s.ports.clock_in = Some("  ".into());
        s.remember_show("a.show.json");
        s.remember_show("b.show.json");
        s.remember_show("a.show.json");
        s.library_shows = (0..12).map(|i| format!(" l{} .show.json", i % 11)).collect();
        let s = s.normalized();
        assert_eq!(s.library_shows.len(), MAX_LIBRARY, "trimmed, unique, at most 10");
        assert_eq!((s.library_shows[0].as_str(), s.library_shows[9].as_str()), ("l0 .show.json", "l9 .show.json"));
        assert_eq!((s.ports.led_out.clone(), s.ports.clock_in.clone()), (None, None));
        assert_eq!(s.recent_shows, vec!["a.show.json", "b.show.json"]);
        save(&dir, &s).unwrap();
        assert_eq!(load(&dir).unwrap(), s);
        // A version-1 file written before the safety section still loads.
        let mut old: serde_json::Value = serde_json::from_str(&s.to_json()).unwrap();
        old.as_object_mut().unwrap().remove("safety");
        old.as_object_mut().unwrap().remove("library_shows");
        std::fs::write(dir.join(SETTINGS_FILE), old.to_string()).unwrap();
        assert_eq!(load(&dir).unwrap().safety, omoiiro_core::safety::SafetyLimits::default());
        assert!(load(&dir).unwrap().library_shows.is_empty(), "files without the library still load");
        let mut bad = TerminalSettings::default();
        bad.safety.max_flash_hz = 100.0;
        assert!(!check_roles(&bad, &[], &[]).ok, "out-of-range safety values are refused");
        std::fs::write(dir.join(SETTINGS_FILE), "{\"version\": 1, \"unknown\": true}").unwrap();
        assert!(load(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn listen_counts_classify_messages() {
        let mut c = ListenCounts::default();
        for b in [&[0xF8][..], &[0xFA], &[0xF1, 0x10], &[0xF0, 0x7F, 0xF7], &[0x90, 1, 2], &[0xB0, 1, 2], &[0xE0, 0, 64]] {
            c.add(b);
        }
        assert_eq!(c, ListenCounts { total: 7, midi_clock: 1, transport: 1, mtc_quarter_frame: 1, sysex: 1, note: 1, control_change: 1, other: 1 });
    }
}
