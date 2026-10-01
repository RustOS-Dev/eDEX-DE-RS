//! Night light: a colour temperature applied through each CRTC's gamma ramp (what hyprsunset
//! did on Hyprland).

/// White point of a black body at `kelvin`, as channel multipliers 0..=1 (Tanner Helland's fit,
/// the same curve redshift/wlsunset use). 6500 K is neutral.
pub fn whitepoint(kelvin: u32) -> [f64; 3] {
    let t = (kelvin.clamp(1000, 40000) as f64) / 100.0;
    let r = if t <= 66.0 {
        255.0
    } else {
        329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2)
    };
    let g = if t <= 66.0 {
        99.470_802_586_1 * t.ln() - 161.119_568_166_1
    } else {
        288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
    };
    let n = |v: f64| (v / 255.0).clamp(0.0, 1.0);
    let (r, g, b) = (n(r), n(g), n(b));
    // Normalise so 6500 K is exactly neutral.
    let max = r.max(g).max(b);
    [r / max, g / max, b / max]
}

/// Gamma ramps of `size` entries for `kelvin` (`None` = identity).
pub fn ramps(size: usize, kelvin: Option<u32>) -> (Vec<u16>, Vec<u16>, Vec<u16>) {
    let wp = kelvin.map(whitepoint).unwrap_or([1.0; 3]);
    let ramp = |mul: f64| -> Vec<u16> {
        (0..size)
            .map(|i| {
                let v = if size > 1 {
                    i as f64 / (size - 1) as f64
                } else {
                    1.0
                };
                (v * mul * 65535.0).round().clamp(0.0, 65535.0) as u16
            })
            .collect()
    };
    (ramp(wp[0]), ramp(wp[1]), ramp(wp[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_and_warm() {
        let n = whitepoint(6500);
        assert!(n.iter().all(|c| *c > 0.97));
        let warm = whitepoint(3000);
        assert_eq!(warm[0], 1.0);
        assert!(warm[1] < 0.8 && warm[2] < warm[1]);
        let (r, g, b) = ramps(256, Some(3000));
        assert_eq!(r.len(), 256);
        assert_eq!(r[0], 0);
        assert_eq!(r[255], 65535);
        assert!(b[255] < g[255]);
        let (r, _, b) = ramps(4, None);
        assert_eq!(r, b);
        assert_eq!(r, vec![0, 21845, 43690, 65535]);
    }
}
