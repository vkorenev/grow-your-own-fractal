use crate::app::{ConfigContext, update_clean_config};
use crate::ui::DebouncedTextInput;
use leptos::prelude::*;
use lsystem_app_model::{
    EditorGenerationConfig, GrammarDraftError, ValidGrammar, validate_grammar_draft,
};
use lsystem_core::Dimensions;

/// One rule row of the grammar editor. The text signals are reference-counted
/// rather than arena-allocated: rows are created from whichever reactive owner
/// is current when the draft is resynced or a rule is added (often an event
/// handler inside a re-rendering view), and an arena signal would be disposed
/// together with that owner while the row is still displayed. An `Arc` signal
/// lives exactly as long as the row is referenced.
#[derive(Clone)]
pub(crate) struct GrammarRow {
    id: u32,
    symbol: ArcRwSignal<String>,
    rhs: ArcRwSignal<String>,
}

/// The grammar editor's draft: the dimensions, axiom and rule rows being
/// edited, and their validation against the same checks applying performs.
/// Owned by `App` (provided via `ConfigContext`) because entry switches, a
/// successful raw TOML apply and a bundled-preset reset resync it from
/// there, and the raw TOML panel reads `held`.
///
/// The text inputs only commit settled edits (see `DebouncedTextInput`), so
/// every change to these signals is applied right away when valid. The
/// draft applies itself (see `new`), so this holds whether or not the grammar
/// panel is mounted.
#[derive(Clone, Copy)]
pub(crate) struct GrammarDraft {
    dimensions: RwSignal<Dimensions>,
    axiom: RwSignal<String>,
    rows: RwSignal<Vec<GrammarRow>>,
    row_counter: StoredValue<u32>,
    applied: Memo<EditorGenerationConfig>,
    /// The normalized grammar the draft would apply, or why it can't be.
    /// The single source for errors, `held`, and auto-apply.
    validated: Memo<Result<ValidGrammar, Vec<GrammarDraftError>>>,
    /// Whether the draft is invalid and so held back from being applied.
    /// Valid edits are applied as soon as they're committed, so this is the
    /// only case in which a raw TOML apply would discard grammar changes.
    pub(crate) held: Memo<bool>,
    /// Letters used in the axiom or as rule symbols, for the symbol picker.
    /// A memo so the picker only re-renders when the set of letters changes.
    symbols: Memo<Vec<char>>,
}

impl GrammarDraft {
    /// Creates a draft of the applied grammar that auto-applies every
    /// committed change that alters what would be applied. Resyncs (entry
    /// switch, TOML apply, reset) also change `validated`, but to the applied
    /// grammar, so `apply` no-ops. Unexpected apply failures go to `error`.
    pub(crate) fn new(
        applied: Memo<EditorGenerationConfig>,
        config_workspace: RwSignal<lsystem_app_model::ConfigWorkspace>,
        error: RwSignal<Option<String>>,
    ) -> Self {
        let row_counter = StoredValue::new(0u32);
        let (dimensions, axiom, rows) = applied.with_untracked(|generation| {
            (
                generation.dimensions,
                generation.axiom.clone(),
                rows_from_rules(&generation.rules, row_counter),
            )
        });
        let dimensions = RwSignal::new(dimensions);
        let axiom = RwSignal::new(axiom);
        let rows = RwSignal::new(rows);
        let validated = Memo::new(move |_| {
            let rows = rows.with(|rows| {
                rows.iter()
                    .map(|row| (row.symbol.get(), row.rhs.get()))
                    .collect::<Vec<_>>()
            });
            axiom.with(|axiom| validate_grammar_draft(dimensions.get(), axiom, &rows))
        });
        let held = Memo::new(move |_| validated.with(Result::is_err));
        let symbols = Memo::new(move |_| {
            let mut set = std::collections::BTreeSet::new();
            axiom.with(|a| set.extend(a.chars().filter(|c| c.is_alphabetic())));
            rows.with(|rows| {
                for row in rows {
                    row.symbol
                        .with(|s| set.extend(s.chars().filter(|c| c.is_alphabetic())));
                }
            });
            set.into_iter().collect()
        });
        let draft = Self {
            dimensions,
            axiom,
            rows,
            row_counter,
            applied,
            validated,
            held,
            symbols,
        };
        Effect::watch(
            move || validated.track(),
            move |_, _, _: Option<()>| draft.apply(config_workspace, error),
            false,
        );
        draft
    }

    /// Resets the draft to the currently applied grammar.
    pub(crate) fn sync(&self) {
        let generation = self.applied.get_untracked();
        self.dimensions.set(generation.dimensions);
        self.axiom.set(generation.axiom);
        self.rows
            .set(rows_from_rules(&generation.rules, self.row_counter));
    }

    fn add_row(&self) {
        let row = new_row(self.row_counter, String::new(), String::new());
        self.rows.update(|rows| rows.push(row));
    }

    fn remove_row(&self, id: u32) {
        self.rows.update(|rows| rows.retain(|row| row.id != id));
    }

    /// Applies the draft if it is valid and differs from the applied
    /// grammar. A failure here is unexpected, since `validated` runs the same
    /// checks applying does, so it goes to the grammar panel's `error`.
    fn apply(
        &self,
        config_workspace: RwSignal<lsystem_app_model::ConfigWorkspace>,
        error: RwSignal<Option<String>>,
    ) {
        let grammar = self.validated.with_untracked(|validated| {
            validated
                .as_ref()
                .ok()
                .filter(|grammar| {
                    !self
                        .applied
                        .with_untracked(|applied| grammar.matches(applied))
                })
                .cloned()
        });
        if let Some(grammar) = grammar {
            update_clean_config(
                config_workspace,
                error,
                "grammar auto-apply",
                move |clean| clean.set_grammar(&grammar),
            );
        }
    }
}

fn new_row(counter: StoredValue<u32>, symbol: String, rhs: String) -> GrammarRow {
    let id = counter.get_value();
    counter.set_value(id + 1);
    GrammarRow {
        id,
        symbol: ArcRwSignal::new(symbol),
        rhs: ArcRwSignal::new(rhs),
    }
}

fn rows_from_rules(
    rules: &std::collections::BTreeMap<char, String>,
    counter: StoredValue<u32>,
) -> Vec<GrammarRow> {
    rules
        .iter()
        .map(|(symbol, rhs)| new_row(counter, symbol.to_string(), rhs.clone()))
        .collect()
}

/// User-facing text for a draft problem. The 3D-symbol message is the same
/// whether the user switched to 2D or typed a 3D symbol while in 2D.
fn grammar_draft_error_message(error: &GrammarDraftError) -> String {
    match error {
        GrammarDraftError::MissingRuleSymbol => {
            "Each rule must have a symbol. Remove or complete rows without one.".to_string()
        }
        GrammarDraftError::DuplicateRuleSymbol(c) => {
            format!("Duplicate rule symbol '{c}'. Each symbol may appear only once.")
        }
        GrammarDraftError::ThreeDSymbolsIn2D => {
            "3D-only symbols (& ^ / \\) can't be used in 2D. Remove them or switch to 3D."
                .to_string()
        }
        GrammarDraftError::Invalid(error) => error.to_string(),
    }
}

#[component]
pub(crate) fn GrammarPanel() -> impl IntoView {
    let ConfigContext {
        config_workspace,
        grammar_error,
        generation_config,
        is_dirty,
        unused_rule_symbols,
        iterations,
        max_iterations,
        angle,
        grammar,
        ..
    } = expect_context();

    let draft_errors = move || {
        grammar.validated.with(|validated| {
            validated
                .as_ref()
                .err()
                .map(|errors| errors.iter().map(grammar_draft_error_message).collect())
                .unwrap_or_else(Vec::new)
        })
    };

    view! {
        <crate::ui::Disclosure title="L-System" badge=grammar.held>
            <crate::ui::DirtyDraftWarning is_dirty=is_dirty />
            <div style="display:flex;flex-direction:column;gap:5px">
                <span class="section-label">"Dimensions"</span>
                // Part of the grammar draft: a switch to 2D while 3D-only
                // symbols remain is held, and explained, like any other
                // invalid draft.
                <crate::ui::SegmentedToggle
                    options=vec![(Dimensions::TwoD, "2D"), (Dimensions::ThreeD, "3D")]
                    selected=grammar.dimensions
                    on_change=move |key| grammar.dimensions.set(key)
                />
            </div>

            <hr class="section-divider" />

            <span class="section-label">"Grammar"</span>
            <table class="grammar-table">
                <tbody>
                    <tr>
                        <td class="g-symbol">
                            <span class="grammar-axiom-label">"axiom"</span>
                        </td>
                        <td>
                            <DebouncedTextInput
                                class="grammar-rhs"
                                value=grammar.axiom
                                on_commit=move |text| grammar.axiom.set(text)
                            />
                        </td>
                        <td class="g-delete"></td>
                    </tr>
                    <For
                        each=move || grammar.rows.get()
                        key=|row| row.id
                        children=move |row: GrammarRow| {
                            // Copyable handles for the view closures, owned by
                            // this row's view and disposed with it.
                            let id = row.id;
                            let symbol = RwSignal::from(row.symbol);
                            let rhs = RwSignal::from(row.rhs);
                            view! {
                                <tr>
                                    <td class="g-symbol">
                                        <DebouncedTextInput
                                            class="grammar-combo"
                                            list="grammar-symbols"
                                            maxlength=1
                                            value=symbol
                                            on_commit=move |text| symbol.set(text)
                                        />
                                    </td>
                                    <td>
                                        <DebouncedTextInput
                                            class="grammar-rhs"
                                            value=rhs
                                            on_commit=move |text| rhs.set(text)
                                        />
                                    </td>
                                    <td class="g-delete">
                                        <button
                                            type="button"
                                            class="grammar-delete-btn"
                                            on:click=move |_| grammar.remove_row(id)
                                        >"×"</button>
                                    </td>
                                </tr>
                            }
                        }
                    />
                </tbody>
            </table>
            <datalist id="grammar-symbols">
                {move || {
                    grammar
                        .symbols
                        .get()
                        .into_iter()
                        .map(|c| {
                            let s = c.to_string();
                            view! { <option value=s /> }
                        })
                        .collect_view()
                }}
            </datalist>

            <div class="btn-row">
                <button type="button" on:click=move |_| grammar.add_row()>"Add rule"</button>
            </div>
            {move || draft_errors().into_iter().map(|msg| view! {
                <span class="inline-status error">{msg}</span>
            }).collect_view()}
            {move || grammar_error.get().map(|msg| view! {
                <span class="inline-status error">{msg}</span>
            })}
            {move || {
                let unused = unused_rule_symbols.get();
                (!unused.is_empty()).then(|| {
                    let symbols = unused.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", ");
                    view! {
                        <span class="inline-status warning">
                            {format!("Unused rules: {symbols} (never used during expansion)")}
                        </span>
                    }
                })
            }}

            <hr class="section-divider" />

            <span class="section-label">"Parameters"</span>
            <div class="spinner-row">
                <span class="spinner-label">"Angle (°)"</span>
                <crate::ui::Spinner
                    value=angle
                    step=0.5
                    min=1.0_f32
                    max=180.0_f32
                    decimals=1
                    on_commit=move |v: f32| {
                        update_clean_config(
                            config_workspace, grammar_error,"angle",
                            move |clean| clean.set_angle(v),
                        );
                    }
                />
            </div>

            <div class="spinner-row">
                <span class="spinner-label">"Initial heading (°)"</span>
                <crate::ui::Spinner
                    value=Signal::derive(move || generation_config.with(|g| g.initial_heading))
                    step=1.0
                    decimals=1
                    on_commit=move |v: f32| {
                        update_clean_config(
                            config_workspace, grammar_error,"initial_heading",
                            move |clean| clean.set_initial_heading(v),
                        );
                    }
                />
            </div>

            <div class="spinner-row">
                <span class="spinner-label">{move || format!("Iterations (max {})", max_iterations.get())}</span>
                <crate::ui::Spinner
                    value=iterations
                    step=1.0
                    max=max_iterations
                    on_commit=move |v: u16| {
                        update_clean_config(
                            config_workspace, grammar_error,"iterations",
                            move |clean| clean.set_iterations(v),
                        );
                    }
                />
            </div>
        </crate::ui::Disclosure>
    }
}
