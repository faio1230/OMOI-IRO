//! Color quantization to device formats. MVP: values are used as-is
//! (no calibration or gamma, docs/02 "再現可能な評価とレイヤー").

use omoiiro_core::surface::Rgb;

/// 0..=1 per channel -> 0..=127, rounded and clamped.
pub fn quantize_rgb7(c: Rgb) -> [u8; 3] {
    let q = |v: f32| if v.is_nan() { 0 } else { (v.clamp(0.0, 1.0) * 127.0).round() as u8 };
    [q(c.r), q(c.g), q(c.b)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb7_rounds_and_clamps() {
        assert_eq!(quantize_rgb7(Rgb::new(0.0, 0.5, 1.0)), [0, 64, 127]);
        assert_eq!(quantize_rgb7(Rgb::new(-1.0, 2.0, f32::NAN)), [0, 127, 0]);
        assert_eq!(quantize_rgb7(Rgb::new(1.0 / 127.0, 0.4 / 127.0, 0.6 / 127.0)), [1, 0, 1]);
    }
}
