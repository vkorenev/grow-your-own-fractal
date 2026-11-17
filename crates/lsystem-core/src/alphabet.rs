use crate::config::{ConfigError, Dimensions};

pub(crate) const TERMINALS_UNIVERSAL: &str = "Ff+-|[]";
pub(crate) const TERMINALS_3D: &str = "&^/\\";

pub fn validate_symbols(
    chars: &str,
    field: &str,
    dimensions: Dimensions,
) -> Result<(), ConfigError> {
    for (position, ch) in chars.chars().enumerate() {
        if ch.is_alphabetic() {
            continue;
        }
        if TERMINALS_UNIVERSAL.contains(ch) {
            continue;
        }
        if TERMINALS_3D.contains(ch) {
            if dimensions == Dimensions::ThreeD {
                continue;
            }
            return Err(ConfigError::ThreeDOnlySymbol {
                ch,
                field: field.to_string(),
                position,
            });
        }
        return Err(ConfigError::InvalidSymbol {
            ch,
            field: field.to_string(),
            position,
        });
    }
    Ok(())
}

pub fn validate_bracket_balance(chars: &str, field: &str) -> Result<(), ConfigError> {
    let mut depth: usize = 0;
    let mut first_open_pos: Option<usize> = None;
    for (position, ch) in chars.chars().enumerate() {
        match ch {
            '[' => {
                if depth == 0 {
                    first_open_pos = Some(position);
                }
                depth += 1;
            }
            ']' => {
                if depth == 0 {
                    return Err(ConfigError::UnmatchedClose {
                        field: field.to_string(),
                        position,
                    });
                }
                depth -= 1;
                if depth == 0 {
                    first_open_pos = None;
                }
            }
            _ => {}
        }
    }
    if depth > 0 {
        return Err(ConfigError::UnmatchedOpen {
            field: field.to_string(),
            position: first_open_pos.unwrap(),
        });
    }
    Ok(())
}

pub fn contains_3d_symbols(s: &str) -> bool {
    s.chars().any(|c| TERMINALS_3D.contains(c))
}

#[cfg(test)]
mod tests {
    use super::{contains_3d_symbols, validate_symbols};
    use crate::config::{ConfigError, Dimensions};

    #[test]
    fn reports_3d_symbols_in_2d_as_3d_only() {
        assert_eq!(
            validate_symbols("F+F&F", "axiom", Dimensions::TwoD),
            Err(ConfigError::ThreeDOnlySymbol {
                ch: '&',
                field: "axiom".to_string(),
                position: 3,
            })
        );
        assert_eq!(
            validate_symbols("F+F&F", "axiom", Dimensions::ThreeD),
            Ok(())
        );
        assert!(matches!(
            validate_symbols("F!", "axiom", Dimensions::TwoD),
            Err(ConfigError::InvalidSymbol { ch: '!', .. })
        ));
    }

    #[test]
    fn recognizes_all_3d_symbols() {
        assert!(contains_3d_symbols("F&F"));
        assert!(contains_3d_symbols("F^F"));
        assert!(contains_3d_symbols("F/F"));
        assert!(contains_3d_symbols("F\\F"));
    }

    #[test]
    fn returns_false_for_2d_only_strings() {
        assert!(!contains_3d_symbols("F+F-F|[]"));
    }
}
