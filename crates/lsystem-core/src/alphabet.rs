use crate::config::ConfigError;

const TERMINALS_UNIVERSAL: &str = "Ff+-|[]";

pub fn validate_symbols(chars: &str, field: &str) -> Result<(), ConfigError> {
    for (position, ch) in chars.chars().enumerate() {
        if ch.is_ascii_alphabetic() {
            continue;
        }
        if TERMINALS_UNIVERSAL.contains(ch) {
            continue;
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
