//! `MidiBackend` over midir (WinMM / CoreMIDI / ALSA).
//!
//! Inputs are opened with `Ignore::None` so SysEx, MTC Quarter Frames and
//! Real Time bytes arrive. The midir callback only calls `Ingress::deliver`.

use crate::backend::{resolve_error, Ingress, InputConn, MidiBackend, OutputConn, PortError};
use crate::ports::{resolve, resolve_exact};
use midir::{Ignore, MidiInput, MidiOutput};

pub struct MidirBackend {
    client: String,
    next_generation: u64,
}

impl MidirBackend {
    /// `client` is the client name shown by the OS (suffixed per port).
    pub fn new(client: &str) -> MidirBackend {
        MidirBackend { client: client.to_string(), next_generation: 1 }
    }

    fn generation(&mut self) -> u64 {
        let g = self.next_generation;
        self.next_generation += 1;
        g
    }
}

fn input_client(client: &str) -> Result<MidiInput, PortError> {
    let mut input = MidiInput::new(client).map_err(|e| PortError::Open(e.to_string()))?;
    input.ignore(Ignore::None);
    Ok(input)
}

impl MidiBackend for MidirBackend {
    fn backend_name(&self) -> &'static str {
        crate::ports::backend_name()
    }

    fn list_inputs(&self) -> Result<Vec<String>, PortError> {
        let input = input_client(&format!("{}-list", self.client))?;
        Ok(input.ports().iter().map(|p| input.port_name(p).unwrap_or_default()).collect())
    }

    fn list_outputs(&self) -> Result<Vec<String>, PortError> {
        let output = MidiOutput::new(&format!("{}-list", self.client)).map_err(|e| PortError::Open(e.to_string()))?;
        Ok(output.ports().iter().map(|p| output.port_name(p).unwrap_or_default()).collect())
    }

    fn open_input(&mut self, spec: &str, exact: bool, ingress: Ingress) -> Result<InputConn, PortError> {
        let input = input_client(&self.client)?;
        let ports = input.ports();
        let names: Vec<String> = ports.iter().map(|p| input.port_name(p).unwrap_or_default()).collect();
        let i = if exact { resolve_exact(&names, spec) } else { resolve(&names, spec) }.map_err(resolve_error)?;
        let generation = self.generation();
        let ingress = ingress.with_generation(generation);
        let conn = input
            .connect(&ports[i], &format!("{}-in", self.client), move |ts, bytes, _| ingress.deliver(ts, bytes), ())
            .map_err(|e| PortError::Open(format!("{:?}: {e}", names[i])))?;
        Ok(InputConn::new(names[i].clone(), generation, Box::new(conn)))
    }

    fn open_output(&mut self, spec: &str) -> Result<OutputConn, PortError> {
        let output = MidiOutput::new(&self.client).map_err(|e| PortError::Open(e.to_string()))?;
        let ports = output.ports();
        let names: Vec<String> = ports.iter().map(|p| output.port_name(p).unwrap_or_default()).collect();
        let i = resolve_exact(&names, spec).map_err(resolve_error)?;
        let generation = self.generation();
        let conn = output
            .connect(&ports[i], &format!("{}-out", self.client))
            .map_err(|e| PortError::Open(format!("{:?}: {e}", names[i])))?;
        Ok(OutputConn::new(names[i].clone(), generation, Box::new(conn)))
    }
}
