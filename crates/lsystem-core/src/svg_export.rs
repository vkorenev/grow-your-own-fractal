use glam::Vec2;

use crate::{Config, LineColorConfig, color_util::rgb_to_hsv, generate};

/// Generate an SVG string for the given config.
///
/// The SVG uses the natural turtle coordinate system, scaled to fit the fractal.
/// Colors match the GPU render exactly.
pub fn export_svg(config: &Config) -> String {
    let segments: Vec<[Vec2; 2]> = generate(&config.generation).collect();

    if segments.is_empty() {
        let bg = to_hex(config.colors.background);
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><rect width="1" height="1" fill="{bg}"/></svg>"#
        );
    }

    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for [a, b] in &segments {
        for p in [a, b] {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
    }

    // Pad degenerate (zero-width or zero-height) bounding boxes.
    let pad = config.generation.step * 0.5;
    if max_x == min_x {
        min_x -= pad;
        max_x += pad;
    }
    if max_y == min_y {
        min_y -= pad;
        max_y += pad;
    }

    let w = max_x - min_x;
    let h = max_y - min_y;
    let stroke_width = (w + h) * 0.0005;

    // Add a small margin so strokes near the edges are not clipped.
    let margin = (w + h) * 0.02;
    min_x -= margin;
    max_x += margin;
    min_y -= margin;
    max_y += margin;
    let w = max_x - min_x;
    let h = max_y - min_y;

    let bg = to_hex(config.colors.background);
    // SVG Y-axis is flipped relative to the turtle (math Y-up vs screen Y-down).
    // We use a group transform "matrix(1 0 0 -1 0 0)" so turtle coordinates can be
    // written as-is. The viewBox compensates: top of image = -max_y in SVG space.
    let neg_max_y = -max_y;

    let body = build_body(&segments, &config.colors.line);

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{min_x:.3} {neg_max_y:.3} {w:.3} {h:.3}\">\n\
        <rect x=\"{min_x:.3}\" y=\"{neg_max_y:.3}\" width=\"{w:.3}\" height=\"{h:.3}\" fill=\"{bg}\"/>\n\
        <g transform=\"matrix(1 0 0 -1 0 0)\" stroke-width=\"{stroke_width:.4}\" stroke-linecap=\"round\" fill=\"none\">\n\
        {body}\
        </g>\n\
        </svg>"
    )
}

fn build_body(segments: &[[Vec2; 2]], line: &LineColorConfig) -> String {
    match line {
        LineColorConfig::Solid { color: c } => {
            let color = to_hex(*c);
            let mut d = String::new();
            for [a, b] in segments {
                d.push_str(&format!("M{:.3},{:.3}L{:.3},{:.3}", a.x, a.y, b.x, b.y));
            }
            format!("<path d=\"{d}\" stroke=\"{color}\"/>\n")
        }
        LineColorConfig::Gradient { start, end } => {
            let n = segments.len();
            let denom = (n.max(2) - 1) as f32;
            let mut out = String::new();
            for (i, [a, b]) in segments.iter().enumerate() {
                let t = i as f32 / denom;
                let rgb = [
                    start[0] + (end[0] - start[0]) * t,
                    start[1] + (end[1] - start[1]) * t,
                    start[2] + (end[2] - start[2]) * t,
                ];
                let color = to_hex(rgb);
                out.push_str(&format!(
                    "<line x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\" stroke=\"{color}\"/>\n",
                    a.x, a.y, b.x, b.y
                ));
            }
            out
        }
        LineColorConfig::HueCycle { initial } => {
            let (start_hue, saturation, value) = rgb_to_hsv(*initial);
            let n = segments.len();
            let denom = (n.max(2) - 1) as f32;
            let mut out = String::new();
            for (i, [a, b]) in segments.iter().enumerate() {
                let t = i as f32 / denom;
                let hue = start_hue + t * 360.0;
                let rgb = hsv_to_rgb(hue, saturation, value);
                let color = to_hex(rgb);
                out.push_str(&format!(
                    "<line x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\" stroke=\"{color}\"/>\n",
                    a.x, a.y, b.x, b.y
                ));
            }
            out
        }
    }
}

/// Port of the WGSL `hsv_to_rgb` in `shader.wgsl`. Inputs must match the shader's
/// conventions: `h` is in any range (modulo 360 is applied internally), `s` and `v`
/// are in [0, 1].
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = (h % 360.0) / 60.0;
    let i = (h6 as u32) % 6;
    let f = h6 - h6.floor();
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let tc = v * (1.0 - (1.0 - f) * s);
    match i {
        0 => [v, tc, p],
        1 => [q, v, p],
        2 => [p, v, tc],
        3 => [p, q, v],
        4 => [tc, p, v],
        _ => [v, p, q],
    }
}

fn to_hex(rgb: [f32; 3]) -> String {
    let [r, g, b] = rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{r:02X}{g:02X}{b:02X}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConfigDocument, ConfigSource};

    fn make_config(extra_toml: &str) -> Config {
        let toml = format!(
            r#"[metadata]
name = "Test"

[l-system]
dimensions = 2
axiom = "F+F"
iterations = 1

[l-system.rules]

[turtle]
angle = 90.0
step = 1.0
initial_heading = 0.0

[colors]
background = [0.0, 0.0, 0.0]

[colors.line]
{extra_toml}
"#
        );
        ConfigDocument::try_from(ConfigSource::parse(&toml).unwrap())
            .unwrap()
            .into()
    }

    fn make_empty_config() -> Config {
        let toml = r#"[metadata]
name = "Empty"

[l-system]
dimensions = 2
axiom = "+"
iterations = 1

[l-system.rules]

[turtle]
angle = 90.0
step = 1.0
initial_heading = 0.0

[colors]
background = [0.0, 0.0, 0.0]

[colors.line]
mode = "solid"
color = [0.0, 0.9, 0.5]
"#;
        ConfigDocument::try_from(ConfigSource::parse(toml).unwrap())
            .unwrap()
            .into()
    }

    #[test]
    fn solid_contains_svg_and_color() {
        let cfg = make_config("mode = \"solid\"\ncolor = [1.0, 0.0, 0.0]");
        let svg = export_svg(&cfg);
        assert!(svg.contains("<svg"), "missing <svg tag");
        assert!(
            svg.contains("matrix(1 0 0 -1 0 0)"),
            "missing Y-flip transform"
        );
        assert!(svg.contains("#FF0000"), "missing solid color");
        assert!(svg.contains("<path"), "expected <path for solid mode");
    }

    #[test]
    fn gradient_first_and_last_segment_colors() {
        let cfg =
            make_config("mode = \"gradient\"\nstart = [1.0, 0.0, 0.0]\nend = [0.0, 0.0, 1.0]");
        let svg = export_svg(&cfg);
        assert!(svg.contains("<svg"), "missing <svg tag");
        assert!(
            svg.contains("matrix(1 0 0 -1 0 0)"),
            "missing Y-flip transform"
        );
        // 2 segments: t=0 → #FF0000, t=1 → #0000FF
        assert!(svg.contains("#FF0000"), "missing gradient start color");
        assert!(svg.contains("#0000FF"), "missing gradient end color");
    }

    #[test]
    fn hue_cycle_start_color() {
        let cfg = make_config("mode = \"hue_cycle\"\ninitial = [1.0, 0.0, 0.0]");
        let svg = export_svg(&cfg);
        assert!(svg.contains("<svg"), "missing <svg tag");
        assert!(
            svg.contains("matrix(1 0 0 -1 0 0)"),
            "missing Y-flip transform"
        );
        assert!(
            svg.contains("#FF0000"),
            "missing hue-cycle start color (red at hue=0)"
        );
    }

    #[test]
    fn empty_geometry_returns_minimal_svg() {
        let cfg = make_empty_config();
        let svg = export_svg(&cfg);
        assert!(svg.contains("<svg"), "missing <svg tag");
        assert!(!svg.contains("<path"), "unexpected <path in empty SVG");
        assert!(!svg.contains("<line"), "unexpected <line in empty SVG");
    }
}
