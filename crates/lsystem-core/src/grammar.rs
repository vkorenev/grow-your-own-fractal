use std::collections::HashMap;

pub(crate) struct ExpandIter<'a> {
    stack: Vec<(std::str::Chars<'a>, u32)>,
    rules: &'a HashMap<char, String>,
}

impl<'a> Iterator for ExpandIter<'a> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        loop {
            let top = self.stack.last_mut()?;
            let depth = top.1;
            match top.0.next() {
                None => {
                    self.stack.pop();
                }
                Some(ch) => {
                    if depth > 0
                        && let Some(rhs) = self.rules.get(&ch)
                    {
                        self.stack.push((rhs.chars(), depth - 1));
                        continue;
                    }
                    return Some(ch);
                }
            }
        }
    }
}

pub fn expand<'a>(
    axiom: &'a str,
    rules: &'a HashMap<char, String>,
    iterations: u32,
) -> impl Iterator<Item = char> + 'a {
    ExpandIter {
        stack: vec![(axiom.chars(), iterations)],
        rules,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn koch_rules() -> HashMap<char, String> {
        [('F', "F-F++F-F".to_string())].into()
    }

    #[test]
    fn zero_iterations_returns_axiom() {
        let result: String = expand("F++F++F", &koch_rules(), 0).collect();
        assert_eq!(result, "F++F++F");
    }

    #[test]
    fn one_iteration_expands_f() {
        let result: String = expand("F++F++F", &koch_rules(), 1).collect();
        // Each F → F-F++F-F; the ++ between each F are carried through.
        assert_eq!(result, "F-F++F-F++F-F++F-F++F-F++F-F");
    }

    #[test]
    fn f_count_grows_as_power_of_four() {
        // Koch snowflake: 3 F's at iter 0, multiplied by 4 each iteration.
        let rules = koch_rules();
        for iter in 0..=4u32 {
            let f_count = expand("F++F++F", &rules, iter)
                .filter(|&c| c == 'F')
                .count();
            assert_eq!(f_count, 3 * 4usize.pow(iter), "iter {iter}");
        }
    }

    #[test]
    fn undefined_symbols_pass_through() {
        // X has no rule; F maps to FX.
        // iter 1: "F" → "FX"
        // iter 2: F→FX, X→X → "FXX"
        let rules: HashMap<char, String> = [('F', "FX".to_string())].into();
        let result: String = expand("F", &rules, 2).collect();
        assert_eq!(result, "FXX");
    }

    #[test]
    fn expand_interleaves_multiple_rules() {
        // Rules: A → aA, B → Bb  (each rule adds a terminal and recurses).
        // iter 0: "AB"
        // iter 1: A→aA, B→Bb  →  "aABb"
        // iter 2: a→a, A→aA, B→Bb, b→b  →  "aaABbb"
        let rules: HashMap<char, String> =
            [('A', "aA".to_string()), ('B', "Bb".to_string())].into();
        let result: String = expand("AB", &rules, 2).collect();
        assert_eq!(result, "aaABbb");
    }
}
