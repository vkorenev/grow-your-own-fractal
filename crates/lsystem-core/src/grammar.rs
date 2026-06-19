use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct ExpandIter<'a> {
    stack: Vec<(std::str::Chars<'a>, u32)>,
    rules: &'a BTreeMap<char, String>,
}

impl Iterator for ExpandIter<'_> {
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

pub(crate) fn expand<'a>(
    axiom: &'a str,
    rules: &'a BTreeMap<char, String>,
    iterations: u32,
) -> ExpandIter<'a> {
    ExpandIter {
        stack: vec![(axiom.chars(), iterations)],
        rules,
    }
}

/// Returns the maximum iteration count for which the total number of drawn segments
/// (produced by `F` symbols) does not exceed `max_segments`.
///
/// Uses symbolic growth tracking: iterates the per-character segment yield one step at
/// a time without materialising any strings. Saturating arithmetic prevents overflow for
/// fast-growing systems. Hard-capped at 30 so the loop always terminates.
pub fn max_safe_iterations(axiom: &str, rules: &BTreeMap<char, String>, max_segments: u64) -> u32 {
    const HARD_MAX: u32 = 30;

    let axiom_counts: BTreeMap<char, u64> = axiom.chars().fold(BTreeMap::new(), |mut m, c| {
        *m.entry(c).or_insert(0) += 1;
        m
    });

    let total = |yields: &BTreeMap<char, u64>| -> u64 {
        axiom_counts
            .iter()
            .map(|(c, n)| n.saturating_mul(*yields.get(c).unwrap_or(&0)))
            .fold(0u64, |a, x| a.saturating_add(x))
    };

    let mut yields: BTreeMap<char, u64> = [('F', 1u64)].into();

    let mut updates: Vec<(char, u64)> = Vec::with_capacity(rules.len());
    for n in 0..=HARD_MAX {
        if total(&yields) > max_segments {
            return n.saturating_sub(1);
        }
        updates.clear();
        for (c, rhs) in rules {
            let v = rhs
                .chars()
                .map(|ch| *yields.get(&ch).unwrap_or(&0))
                .fold(0u64, |a, x| a.saturating_add(x));
            updates.push((*c, v));
        }
        for (c, v) in updates.drain(..) {
            yields.insert(c, v);
        }
    }
    HARD_MAX
}

/// Returns the rule symbols that are never reached during expansion of `axiom`.
///
/// A rule is reachable if its symbol appears in `axiom`, or in the RHS of any
/// other reachable rule. Unreachable rules are dead: `expand` never invokes them,
/// no matter the iteration count.
pub fn unused_rules(axiom: &str, rules: &BTreeMap<char, String>) -> Vec<char> {
    let mut reachable = BTreeSet::new();
    let mut stack: Vec<char> = axiom.chars().collect();
    while let Some(c) = stack.pop() {
        if reachable.insert(c)
            && let Some(rhs) = rules.get(&c)
        {
            stack.extend(rhs.chars());
        }
    }
    rules
        .keys()
        .filter(|k| !reachable.contains(k))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn koch_rules() -> BTreeMap<char, String> {
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

    fn sierpinski_rules() -> BTreeMap<char, String> {
        [('F', "F-F+F+F-F".to_string())].into()
    }

    #[test]
    fn max_safe_koch_exact_boundary() {
        // Koch: 3 × 4^n segments. At n=4 → 768; at n=5 → 3072.
        let rules = koch_rules();
        assert_eq!(max_safe_iterations("F++F++F", &rules, 768), 4);
        assert_eq!(max_safe_iterations("F++F++F", &rules, 767), 3);
    }

    #[test]
    fn max_safe_sierpinski_gpu_limit() {
        // Sierpiński: 3 × 5^n segments.
        // n=9 → 5_859_375, n=10 → 29_296_875. Limit = 16_777_216.
        let rules = sierpinski_rules();
        assert_eq!(max_safe_iterations("F-F-F", &rules, 16_777_216), 9);
    }

    #[test]
    fn max_safe_no_drawing_symbols_returns_hard_max() {
        // Axiom "A" with no F → always 0 segments, should return HARD_MAX (30).
        let rules: BTreeMap<char, String> = [('A', "AA".to_string())].into();
        assert_eq!(max_safe_iterations("A", &rules, 16_777_216), 30);
    }

    #[test]
    fn undefined_symbols_pass_through() {
        // X has no rule; F maps to FX.
        // iter 1: "F" → "FX"
        // iter 2: F→FX, X→X → "FXX"
        let rules: BTreeMap<char, String> = [('F', "FX".to_string())].into();
        let result: String = expand("F", &rules, 2).collect();
        assert_eq!(result, "FXX");
    }

    #[test]
    fn expand_interleaves_multiple_rules() {
        // Rules: A → aA, B → Bb  (each rule adds a terminal and recurses).
        // iter 0: "AB"
        // iter 1: A→aA, B→Bb  →  "aABb"
        // iter 2: a→a, A→aA, B→Bb, b→b  →  "aaABbb"
        let rules: BTreeMap<char, String> =
            [('A', "aA".to_string()), ('B', "Bb".to_string())].into();
        let result: String = expand("AB", &rules, 2).collect();
        assert_eq!(result, "aaABbb");
    }

    #[test]
    fn unused_rules_empty_when_all_reachable() {
        let rules = koch_rules();
        assert_eq!(unused_rules("F++F++F", &rules), Vec::<char>::new());
    }

    #[test]
    fn unused_rules_finds_symbol_never_referenced() {
        // 'X' has a rule but never appears in the axiom or any reachable RHS.
        let rules: BTreeMap<char, String> =
            [('F', "F-F".to_string()), ('X', "FF".to_string())].into();
        assert_eq!(unused_rules("F", &rules), vec!['X']);
    }

    #[test]
    fn unused_rules_finds_self_referencing_cycle_disconnected_from_axiom() {
        // 'X' only ever refers to itself; it's never reachable from the axiom.
        let rules: BTreeMap<char, String> =
            [('F', "F-F".to_string()), ('X', "XX".to_string())].into();
        assert_eq!(unused_rules("F", &rules), vec!['X']);
    }

    #[test]
    fn unused_rules_empty_for_no_rules() {
        let rules: BTreeMap<char, String> = BTreeMap::new();
        assert_eq!(unused_rules("F", &rules), Vec::<char>::new());
    }
}
