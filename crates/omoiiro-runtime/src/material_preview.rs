//! Bounded, isolated library samples. No session, port, clock or device access.
use omoiiro_core::edit::{self, EditCommand, MaterialKind, Snap};
use omoiiro_core::model::Show;
use crate::preview::LocalPreview;

pub fn sample(show: &Show, kind: MaterialKind, id: &str) -> Result<Vec<Vec<u8>>, String> {
    let mut isolated = edit::new_show("Material preview");
    isolated.seed = show.seed;
    isolated.preview = show.preview.clone();
    isolated.presets = show.presets.clone();
    let duration = match kind {
        MaterialKind::Preset => {
            let (s, _) = edit::apply(&isolated, &EditCommand::AddContinuousLayer { name: "Preview".into(), preset_id: id.into(), index: None }, Snap::None).map_err(|e| e.to_string())?;
            isolated = s;
            2_000_000
        }
        MaterialKind::Compound => {
            let c = show.compounds.iter().find(|c| c.id == id).ok_or("material not found")?;
            isolated.layers = c.layers.clone();
            for l in &mut isolated.layers { l.target_id = isolated.targets[0].id.clone(); }
            c.length_us().clamp(1, 30_000_000) as i64
        }
    };
    let mut preview = LocalPreview::new(&isolated).map_err(|e| format!("{e:?}"))?;
    let mut frames = Vec::with_capacity(24);
    for i in 0..24 {
        let t = i * duration / 24;
        preview.clock.seek(t, t);
        // Explicitly simulated input, identical on every request.
        if i % 8 == 0 { preview.press(2 + (i / 8) as u8, 3, true, t); }
        if i % 8 == 4 { preview.press(2 + (i / 8) as u8, 3, false, t); }
        let f = preview.render_at_now(t, t);
        frames.push(f.rgb7.iter().flat_map(|c| c.iter().map(|x| ((*x as u16 * 255 + 63) / 127) as u8)).collect());
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolated_bounded_and_deterministic() {
        let s = edit::new_show("Source");
        let before = s.clone();
        let a = sample(&s, MaterialKind::Preset, "preset-white").unwrap();
        assert_eq!(s, before);
        assert_eq!(a.len(), 24);
        assert!(a.iter().all(|f| f.len() == 192 && f.iter().all(|x| *x == 255)));
        assert_eq!(a, sample(&s, MaterialKind::Preset, "preset-white").unwrap());
        assert!(sample(&s, MaterialKind::Preset, "missing").is_err());
        let (s, out) = edit::apply(&s, &EditCommand::AddCompound { name: "Test".into() }, Snap::None).unwrap();
        assert_eq!(sample(&s, MaterialKind::Compound, &out.compound_id.unwrap()).unwrap().len(), 24);
    }
}
