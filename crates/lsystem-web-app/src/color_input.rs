/// Converts an RGB color with components in `0.0..=1.0` to a CSS `#rrggbb`
/// color string.
pub(crate) fn rgb_to_hex(rgb: [f32; 3]) -> String {
    debug_assert!(rgb.iter().all(|component| (0.0..=1.0).contains(component)));
    let [r, g, b] = rgb.map(|component| (component.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Parses a CSS `#rrggbb` color string into normalized RGB components.
pub(crate) fn hex_to_rgb(hex: &str) -> Option<[f32; 3]> {
    let bytes = hex.strip_prefix('#')?.as_bytes();
    if bytes.len() != 6 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }

    let parse_pair = |range| {
        let pair = std::str::from_utf8(&bytes[range]).ok()?;
        u8::from_str_radix(pair, 16).ok()
    };
    let r = parse_pair(0..2)?;
    let g = parse_pair(2..4)?;
    let b = parse_pair(4..6)?;
    Some([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_to_hex_rounds_components_to_byte_values() {
        assert_eq!(rgb_to_hex([0.0, 0.5, 1.0]), "#0080ff");
        assert_eq!(rgb_to_hex([1.0, 0.0, 0.25]), "#ff0040");
    }

    #[test]
    fn hex_to_rgb_parses_css_color_input_values() {
        assert_eq!(hex_to_rgb("#0080ff"), Some([0.0, 128.0 / 255.0, 1.0]));
        assert_eq!(hex_to_rgb("#FF0040"), Some([1.0, 0.0, 64.0 / 255.0]));
    }

    #[test]
    fn hex_to_rgb_rejects_non_six_digit_hex_values() {
        assert_eq!(hex_to_rgb("0080ff"), None);
        assert_eq!(hex_to_rgb("#080"), None);
        assert_eq!(hex_to_rgb("#0080ff00"), None);
        assert_eq!(hex_to_rgb("#0080fg"), None);
        assert_eq!(hex_to_rgb("#0é80f"), None);
        assert_eq!(hex_to_rgb("#００８０ｆｆ"), None);
    }
}
