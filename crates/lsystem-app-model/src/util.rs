/// Converts a name to a safe ASCII filename with the given extension.
/// ASCII alphanumeric characters are lowercased; all other characters become `_`.
pub fn sanitize_filename(name: &str, extension: &str) -> String {
    let base: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{base}.{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_alphanumeric_lowercased() {
        assert_eq!(sanitize_filename("Koch", "svg"), "koch.svg");
        assert_eq!(
            sanitize_filename("DragonCurve3D", "png"),
            "dragoncurve3d.png"
        );
    }

    #[test]
    fn spaces_and_punctuation_become_underscores() {
        assert_eq!(sanitize_filename("Koch Curve", "svg"), "koch_curve.svg");
        assert_eq!(sanitize_filename("L-System!", "png"), "l_system_.png");
    }

    #[test]
    fn non_ascii_become_underscores() {
        assert_eq!(sanitize_filename("étoile", "svg"), "_toile.svg");
        assert_eq!(sanitize_filename("ñ", "png"), "_.png");
    }

    #[test]
    fn empty_name_produces_bare_extension() {
        assert_eq!(sanitize_filename("", "svg"), ".svg");
    }

    #[test]
    fn consecutive_specials_are_not_collapsed() {
        assert_eq!(sanitize_filename("A  B", "svg"), "a__b.svg");
    }
}
