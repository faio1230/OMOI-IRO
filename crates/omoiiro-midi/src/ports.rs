//! MIDI port listing and explicit port selection.
//!
//! Listing never sends anything. Selection accepts an exact name, `#index`,
//! or a unique case-insensitive substring; an ambiguous substring is an
//! error so that a device is never chosen by accident. Commands that send to
//! a device or into Live (`lpx-check`, `route-check`) use the `_exact`
//! variants, which accept only an exact name or `#index`.

use midir::{Ignore, MidiInput, MidiInputPort, MidiOutput, MidiOutputPort};

/// Virtual bus names used by the project (one place for the planned rename
/// to the product name, docs/07-decisions.md A18).
pub mod names {
    /// App -> Live control bus (Note / CC).
    pub const CONTROL_PORT: &str = "OMOI-IRO Control";
    /// App -> external gear MTC output bus.
    pub const TIMECODE_PORT: &str = "OMOI-IRO Timecode";
    /// Live -> app MIDI Clock bus.
    pub const CLOCK_PORT: &str = "OMOI-IRO Clock";
}
use std::error::Error;

#[derive(Debug, Clone)]
pub struct PortList {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

pub fn list() -> Result<PortList, Box<dyn Error>> {
    let input = MidiInput::new("omoiiro-probe-list")?;
    let output = MidiOutput::new("omoiiro-probe-list")?;
    let inputs = input
        .ports()
        .iter()
        .map(|p| input.port_name(p).unwrap_or_else(|_| "<unnamed>".to_string()))
        .collect();
    let outputs = output
        .ports()
        .iter()
        .map(|p| output.port_name(p).unwrap_or_else(|_| "<unnamed>".to_string()))
        .collect();
    Ok(PortList { inputs, outputs })
}

pub fn backend_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "midir/WinMM"
    } else if cfg!(target_os = "macos") {
        "midir/CoreMIDI"
    } else {
        "midir/ALSA"
    }
}

/// Resolve a port spec against a list of names.
pub fn resolve(names: &[String], spec: &str) -> Result<usize, String> {
    if let Some(idx) = spec.strip_prefix('#') {
        let i: usize = idx
            .parse()
            .map_err(|_| format!("bad port index {spec:?}"))?;
        if i < names.len() {
            return Ok(i);
        }
        return Err(format!("port index {i} out of range (0..{})", names.len()));
    }
    if let Some(i) = names.iter().position(|n| n == spec) {
        return Ok(i);
    }
    let needle = spec.to_lowercase();
    let hits: Vec<usize> = names
        .iter()
        .enumerate()
        .filter(|(_, n)| n.to_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect();
    match hits.as_slice() {
        [i] => Ok(*i),
        [] => Err(format!("no port matches {spec:?}; available: {names:?}")),
        many => Err(format!(
            "{spec:?} is ambiguous: {:?}",
            many.iter().map(|i| &names[*i]).collect::<Vec<_>>()
        )),
    }
}

/// Resolve only an exact name or `#index`; substrings are rejected.
pub fn resolve_exact(names: &[String], spec: &str) -> Result<usize, String> {
    if spec.starts_with('#') {
        return resolve(names, spec);
    }
    let hits: Vec<usize> = names.iter().enumerate().filter(|(_, n)| *n == spec).map(|(i, _)| i).collect();
    match hits.as_slice() {
        [i] => Ok(*i),
        [] => Err(format!("no port named exactly {spec:?} (use the full name or #index); available: {names:?}")),
        // Two ports with the same name (e.g. two identical devices): never pick one silently.
        many => Err(format!("{} ports are named {spec:?} (#{}); select one with #index", many.len(),
            many.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", #"))),
    }
}

type Resolver = fn(&[String], &str) -> Result<usize, String>;

fn open_input_with(
    client: &str,
    spec: &str,
    resolver: Resolver,
) -> Result<(MidiInput, MidiInputPort, String), Box<dyn Error>> {
    let mut input = MidiInput::new(client)?;
    // Sync ports must receive SysEx (Full Frame), F1 and Real Time bytes.
    input.ignore(Ignore::None);
    let ports = input.ports();
    let names: Vec<String> = ports
        .iter()
        .map(|p| input.port_name(p).unwrap_or_default())
        .collect();
    let i = resolver(&names, spec)?;
    Ok((input, ports[i].clone(), names[i].clone()))
}

fn open_output_with(
    client: &str,
    spec: &str,
    resolver: Resolver,
) -> Result<(MidiOutput, MidiOutputPort, String), Box<dyn Error>> {
    let output = MidiOutput::new(client)?;
    let ports = output.ports();
    let names: Vec<String> = ports
        .iter()
        .map(|p| output.port_name(p).unwrap_or_default())
        .collect();
    let i = resolver(&names, spec)?;
    Ok((output, ports[i].clone(), names[i].clone()))
}

pub fn open_input(client: &str, spec: &str) -> Result<(MidiInput, MidiInputPort, String), Box<dyn Error>> {
    open_input_with(client, spec, resolve)
}

pub fn open_output(client: &str, spec: &str) -> Result<(MidiOutput, MidiOutputPort, String), Box<dyn Error>> {
    open_output_with(client, spec, resolve)
}

pub fn open_input_exact(client: &str, spec: &str) -> Result<(MidiInput, MidiInputPort, String), Box<dyn Error>> {
    open_input_with(client, spec, resolve_exact)
}

pub fn open_output_exact(client: &str, spec: &str) -> Result<(MidiOutput, MidiOutputPort, String), Box<dyn Error>> {
    open_output_with(client, spec, resolve_exact)
}

/// True for names that look like a Launchpad port.
pub fn looks_like_launchpad(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("launchpad") || lower.contains("lpx")
}

/// True for the Launchpad's DAW-mode interface ("LPX DAW", macOS
/// "Launchpad X LPX DAW In/Out"). Pads and LEDs in Programmer Mode use the
/// MIDI interface ("LPX MIDI"), never this one.
pub fn is_launchpad_daw_port(name: &str) -> bool {
    looks_like_launchpad(name) && name.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|w| w == "daw")
}

/// One key for both directions of a device port: CoreMIDI names the source
/// and the destination differently ("Launchpad X LPX MIDI Out" / "... In"),
/// WinMM uses one name ("LPX MIDI"). Lowercase, without a trailing In / Out.
pub fn device_port_key(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    for suffix in [" out", " in"] {
        if let Some(base) = lower.strip_suffix(suffix) {
            return base.trim_end().to_string();
        }
    }
    lower
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        vec![
            "loopMIDI Port".to_string(),
            "loopMIDI Port 1".to_string(),
            "LPX MIDI".to_string(),
            "MIDIIN2 (LPX MIDI)".to_string(),
        ]
    }

    #[test]
    fn exact_name_wins_over_substring() {
        assert_eq!(resolve(&names(), "loopMIDI Port"), Ok(0));
        assert_eq!(resolve(&names(), "LPX MIDI"), Ok(2));
    }

    #[test]
    fn index_spec() {
        assert_eq!(resolve(&names(), "#3"), Ok(3));
        assert!(resolve(&names(), "#4").is_err());
        assert!(resolve(&names(), "#x").is_err());
    }

    #[test]
    fn unique_substring_is_case_insensitive() {
        assert_eq!(resolve(&names(), "midiin2"), Ok(3));
        assert_eq!(resolve(&names(), "port 1"), Ok(1));
    }

    #[test]
    fn exact_resolution_rejects_substrings() {
        assert_eq!(resolve_exact(&names(), "LPX MIDI"), Ok(2));
        assert_eq!(resolve_exact(&names(), "#3"), Ok(3));
        assert!(resolve_exact(&names(), "midiin2").is_err());
        assert!(resolve_exact(&names(), "lpx midi").is_err());
        assert!(resolve_exact(&names(), "#9").is_err());
    }

    #[test]
    fn exact_resolution_rejects_duplicate_names() {
        let n = vec!["LPX MIDI".to_string(), "OMOI-IRO Control".to_string(), "LPX MIDI".to_string()];
        let e = resolve_exact(&n, "LPX MIDI").unwrap_err();
        assert!(e.contains("#0, #2"), "{e}");
        assert_eq!(resolve_exact(&n, "#2"), Ok(2));
        assert_eq!(resolve_exact(&n, "OMOI-IRO Control"), Ok(1));
    }

    #[test]
    fn launchpad_names_on_windows_and_macos() {
        for n in ["LPX MIDI", "MIDIIN2 (LPX MIDI)", "Launchpad X LPX MIDI In", "Launchpad X LPX MIDI Out"] {
            assert!(looks_like_launchpad(n) && !is_launchpad_daw_port(n), "{n}");
        }
        for n in ["LPX DAW", "Launchpad X LPX DAW In", "Launchpad X LPX DAW Out"] {
            assert!(is_launchpad_daw_port(n), "{n}");
        }
        // IAC buses (Japanese and English) are not devices.
        for n in ["IACドライバ OMOI-IRO Clock", "IAC Driver OMOI-IRO Control", "OMOI-IRO Timecode"] {
            assert!(!looks_like_launchpad(n) && !is_launchpad_daw_port(n), "{n}");
        }
        assert_eq!(device_port_key("Launchpad X LPX MIDI Out"), device_port_key("Launchpad X LPX MIDI In"));
        assert_eq!(device_port_key("LPX MIDI"), "lpx midi");
        assert_ne!(device_port_key("Launchpad X LPX MIDI Out"), device_port_key("Launchpad X LPX DAW In"));
    }

    #[test]
    fn ambiguous_or_missing_is_an_error() {
        assert!(resolve(&names(), "lpx").unwrap_err().contains("ambiguous"));
        assert!(resolve(&names(), "nothing").unwrap_err().contains("no port"));
    }
}
