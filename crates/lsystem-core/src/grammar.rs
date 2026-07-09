use std::collections::{BTreeMap, BTreeSet};

use crate::alphabet::{TERMINALS_3D, TERMINALS_UNIVERSAL};

const NON_ASCII_BASE: u8 = 128;
const MAX_NON_ASCII_SYMBOLS: usize = (u8::MAX - NON_ASCII_BASE + 1) as usize;

struct Frame {
    pos: u32,
    end: u32,
    depth: u32,
}

/// Bytes with a turtle effect. Expansion in effects-only mode drops every
/// other byte, so the generation path never sees rewrite-only symbols.
const EFFECTS: [bool; 256] = {
    let mut table = [false; 256];
    let universal = TERMINALS_UNIVERSAL.as_bytes();
    let three_d = TERMINALS_3D.as_bytes();
    let mut i = 0;
    while i < universal.len() {
        table[universal[i] as usize] = true;
        i += 1;
    }
    let mut i = 0;
    while i < three_d.len() {
        table[three_d[i] as usize] = true;
        i += 1;
    }
    table
};

/// Half-open arena spans for one expandable string: the string itself and
/// its effect-filtered copy. Storing both behind one `Option` makes it
/// impossible for a symbol to have one span without the other.
#[derive(Clone, Copy)]
struct RuleSpans {
    full: (u32, u32),
    effects: (u32, u32),
}

/// The compiled form of a grammar: axiom and rule RHS spans in one shared u8
/// arena, with O(1) rule lookup per symbol byte. This is the single place
/// where `char`-domain axiom and rules convert to the byte-domain arena;
/// every expansion flavor iterates over one compiled value. Rules that can
/// never be reached from the axiom are dropped during compilation, so the
/// rule table holds exactly the rules expansion can apply.
///
/// Each expandable string is stored twice ([`RuleSpans`]): the full form and
/// an effect-filtered copy. Effects-only expansion streams terminal (depth-0)
/// spans from the filtered copies and drops pass-through bytes emitted at
/// depth > 0 via [`EFFECTS`].
pub(crate) struct CompiledGrammar {
    arena: Vec<u8>,
    rules: Box<[Option<RuleSpans>; 256]>,
    axiom: RuleSpans,
}

impl CompiledGrammar {
    pub(crate) fn compile(axiom: &str, rules: &BTreeMap<char, String>) -> Self {
        let reachable = reachable_symbols(axiom, rules);

        let mut non_ascii: Vec<(char, u8)> = Vec::new();
        let mut next_id = NON_ASCII_BASE;
        let mut arena: Vec<u8> = Vec::new();

        // Appends the effect-filtered copy of `arena[start..]` and returns
        // both spans.
        fn finish_spans(arena: &mut Vec<u8>, start: u32) -> RuleSpans {
            let full = (start, arena.len() as u32);
            for pos in full.0 as usize..full.1 as usize {
                let byte = arena[pos];
                if EFFECTS[byte as usize] {
                    arena.push(byte);
                }
            }
            RuleSpans {
                full,
                effects: (full.1, arena.len() as u32),
            }
        }

        // The axiom occupies the arena prefix; each rule RHS is appended
        // afterward. Every string is immediately followed by its filtered
        // copy, and the spans record the half-open ranges of both.
        for c in axiom.chars() {
            arena.push(char_to_id(c, &mut non_ascii, &mut next_id));
        }
        let axiom_spans = finish_spans(&mut arena, 0);

        let mut rule_table: Box<[Option<RuleSpans>; 256]> = Box::new([None; 256]);
        for (k, v) in rules.iter().filter(|(k, _)| reachable.contains(k)) {
            let key_id = char_to_id(*k, &mut non_ascii, &mut next_id) as usize;
            let rhs_start = arena.len() as u32;
            for c in v.chars() {
                arena.push(char_to_id(c, &mut non_ascii, &mut next_id));
            }
            rule_table[key_id] = Some(finish_spans(&mut arena, rhs_start));
        }

        Self {
            arena,
            rules: rule_table,
            axiom: axiom_spans,
        }
    }

    /// Full expansion: every expanded symbol is yielded.
    #[cfg(test)]
    pub(crate) fn expand(self, iterations: u32) -> ExpandIter {
        let (pos, end) = self.axiom.full;
        ExpandIter {
            stack: vec![Frame {
                pos,
                end,
                depth: iterations,
            }],
            effects_only: false,
            grammar: self,
            terminal_pos: 0,
            terminal_end: 0,
        }
    }

    /// Effects-only expansion: symbols with no turtle effect are stripped.
    pub(crate) fn expand_effects(self, iterations: u32) -> ExpandIter {
        // At zero iterations the axiom frame itself streams terminally, so an
        // effects-only expansion starts on the filtered copy.
        let (pos, end) = if iterations == 0 {
            self.axiom.effects
        } else {
            self.axiom.full
        };
        ExpandIter {
            stack: vec![Frame {
                pos,
                end,
                depth: iterations,
            }],
            effects_only: true,
            grammar: self,
            terminal_pos: 0,
            terminal_end: 0,
        }
    }
}

pub(crate) struct ExpandIter {
    grammar: CompiledGrammar,
    effects_only: bool,
    stack: Vec<Frame>,
    terminal_pos: u32,
    terminal_end: u32,
}

impl Iterator for ExpandIter {
    type Item = u8;

    fn next(&mut self) -> Option<u8> {
        if self.terminal_pos < self.terminal_end {
            let byte = self.grammar.arena[self.terminal_pos as usize];
            self.terminal_pos += 1;
            return Some(byte);
        }

        loop {
            let (byte, depth) = {
                let frame = self.stack.last_mut()?;
                if frame.pos == frame.end {
                    self.stack.pop();
                    continue;
                }
                if frame.depth == 0 {
                    let pos = frame.pos;
                    self.terminal_pos = pos + 1;
                    self.terminal_end = frame.end;
                    self.stack.pop();
                    return Some(self.grammar.arena[pos as usize]);
                }
                let b = self.grammar.arena[frame.pos as usize];
                frame.pos += 1;
                (b, frame.depth)
            };
            if let Some(spans) = self.grammar.rules[byte as usize] {
                // In effects-only mode, children entering depth 0 stream
                // their span terminally, so they take the filtered copy.
                let (pos, end) = if depth == 1 && self.effects_only {
                    spans.effects
                } else {
                    spans.full
                };
                self.stack.push(Frame {
                    pos,
                    end,
                    depth: depth - 1,
                });
                continue;
            }
            if !self.effects_only || EFFECTS[byte as usize] {
                return Some(byte);
            }
        }
    }

    fn fold<B, F>(mut self, init: B, mut f: F) -> B
    where
        Self: Sized,
        F: FnMut(B, Self::Item) -> B,
    {
        let mut acc = self.grammar.arena[self.terminal_pos as usize..self.terminal_end as usize]
            .iter()
            .copied()
            .fold(init, &mut f);

        loop {
            let Some(frame) = self.stack.last_mut() else {
                return acc;
            };
            if frame.pos == frame.end {
                self.stack.pop();
                continue;
            }
            if frame.depth == 0 {
                let start = frame.pos as usize;
                let end = frame.end as usize;
                self.stack.pop();
                acc = self.grammar.arena[start..end]
                    .iter()
                    .copied()
                    .fold(acc, &mut f);
                continue;
            }

            let byte = self.grammar.arena[frame.pos as usize];
            frame.pos += 1;
            let depth = frame.depth - 1;
            if let Some(spans) = self.grammar.rules[byte as usize] {
                // Mirrors the span selection in `next`; `depth` here is the
                // child's depth (already decremented), so `depth == 0`
                // corresponds to `depth == 1` there.
                let (pos, end) = if depth == 0 && self.effects_only {
                    spans.effects
                } else {
                    spans.full
                };
                self.stack.push(Frame { pos, end, depth });
            } else if !self.effects_only || EFFECTS[byte as usize] {
                acc = f(acc, byte);
            }
        }
    }
}

/// Maps ASCII symbols to their byte values and assigns non-ASCII symbols
/// sequential IDs from [`NON_ASCII_BASE`] through `u8::MAX`.
fn char_to_id(c: char, non_ascii: &mut Vec<(char, u8)>, next_id: &mut u8) -> u8 {
    if c.is_ascii() {
        c as u8
    } else if let Some(&(_, id)) = non_ascii.iter().find(|(ch, _)| *ch == c) {
        id
    } else {
        assert!(
            non_ascii.len() < MAX_NON_ASCII_SYMBOLS,
            "too many distinct non-ASCII symbols (max {MAX_NON_ASCII_SYMBOLS})"
        );
        let id = *next_id;
        *next_id = next_id.wrapping_add(1);
        non_ascii.push((c, id));
        id
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

/// Returns every symbol that can appear during expansion of `axiom`: the
/// axiom's own symbols plus the RHS symbols of every reachable rule.
fn reachable_symbols(axiom: &str, rules: &BTreeMap<char, String>) -> BTreeSet<char> {
    let mut reachable = BTreeSet::new();
    let mut stack: Vec<char> = axiom.chars().collect();
    while let Some(c) = stack.pop() {
        if reachable.insert(c)
            && let Some(rhs) = rules.get(&c)
        {
            stack.extend(rhs.chars());
        }
    }
    reachable
}

/// Returns the rule symbols that are never reached during expansion of `axiom`.
///
/// A rule is reachable if its symbol appears in `axiom`, or in the RHS of any
/// other reachable rule. Unreachable rules are dead: expansion never applies
/// them, no matter the iteration count.
pub fn unused_rules(axiom: &str, rules: &BTreeMap<char, String>) -> Vec<char> {
    let reachable = reachable_symbols(axiom, rules);
    rules
        .keys()
        .filter(|k| !reachable.contains(k))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::collect_with_next;

    fn koch_rules() -> BTreeMap<char, String> {
        [('F', "F-F++F-F".to_string())].into()
    }

    #[test]
    fn zero_iterations_returns_axiom() {
        let result: Vec<u8> = CompiledGrammar::compile("F++F++F", &koch_rules())
            .expand(0)
            .collect();
        assert_eq!(result, b"F++F++F");
    }

    #[test]
    fn depth_zero_frame_streams_remaining_terminal_span_after_pop() {
        let mut symbols = CompiledGrammar::compile("ABC", &BTreeMap::new()).expand(0);

        assert_eq!(symbols.next(), Some(b'A'));
        assert!(symbols.stack.is_empty());
        assert_eq!(symbols.collect::<Vec<_>>(), b"BC");
    }

    #[test]
    fn empty_axiom_returns_no_symbols() {
        let result: Vec<u8> = CompiledGrammar::compile("", &koch_rules())
            .expand(1)
            .collect();
        assert!(result.is_empty());
    }

    #[test]
    fn empty_rule_rhs_resumes_parent_frame() {
        let rules: BTreeMap<char, String> = [('F', String::new())].into();
        let result: Vec<u8> = CompiledGrammar::compile("FA", &rules).expand(1).collect();
        assert_eq!(result, b"A");
    }

    #[test]
    fn one_iteration_expands_f() {
        let result: Vec<u8> = CompiledGrammar::compile("F++F++F", &koch_rules())
            .expand(1)
            .collect();
        // Each F → F-F++F-F; the ++ between each F are carried through.
        assert_eq!(result, b"F-F++F-F++F-F++F-F++F-F++F-F");
    }

    #[test]
    fn effect_expansion_matches_full_expansion_with_ignored_symbols_removed() {
        let rules: BTreeMap<char, String> = [
            ('X', "F+Y&ä".to_string()),
            ('Y', "F[-X]^f".to_string()),
            ('ä', "F/Z".to_string()),
        ]
        .into();
        let is_effect = |byte: &u8| {
            TERMINALS_UNIVERSAL.as_bytes().contains(byte) || TERMINALS_3D.as_bytes().contains(byte)
        };
        for iterations in 0..=3 {
            let expected: Vec<_> = CompiledGrammar::compile("X|Y", &rules)
                .expand(iterations)
                .filter(is_effect)
                .collect();
            let actual: Vec<_> = CompiledGrammar::compile("X|Y", &rules)
                .expand_effects(iterations)
                .collect();

            assert_eq!(actual, expected, "iterations={iterations}");
        }
    }

    #[test]
    fn fold_matches_repeated_next_after_partial_terminal_consumption() {
        let rules: BTreeMap<char, String> = [('F', "F+F-X".to_string())].into();
        let mut folded = CompiledGrammar::compile("F", &rules).expand(3);
        let first = folded.next().unwrap();
        let folded = folded.fold(vec![first], |mut bytes, byte| {
            bytes.push(byte);
            bytes
        });

        assert_eq!(
            folded,
            collect_with_next(CompiledGrammar::compile("F", &rules).expand(3))
        );
    }

    #[test]
    fn f_count_grows_as_power_of_four() {
        // Koch snowflake: 3 F's at iter 0, multiplied by 4 each iteration.
        let rules = koch_rules();
        for iter in 0..=4u32 {
            let f_count = CompiledGrammar::compile("F++F++F", &rules)
                .expand(iter)
                .filter(|&b| b == b'F')
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
        let result: Vec<u8> = CompiledGrammar::compile("F", &rules).expand(2).collect();
        assert_eq!(result, b"FXX");
    }

    #[test]
    fn expand_interleaves_multiple_rules() {
        // Rules: A → aA, B → Bb  (each rule adds a terminal and recurses).
        // iter 0: "AB"
        // iter 1: A→aA, B→Bb  →  "aABb"
        // iter 2: a→a, A→aA, B→Bb, b→b  →  "aaABbb"
        let rules: BTreeMap<char, String> =
            [('A', "aA".to_string()), ('B', "Bb".to_string())].into();
        let result: Vec<u8> = CompiledGrammar::compile("AB", &rules).expand(2).collect();
        assert_eq!(result, b"aaABbb");
    }

    #[test]
    fn non_ascii_rule_key_is_applied() {
        let rules: BTreeMap<char, String> = [('ä', "FFF".to_string())].into();
        let result: Vec<u8> = CompiledGrammar::compile("ä", &rules).expand(1).collect();
        assert_eq!(result, b"FFF");
    }

    #[test]
    fn non_ascii_without_rule_passes_through() {
        // 'ä' is assigned ID 128; with no rule it passes through unchanged.
        let result: Vec<u8> = CompiledGrammar::compile("ä", &BTreeMap::new())
            .expand(1)
            .collect();
        assert_eq!(result, [128u8]);
    }

    #[test]
    fn two_distinct_non_ascii_chars_get_distinct_ids() {
        // If 'ä' and 'ö' both got ID 128, the second rule would overwrite the
        // first in the rule table and "äö" would expand to "FFFF" instead of "FFF".
        let rules: BTreeMap<char, String> =
            [('ä', "F".to_string()), ('ö', "FF".to_string())].into();
        let result: Vec<u8> = CompiledGrammar::compile("äö", &rules).expand(1).collect();
        assert_eq!(result, b"FFF");
    }

    #[test]
    fn non_ascii_in_rule_rhs_passes_through() {
        // 'ä' in the RHS is encoded in the arena (ID 128) and passes through
        // when the iterator encounters it with no rule of its own.
        let rules: BTreeMap<char, String> = [('F', "äF".to_string())].into();
        let result: Vec<u8> = CompiledGrammar::compile("F", &rules).expand(1).collect();
        assert_eq!(result, [128u8, b'F']);
    }

    #[test]
    fn non_ascii_non_terminal_in_rhs_expands_correctly() {
        // 'ä' appears in both a RHS and has its own rule.
        // axiom "ä", rule ä→"äF":
        //   iter 0: [128]
        //   iter 1: ä→äF → [128, b'F']  (ä at depth 0 passes through)
        //   iter 2: ä→äF, depth of inner ä is 0 → [128, b'F', b'F']
        let rules: BTreeMap<char, String> = [('ä', "äF".to_string())].into();
        let result: Vec<u8> = CompiledGrammar::compile("ä", &rules).expand(2).collect();
        assert_eq!(result, [128u8, b'F', b'F']);
    }

    #[test]
    fn mixed_ascii_and_non_ascii_axiom_with_rules() {
        // "Fä": F has no rule (passes through), ä → "FF".
        // iter 1: F passes through, ä→FF → "FFF"
        let rules: BTreeMap<char, String> = [('ä', "FF".to_string())].into();
        let result: Vec<u8> = CompiledGrammar::compile("Fä", &rules).expand(1).collect();
        assert_eq!(result, b"FFF");
    }

    #[test]
    fn supports_128_distinct_non_ascii_symbols() {
        // U+0080..=U+00FF are exactly 128 distinct non-ASCII chars (IDs 128..=255).
        // None of them have rules, so they all pass through unchanged.
        let axiom: String = ('\u{0080}'..='\u{00FF}').collect();
        let result: Vec<u8> = CompiledGrammar::compile(&axiom, &BTreeMap::new())
            .expand(0)
            .collect();
        assert_eq!(result.len(), 128);
        assert_eq!(result, (128u8..=255u8).collect::<Vec<_>>());
    }

    #[test]
    #[should_panic(expected = "too many distinct non-ASCII symbols (max 128)")]
    fn panics_on_129th_distinct_non_ascii_symbol() {
        let axiom: String = ('\u{0080}'..='\u{0100}').collect();
        let _ = CompiledGrammar::compile(&axiom, &BTreeMap::new())
            .expand(0)
            .collect::<Vec<_>>();
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

    #[test]
    fn compile_drops_rule_never_referenced() {
        // 'X' has a rule but never appears in the axiom or any reachable RHS.
        let rules: BTreeMap<char, String> =
            [('F', "F-F".to_string()), ('X', "FF".to_string())].into();
        let grammar = CompiledGrammar::compile("F", &rules);

        assert!(grammar.rules[b'X' as usize].is_none());
        assert!(grammar.rules[b'F' as usize].is_some());
    }

    #[test]
    fn compile_drops_self_referencing_cycle_disconnected_from_axiom() {
        // 'X' only ever refers to itself; it's never reachable from the axiom.
        let rules: BTreeMap<char, String> =
            [('F', "F-F".to_string()), ('X', "XX".to_string())].into();
        let grammar = CompiledGrammar::compile("F", &rules);

        assert!(grammar.rules[b'X' as usize].is_none());
    }

    #[test]
    fn compile_arena_matches_compiling_without_the_dropped_rule() {
        // Dropping an unreachable rule must not change the arena bytes or
        // spans produced for the rules that remain.
        let with_dead_rule: BTreeMap<char, String> =
            [('F', "F-F".to_string()), ('X', "FF".to_string())].into();
        let without_dead_rule: BTreeMap<char, String> = [('F', "F-F".to_string())].into();

        let a = CompiledGrammar::compile("F", &with_dead_rule);
        let b = CompiledGrammar::compile("F", &without_dead_rule);

        assert_eq!(a.arena, b.arena);
    }

    #[test]
    fn compile_drops_dead_rule_with_a_unique_non_ascii_key() {
        // A dropped rule's key must never consume a non-ASCII ID: it is
        // filtered out before `char_to_id` sees it.
        let with_dead_rule: BTreeMap<char, String> =
            [('F', "F-F".to_string()), ('ä', "FF".to_string())].into();
        let without_dead_rule: BTreeMap<char, String> = [('F', "F-F".to_string())].into();

        let a = CompiledGrammar::compile("F", &with_dead_rule);
        let b = CompiledGrammar::compile("F", &without_dead_rule);

        assert_eq!(a.arena, b.arena);
    }
}
