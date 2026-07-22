use glam::{Vec2, Vec3};

use crate::{
    AnyCompiledGeneration, ColorConfig, Config, DEFAULT_TEMPLATE_SEGMENT_BUDGET, GenerationConfig,
    LineColorConfig, PreparedGeneration, Rgb, Segment2DWithTopologicalDepth,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SvgExportError {
    #[error("SVG export supports only 2D generations")]
    ThreeDimensional,
}

/// Generate an SVG string for the given config.
///
/// The SVG uses the natural turtle coordinate system, scaled to fit the fractal.
/// Colors match the GPU render exactly.
pub fn export_svg(config: &Config) -> Result<String, SvgExportError> {
    let AnyCompiledGeneration::TwoD(generation) = config.generation.compile() else {
        return Err(SvgExportError::ThreeDimensional);
    };
    let colors = config.colors;
    let plan = generation.plan_templates(DEFAULT_TEMPLATE_SEGMENT_BUDGET);
    let has_stack_directives = plan.has_stack_directives();
    let prepared = plan.prepare();
    if colors.line.needs_topological_depth() && has_stack_directives {
        let mut segments: Vec<Segment2DWithTopologicalDepth> = Vec::new();
        match &prepared {
            PreparedGeneration::Stamped(set) => set
                .depth_segments()
                .for_each(|segment| segments.push(segment)),
            // for_each drives the pipeline's specialized `fold`; `collect`
            // would pull every symbol one at a time through `next`.
            PreparedGeneration::Interpreted(generation) => generation
                .depth_segments()
                .for_each(|segment| segments.push(segment)),
        }
        return Ok(export_svg_with_segments(
            &config.generation,
            &colors,
            segments.iter().map(|segment| segment.points),
            build_depth_body(&segments, &colors.line),
        ));
    }

    let mut segments: Vec<[Vec2; 2]> = Vec::new();
    match &prepared {
        PreparedGeneration::Stamped(set) => {
            set.segments().for_each(|segment| segments.push(segment))
        }
        // for_each drives the pipeline's specialized `fold` (see above).
        PreparedGeneration::Interpreted(generation) => generation
            .segments()
            .for_each(|segment| segments.push(segment)),
    }
    Ok(export_svg_with_segments(
        &config.generation,
        &colors,
        segments.iter().copied(),
        build_body(&segments, &colors.line),
    ))
}

fn export_svg_with_segments(
    generation: &GenerationConfig,
    colors: &ColorConfig,
    segments: impl IntoIterator<Item = [Vec2; 2]>,
    body: String,
) -> String {
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    let mut has_segments = false;
    for [a, b] in segments {
        has_segments = true;
        for p in [a, b] {
            min = min.min(p);
            max = max.max(p);
        }
    }
    if !has_segments {
        let bg = colors.background.to_string();
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><rect width="1" height="1" fill="{bg}"/></svg>"#
        );
    }

    // Pad degenerate (zero-width or zero-height) bounding boxes.
    let pad = generation.step * 0.5;
    if max.x == min.x {
        min.x -= pad;
        max.x += pad;
    }
    if max.y == min.y {
        min.y -= pad;
        max.y += pad;
    }

    let size = max - min;
    let stroke_width = (size.x + size.y) * 0.0005;

    // Add a small margin so strokes near the edges are not clipped.
    let margin = (size.x + size.y) * 0.02;
    min -= Vec2::splat(margin);
    max += Vec2::splat(margin);
    let size = max - min;
    let (min_x, w, h) = (min.x, size.x, size.y);

    let bg = colors.background.to_string();
    // SVG Y-axis is flipped relative to the turtle (math Y-up vs screen Y-down).
    // We use a group transform "matrix(1 0 0 -1 0 0)" so turtle coordinates can be
    // written as-is. The viewBox compensates: top of image = -max_y in SVG space.
    let neg_max_y = -max.y;

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
        LineColorConfig::Solid(c) => {
            let color = c.to_string();
            let mut d = String::new();
            for [a, b] in segments {
                d.push_str(&format!("M{:.3},{:.3}L{:.3},{:.3}", a.x, a.y, b.x, b.y));
            }
            format!("<path d=\"{d}\" stroke=\"{color}\"/>\n")
        }
        LineColorConfig::Gradient { start, end, .. } => {
            let start = rgb_to_vec3(*start);
            let end = rgb_to_vec3(*end);
            let n = segments.len();
            let denom = (n.max(2) - 1) as f32;
            let mut out = String::new();
            for (i, [a, b]) in segments.iter().enumerate() {
                let t = i as f32 / denom;
                let color = quantize_rgb(start.lerp(end, t));
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
                let color = quantize_rgb(hsv_to_rgb(hue, saturation, value));
                out.push_str(&format!(
                    "<line x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\" stroke=\"{color}\"/>\n",
                    a.x, a.y, b.x, b.y
                ));
            }
            out
        }
    }
}

fn build_depth_body(segments: &[Segment2DWithTopologicalDepth], line: &LineColorConfig) -> String {
    let LineColorConfig::Gradient {
        start,
        end,
        topological_depth: true,
    } = *line
    else {
        unreachable!("depth body requires topological-depth gradient line color")
    };
    let start = rgb_to_vec3(start);
    let end = rgb_to_vec3(end);
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
        let color = quantize_rgb(start.lerp(end, t));
        out.push_str(&format!(
            "<line x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\" stroke=\"{color}\"/>\n",
            a.x, a.y, b.x, b.y
        ));
    }
    out
}

/// Port of the WGSL `hsv_to_rgb` in `lsystem-renderer`'s `shaders/common.wesl`.
/// Inputs must match the shader's conventions: `h` is in any range (modulo 360
/// is applied internally), `s` and `v` are in [0, 1].
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Vec3 {
    let h6 = (h % 360.0) / 60.0;
    let i = (h6 as u32) % 6;
    let f = h6 - h6.floor();
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let tc = v * (1.0 - (1.0 - f) * s);
    match i {
        0 => Vec3::new(v, tc, p),
        1 => Vec3::new(q, v, p),
        2 => Vec3::new(p, v, tc),
        3 => Vec3::new(p, q, v),
        4 => Vec3::new(tc, p, v),
        _ => Vec3::new(v, p, q),
    }
}

/// Quantizes normalized RGB components to the byte-precision hex `Rgb` used
/// by the GPU renderer, so exported colors match the shader exactly.
fn quantize_rgb(rgb: Vec3) -> Rgb {
    let [r, g, b] = rgb
        .to_array()
        .map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    Rgb::new(r, g, b)
}

/// Inverse of [`quantize_rgb`]: normalizes byte-precision `Rgb` components to
/// `0.0..=1.0` for gradient lerping.
fn rgb_to_vec3(rgb: Rgb) -> Vec3 {
    Vec3::from(rgb.to_array())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{ColorConfig, Config, Dimensions, GenerationConfig};

    fn make_config(axiom: &str, line: LineColorConfig) -> Config {
        Config {
            name: "Test".to_string(),
            generation: GenerationConfig::new(
                Dimensions::TwoD,
                axiom.to_string(),
                1,
                90.0,
                1.0,
                0.0,
                BTreeMap::new(),
            )
            .expect("balanced config"),
            colors: ColorConfig {
                background: Rgb::new(0, 0, 0),
                line,
            },
        }
    }

    fn hex(s: &str) -> Rgb {
        Rgb::try_from(s).unwrap()
    }

    fn export_svg(config: &Config) -> String {
        super::export_svg(config).expect("2D SVG export succeeds")
    }

    #[test]
    fn solid_contains_svg_and_color() {
        let cfg = make_config("F+F", LineColorConfig::Solid(hex("#ff0000")));
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
            "F+F",
            LineColorConfig::Gradient {
                start: hex("#ff0000"),
                end: hex("#0000ff"),
                topological_depth: false,
            },
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
            "F+F",
            LineColorConfig::HueCycle {
                initial: hex("#ff0000"),
            },
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
    fn topological_gradient_colors_equal_topological_depth_equally() {
        // Bracketed axiom so has_stack_directives() is true → depth path is taken.
        let cfg = make_config(
            "F[+F]F",
            LineColorConfig::Gradient {
                start: hex("#ff0000"),
                end: hex("#0000ff"),
                topological_depth: true,
            },
        );
        let svg = export_svg(&cfg);

        assert!(
            svg.contains("#ff0000"),
            "missing topological-gradient start color"
        );
        assert_eq!(
            svg.matches("#0000ff").count(),
            2,
            "two depth-1 segments should use the same end color"
        );
    }

    #[test]
    fn topological_gradient_single_segment_uses_start_color() {
        // Bracketless axiom: has_stack_directives() is false → plain traversal path.
        // For a single segment, traversal gradient t=0 → start color only.
        let cfg = make_config(
            "F",
            LineColorConfig::Gradient {
                start: hex("#ff0000"),
                end: hex("#0000ff"),
                topological_depth: true,
            },
        );
        let svg = export_svg(&cfg);

        assert!(svg.contains("#ff0000"), "missing start color");
        assert!(
            !svg.contains("#0000ff"),
            "single segment should not use end color"
        );
    }

    #[test]
    fn empty_geometry_returns_minimal_svg() {
        let cfg = make_config("+", LineColorConfig::Solid(hex("#00e680")));
        let svg = export_svg(&cfg);
        assert!(svg.contains("<svg"), "missing <svg tag");
        assert!(!svg.contains("<path"), "unexpected <path in empty SVG");
        assert!(!svg.contains("<line"), "unexpected <line in empty SVG");
    }

    #[test]
    fn topological_gradient_empty_geometry_returns_minimal_svg() {
        // Bracketless axiom with topological_depth gradient → plain path (no stack directives).
        let cfg = make_config(
            "+",
            LineColorConfig::Gradient {
                start: hex("#ff0000"),
                end: hex("#0000ff"),
                topological_depth: true,
            },
        );
        let svg = export_svg(&cfg);

        assert!(svg.contains("<svg"), "missing <svg tag");
        assert!(!svg.contains("<path"), "unexpected <path in empty SVG");
        assert!(!svg.contains("<line"), "unexpected <line in empty SVG");
    }

    #[test]
    fn missing_background_uses_default_black_fill() {
        // Config is built with explicit black background, same as "omitted" case resolves to.
        let cfg = make_config("F+F", LineColorConfig::Solid(hex("#ff0000")));
        let svg = export_svg(&cfg);

        assert!(svg.contains("fill=\"#000000\""));
    }

    #[test]
    fn three_dimensional_generation_is_rejected() {
        let mut config = make_config("F", LineColorConfig::Solid(hex("#ff0000")));
        config.generation = GenerationConfig::new(
            crate::Dimensions::ThreeD,
            "F".to_string(),
            0,
            90.0,
            1.0,
            0.0,
            BTreeMap::new(),
        )
        .expect("balanced config");

        let error = super::export_svg(&config).expect_err("3D SVG export must fail");
        assert_eq!(error, SvgExportError::ThreeDimensional);
        assert_eq!(error.to_string(), "SVG export supports only 2D generations");
    }
}
