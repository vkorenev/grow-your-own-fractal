//! Validation of the structured grammar editor's draft, shared by any
//! frontend that edits grammar row by row rather than as raw TOML.

use lsystem_core::{ConfigError, Dimensions, contains_3d_symbols};

use crate::editor_config::ValidGrammar;

/// Why a grammar-editor draft can't be applied. Frontends turn these into
/// their own user-facing text.
#[derive(Debug, Clone, PartialEq)]
pub enum GrammarDraftError {
    /// A row has a right-hand side but no symbol.
    MissingRuleSymbol,
    /// Two rows rewrite the same symbol.
    DuplicateRuleSymbol(char),
    /// 2D is selected but the draft uses 3D-only symbols.
    ThreeDSymbolsIn2D,
    /// A check shared with config parsing failed (rule symbol, alphabet,
    /// brackets).
    Invalid(ConfigError),
}

/// Checks the rows' structure and returns the rules they define. Rows whose
/// symbol and right-hand side are both blank (e.g. one just added with
/// "Add rule") are skipped rather than reported.
fn grammar_draft_rules(rows: &[(String, String)]) -> Result<Vec<(char, &str)>, GrammarDraftError> {
    let mut seen = std::collections::HashSet::new();
    let mut rules = Vec::with_capacity(rows.len());
    for (symbol, rhs) in rows {
        let symbol = symbol.trim();
        if symbol.is_empty() && rhs.trim().is_empty() {
            continue;
        }
        let Some(c) = symbol.chars().next() else {
            return Err(GrammarDraftError::MissingRuleSymbol);
        };
        if !seen.insert(c) {
            return Err(GrammarDraftError::DuplicateRuleSymbol(c));
        }
        rules.push((c, rhs.as_str()));
    }
    Ok(rules)
}

/// Validates a draft with the same checks config parsing uses
/// ([`ValidGrammar::new`]), returning the normalized grammar to apply, or
/// every independent problem found: at most one structural or shared-check
/// error, plus [`GrammarDraftError::ThreeDSymbolsIn2D`] when it applies, so
/// a 3D-symbol problem is never hidden behind another error.
pub fn validate_grammar_draft(
    dimensions: Dimensions,
    axiom: &str,
    rows: &[(String, String)],
) -> Result<ValidGrammar, Vec<GrammarDraftError>> {
    let three_d_in_2d = dimensions == Dimensions::TwoD
        && (contains_3d_symbols(axiom) || rows.iter().any(|(_, rhs)| contains_3d_symbols(rhs)));
    // With 3D symbols already reported as one problem, check the rest as
    // 3D so each 3D symbol isn't reported again on its own.
    let checked_as = if three_d_in_2d {
        Dimensions::ThreeD
    } else {
        dimensions
    };
    let mut errors = Vec::new();
    let grammar = match grammar_draft_rules(rows) {
        Ok(rules) => ValidGrammar::new(checked_as, axiom, rules)
            .map_err(|e| errors.push(GrammarDraftError::Invalid(e)))
            .ok(),
        Err(e) => {
            errors.push(e);
            None
        }
    };
    if three_d_in_2d {
        errors.push(GrammarDraftError::ThreeDSymbolsIn2D);
    }
    match grammar {
        Some(grammar) if errors.is_empty() => Ok(grammar),
        _ => Err(errors),
    }
}

#[cfg(test)]
mod tests {
    use super::{GrammarDraftError, grammar_draft_rules, validate_grammar_draft};
    use lsystem_core::ConfigError;
    use lsystem_core::Dimensions::{ThreeD, TwoD};

    fn rows(rows: &[(&str, &str)]) -> Vec<(String, String)> {
        rows.iter()
            .map(|(s, r)| (s.to_string(), r.to_string()))
            .collect()
    }

    fn invalid(result: Result<impl std::fmt::Debug, Vec<GrammarDraftError>>) -> ConfigError {
        match result.unwrap_err().as_slice() {
            [GrammarDraftError::Invalid(e)] => e.clone(),
            other => panic!("expected one Invalid error, got {other:?}"),
        }
    }

    #[test]
    fn blank_rows_are_skipped() {
        let draft = rows(&[("F", "F+F"), ("", ""), (" ", "  ")]);
        assert_eq!(grammar_draft_rules(&draft), Ok(vec![('F', "F+F")]));
    }

    #[test]
    fn row_with_rhs_but_no_symbol_is_rejected() {
        assert_eq!(
            grammar_draft_rules(&rows(&[("", "F+F")])),
            Err(GrammarDraftError::MissingRuleSymbol)
        );
    }

    #[test]
    fn duplicate_symbol_is_rejected() {
        assert_eq!(
            grammar_draft_rules(&rows(&[("F", "FF"), ("F", "F-F")])),
            Err(GrammarDraftError::DuplicateRuleSymbol('F'))
        );
    }

    #[test]
    fn valid_draft_is_normalized() {
        let grammar =
            validate_grammar_draft(TwoD, "F X", &rows(&[("X", "F [+X]"), ("", ""), ("F", "")]))
                .unwrap();
        assert_eq!(grammar.dimensions(), TwoD);
        assert_eq!(grammar.axiom(), "FX");
        assert_eq!(grammar.rules()[&'X'], "F[+X]");
        assert_eq!(grammar.rules()[&'F'], "");
    }

    #[test]
    fn rejects_symbols_outside_the_alphabet() {
        assert!(matches!(
            invalid(validate_grammar_draft(TwoD, "F!", &rows(&[]))),
            ConfigError::InvalidSymbol { ch: '!', .. }
        ));
        assert!(matches!(
            invalid(validate_grammar_draft(TwoD, "F", &rows(&[("F", "F%")]))),
            ConfigError::InvalidSymbol { ch: '%', .. }
        ));
    }

    #[test]
    fn rejects_unbalanced_brackets() {
        assert!(matches!(
            invalid(validate_grammar_draft(TwoD, "F[", &rows(&[]))),
            ConfigError::UnmatchedOpen { .. }
        ));
        assert!(matches!(
            invalid(validate_grammar_draft(TwoD, "F", &rows(&[("F", "]F")]))),
            ConfigError::UnmatchedClose { .. }
        ));
    }

    #[test]
    fn rejects_non_letter_rule_symbol() {
        assert!(matches!(
            invalid(validate_grammar_draft(TwoD, "F", &rows(&[("+", "F")]))),
            ConfigError::InvalidRuleKey { .. }
        ));
    }

    #[test]
    fn three_d_symbols_depend_only_on_draft_dimension() {
        let draft = rows(&[("F", "F&F")]);
        assert_eq!(
            validate_grammar_draft(TwoD, "F", &draft),
            Err(vec![GrammarDraftError::ThreeDSymbolsIn2D])
        );
        let grammar = validate_grammar_draft(ThreeD, "F", &draft).unwrap();
        assert_eq!(grammar.dimensions(), ThreeD);
    }

    #[test]
    fn three_d_error_is_reported_alongside_other_errors() {
        assert_eq!(
            validate_grammar_draft(TwoD, "F", &rows(&[("F", "F&F"), ("F", "FF")])),
            Err(vec![
                GrammarDraftError::DuplicateRuleSymbol('F'),
                GrammarDraftError::ThreeDSymbolsIn2D,
            ])
        );
        let errors = validate_grammar_draft(TwoD, "F&!", &rows(&[])).unwrap_err();
        assert!(
            matches!(
                errors.as_slice(),
                [
                    GrammarDraftError::Invalid(ConfigError::InvalidSymbol { ch: '!', .. }),
                    GrammarDraftError::ThreeDSymbolsIn2D,
                ]
            ),
            "{errors:?}"
        );
    }
}
