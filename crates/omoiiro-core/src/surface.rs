//! LED surface and frames.
//!
//! The MVP surface is the Launchpad X 8 x 8 pad grid
//! (`novation.launchpad_x.grid64.v1`). LEDs are addressed with a
//! bottom-left origin: `x` grows to the right, `y` grows upward, and the
//! linear index is `y * 8 + x`. Device codes (e.g. the LPX Programmer Mode
//! note `(y + 1) * 10 + (x + 1)`) belong to `omoiiro-devices`.

/// Profile ID of the only surface supported in the MVP.
pub const GRID64_PROFILE: &str = "novation.launchpad_x.grid64.v1";
pub const GRID_W: usize = 8;
pub const GRID_H: usize = 8;
pub const GRID_LEDS: usize = GRID_W * GRID_H;

/// Bottom-left-origin LED coordinate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LedId {
    pub x: u8,
    pub y: u8,
}

impl LedId {
    pub fn new(x: u8, y: u8) -> Option<LedId> {
        ((x as usize) < GRID_W && (y as usize) < GRID_H).then_some(LedId { x, y })
    }

    pub fn index(self) -> usize {
        self.y as usize * GRID_W + self.x as usize
    }

    pub fn from_index(i: usize) -> Option<LedId> {
        (i < GRID_LEDS).then(|| LedId { x: (i % GRID_W) as u8, y: (i / GRID_W) as u8 })
    }
}

/// Linear RGB-like color in the 0..=1 range (sRGB values used as-is in the
/// MVP; device calibration and gamma belong to the device layer).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0.0, g: 0.0, b: 0.0 };

    pub fn new(r: f32, g: f32, b: f32) -> Rgb {
        Rgb { r, g, b }
    }

    /// Parse `#RRGGBB`.
    pub fn from_hex(s: &str) -> Option<Rgb> {
        let h = s.strip_prefix('#')?;
        if h.len() != 6 || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f32 / 255.0);
        Some(Rgb { r: c(0)?, g: c(2)?, b: c(4)? })
    }

    pub fn scale(self, k: f32) -> Rgb {
        Rgb { r: self.r * k, g: self.g * k, b: self.b * k }
    }

    /// 8-bit per channel, rounded and clamped.
    pub fn to_rgb8(self) -> [u8; 3] {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [q(self.r), q(self.g), q(self.b)]
    }

    pub fn to_hex(self) -> String {
        let [r, g, b] = self.to_rgb8();
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

/// Premultiplied RGBA used while compositing a layer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const TRANSPARENT: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    /// Premultiply `color` by `alpha`.
    pub fn from_color(color: Rgb, alpha: f32) -> Rgba {
        let a = alpha.clamp(0.0, 1.0);
        Rgba { r: color.r * a, g: color.g * a, b: color.b * a, a }
    }
}

/// One rendered frame of the grid: opaque colors, index = `LedId::index`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub leds: [Rgb; GRID_LEDS],
}

impl Default for Frame {
    fn default() -> Self {
        Frame { leds: [Rgb::BLACK; GRID_LEDS] }
    }
}

impl Frame {
    pub fn get(&self, id: LedId) -> Rgb {
        self.leds[id.index()]
    }

    /// Rows top to bottom, each `#RRGGBB` left to right (for text output).
    pub fn rows_top_down(&self) -> Vec<Vec<String>> {
        (0..GRID_H)
            .rev()
            .map(|y| (0..GRID_W).map(|x| self.leds[y * GRID_W + x].to_hex()).collect())
            .collect()
    }
}

/// Per-layer working buffer (premultiplied), allocated once per renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerFrame {
    pub px: [Rgba; GRID_LEDS],
}

impl Default for LayerFrame {
    fn default() -> Self {
        LayerFrame { px: [Rgba::TRANSPARENT; GRID_LEDS] }
    }
}

impl LayerFrame {
    pub fn clear(&mut self) {
        self.px = [Rgba::TRANSPARENT; GRID_LEDS];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn led_ids_use_a_bottom_left_origin() {
        assert_eq!(LedId::new(0, 0).unwrap().index(), 0);
        assert_eq!(LedId::new(7, 0).unwrap().index(), 7);
        assert_eq!(LedId::new(0, 7).unwrap().index(), 56);
        assert_eq!(LedId::from_index(63), LedId::new(7, 7));
        assert_eq!(LedId::new(8, 0), None);
        assert_eq!(LedId::from_index(64), None);
    }

    #[test]
    fn hex_colors() {
        let c = Rgb::from_hex("#163B75").unwrap();
        assert_eq!(c.to_hex(), "#163B75");
        assert_eq!(Rgb::from_hex("#00dcc8").unwrap().to_hex(), "#00DCC8");
        assert_eq!(Rgb::from_hex("163B75"), None);
        assert_eq!(Rgb::from_hex("#GG0000"), None);
    }

    #[test]
    fn text_rows_are_top_down() {
        let mut f = Frame::default();
        f.leds[LedId::new(0, 7).unwrap().index()] = Rgb::new(1.0, 0.0, 0.0);
        let rows = f.rows_top_down();
        assert_eq!(rows[0][0], "#FF0000");
        assert_eq!(rows[7][0], "#000000");
    }
}
