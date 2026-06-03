use glam::Vec2;

use crate::{
    ColorConfig, Config, GenerationConfig, LineColorConfig, Segment2DWithTopologicalDepth,
    generate, generate_with_topological_depth,
};

/// Generate an SVG string for the given config.
///
/// The SVG uses the natural turtle coordinate system, scaled to fit the fractal.
/// Colors match the GPU render exactly.
pub fn export_svg(config: &Config) -> String {
    let colors = config.effective_colors();
    if colors.line.needs_topological_depth() {
        let segments: Vec<Segment2DWithTopologicalDepth> =
            generate_with_topological_depth(&config.generation).collect();
        return export_svg_with_segments(
            &config.generation,
            &colors,
            segments.iter().map(|segment| segment.points),
            build_depth_body(&segments, &colors.line),
        );
    }

    let segments: Vec<[Vec2; 2]> = generate(&config.generation).collect();
    export_svg_with_segments(
        &config.generation,
        &colors,
        segments.iter().copied(),
        build_body(&segments, &colors.line),
    )
}

fn export_svg_with_segments(
    generation: &GenerationConfig,
    colors: &ColorConfig,
    segments: impl IntoIterator<Item = [Vec2; 2]>,
    body: String,
) -> String {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    let mut has_segments = false;
    for [a, b] in segments {
        has_segments = true;
        for p in [a, b] {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
    }
    if !has_segments {
        let bg = colors.effective_background().to_css_hex();
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><rect width="1" height="1" fill="{bg}"/></svg>"#
        );
    }

    // Pad degenerate (zero-width or zero-height) bounding boxes.
    let pad = generation.step * 0.5;
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

    let bg = colors.effective_background().to_css_hex();
    // SVG Y-axis is flipped relative to the turtle (math Y-up vs screen Y-down).
    // We use a group transform "matrix(1 0 0 -1 0 0)" so turtle coordinates can be
    // written as-is. The viewBox compensates: top of image = -max_y in SVG space.
    let neg_max_y = -max_y;

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{min_x:.3} {neg_max_y:.3} {w:.3} {h:.3}">
<rect x="{min_x:.3}" y="{neg_max_y:.3}" width="{w:.3}" height="{h:.3}" fill="{bg}"/>
<g transform="matrix(1 0 0 -1 0 0)" stroke-width="{stroke_width:.4}" stroke-linecap="round" fill="none">
{body}</g>
</svg>"##
    )
}

fn build_body(segments: &[[Vec2; 2]], line: &LineColorConfig) -> String {
    match line {
        LineColorConfig::Solid { color: c } => {
            let color = c.to_css_hex();
            let mut d = String::new();
            for [a, b] in segments {
                d.push_str(&format!("M{:.3},{:.3}L{:.3},{:.3}", a.x, a.y, b.x, b.y));
            }
            format!("<path d=\"{d}\" stroke=\"{color}\"/>\n")
        }
        LineColorConfig::Gradient { start, end } => {
            let start = start.to_f32_array();
            let end = end.to_f32_array();
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
            let (start_hue, saturation, value) = initial.to_hsv();
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
        LineColorConfig::DepthGradient { .. } => {
            unreachable!("depth-gradient SVG is built from topological-depth segments")
        }
    }
}

fn build_depth_body(segments: &[Segment2DWithTopologicalDepth], line: &LineColorConfig) -> String {
    let LineColorConfig::DepthGradient { start, end } = *line else {
        unreachable!("depth body requires depth-gradient line color")
    };
    let start = start.to_f32_array();
    let end = end.to_f32_array();
    let max_topological_depth = segments
        .iter()
        .map(|segment| segment.topological_depth)
        .max()
        .unwrap_or(0)
        .max(1) as f32;
    let mut out = String::new();
    for segment in segments {
        let [a, b] = segment.points;
        let t = segment.topological_depth as f32 / max_topological_depth;
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
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConfigDocument, ConfigSource};

    fn make_config(extra_toml: &str) -> Config {
        make_config_with_axiom("F+F", extra_toml)
    }

    fn make_config_with_axiom(axiom: &str, extra_toml: &str) -> Config {
        let toml = format!(
            r##"[metadata]
name = "Test"

[l-system]
dimensions = "2D"
axiom = "{axiom}"
iterations = 1

[l-system.rules]

[turtle]
angle = 90.0
step = 1.0
initial_heading = 0.0

[colors]
background = "#000000"

[colors.line]
{extra_toml}
"##
        );
        ConfigDocument::try_from(ConfigSource::parse(&toml).unwrap())
            .unwrap()
            .into()
    }

    fn make_empty_config() -> Config {
        let toml = r##"[metadata]
name = "Empty"

[l-system]
dimensions = "2D"
axiom = "+"
iterations = 1

[l-system.rules]

[turtle]
angle = 90.0
step = 1.0
initial_heading = 0.0

[colors]
background = "#000000"

[colors.line]
mode = "solid"
color = "#00e680"
"##;
        ConfigDocument::try_from(ConfigSource::parse(toml).unwrap())
            .unwrap()
            .into()
    }

    fn make_config_without_background() -> Config {
        let toml = r##"[metadata]
name = "No Background"

[l-system]
dimensions = "2D"
axiom = "F+F"
iterations = 1

[l-system.rules]

[turtle]
angle = 90.0
step = 1.0
initial_heading = 0.0

[colors]

[colors.line]
mode = "solid"
color = "#ff0000"
"##;
        ConfigDocument::try_from(ConfigSource::parse(toml).unwrap())
            .unwrap()
            .into()
    }

    #[test]
    fn solid_contains_svg_and_color() {
        let cfg = make_config(
            r##"mode = "solid"
color = "#ff0000""##,
        );
        let svg = export_svg(&cfg);
        assert!(svg.contains("<svg"), "missing <svg tag");
        assert!(
            svg.contains("matrix(1 0 0 -1 0 0)"),
            "missing Y-flip transform"
        );
        assert!(svg.contains("#ff0000"), "missing solid color");
        assert!(svg.contains("<path"), "expected <path for solid mode");
    }

    #[test]
    fn gradient_first_and_last_segment_colors() {
        let cfg = make_config(
            r##"mode = "gradient"
start = "#ff0000"
end = "#0000ff""##,
        );
        let svg = export_svg(&cfg);
        assert!(svg.contains("<svg"), "missing <svg tag");
        assert!(
            svg.contains("matrix(1 0 0 -1 0 0)"),
            "missing Y-flip transform"
        );
        // 2 segments: t=0 → #ff0000, t=1 → #0000ff (interpolated via float, emitted lowercase)
        assert!(svg.contains("#ff0000"), "missing gradient start color");
        assert!(svg.contains("#0000ff"), "missing gradient end color");
    }

    #[test]
    fn hue_cycle_start_color() {
        let cfg = make_config(
            r##"mode = "hue_cycle"
initial = "#ff0000""##,
        );
        let svg = export_svg(&cfg);
        assert!(svg.contains("<svg"), "missing <svg tag");
        assert!(
            svg.contains("matrix(1 0 0 -1 0 0)"),
            "missing Y-flip transform"
        );
        assert!(
            svg.contains("#ff0000"),
            "missing hue-cycle start color (red at hue=0)"
        );
    }

    #[test]
    fn depth_gradient_colors_equal_topological_depth_equally() {
        let cfg = make_config_with_axiom(
            "F[+F]F",
            r##"mode = "depth_gradient"
start = "#ff0000"
end = "#0000ff""##,
        );
        let svg = export_svg(&cfg);

        assert!(
            svg.contains("#ff0000"),
            "missing depth-gradient start color"
        );
        assert_eq!(
            svg.matches("#0000ff").count(),
            2,
            "two depth-1 segments should use the same end color"
        );
    }

    #[test]
    fn depth_gradient_single_segment_uses_start_color() {
        let cfg = make_config_with_axiom(
            "F",
            r##"mode = "depth_gradient"
start = "#ff0000"
end = "#0000ff""##,
        );
        let svg = export_svg(&cfg);

        assert!(svg.contains("#ff0000"), "missing depth-0 start color");
        assert!(
            !svg.contains("#0000ff"),
            "single depth-0 segment should not use end color"
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

    #[test]
    fn depth_gradient_empty_geometry_returns_minimal_svg() {
        let cfg = make_config_with_axiom(
            "+",
            r##"mode = "depth_gradient"
start = "#ff0000"
end = "#0000ff""##,
        );
        let svg = export_svg(&cfg);

        assert!(svg.contains("<svg"), "missing <svg tag");
        assert!(!svg.contains("<path"), "unexpected <path in empty SVG");
        assert!(!svg.contains("<line"), "unexpected <line in empty SVG");
    }

    #[test]
    fn missing_background_uses_default_black_fill() {
        let cfg = make_config_without_background();
        let svg = export_svg(&cfg);

        assert!(svg.contains("fill=\"#000000\""));
    }
}
