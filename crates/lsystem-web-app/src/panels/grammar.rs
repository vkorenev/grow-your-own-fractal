use crate::app::{ConfigContext, GrammarDraft, update_clean_config};
use leptos::prelude::*;
use lsystem_core::Dimensions;

#[derive(Clone, Copy)]
pub(crate) struct GrammarRow {
    pub(crate) id: u32,
    pub(crate) symbol: RwSignal<String>,
    pub(crate) rhs: RwSignal<String>,
}

pub(crate) fn next_grammar_row_id(counter: StoredValue<u32>) -> u32 {
    let id = counter.get_value();
    counter.set_value(id + 1);
    id
}

pub(crate) fn rows_from_rules(
    rules: &std::collections::BTreeMap<char, String>,
    counter: StoredValue<u32>,
) -> Vec<GrammarRow> {
    rules
        .iter()
        .map(|(k, v)| GrammarRow {
            id: next_grammar_row_id(counter),
            symbol: RwSignal::new(k.to_string()),
            rhs: RwSignal::new(v.clone()),
        })
        .collect()
}

#[component]
pub(crate) fn GrammarPanel() -> impl IntoView {
    let ConfigContext {
        config_workspace,
        grammar_error,
        generation_config,
        dimensions,
        is_3d,
        is_dirty,
        unused_rule_symbols,
        iterations,
        max_iterations,
        angle,
        grammar:
            GrammarDraft {
                axiom: grammar_axiom,
                rows: grammar_rows,
                row_counter: grammar_row_counter,
                is_dirty: grammar_is_dirty,
                has_3d_symbols: grammar_has_3d_symbols,
                symbols: grammar_symbols,
                sync: sync_grammar_editor,
            },
        ..
    } = expect_context();

    let dirty_tooltip = move || {
        if is_dirty.get() {
            "Apply or Revert TOML changes first"
        } else {
            ""
        }
    };

    let do_apply_grammar = move || {
        let axiom = grammar_axiom.get_untracked();
        let rows = grammar_rows.get_untracked();
        let mut seen = std::collections::HashSet::new();
        let mut rules: Vec<(char, String)> = Vec::with_capacity(rows.len());
        for row in rows {
            let k = row.symbol.get_untracked();
            let v = row.rhs.get_untracked();
            let Some(c) = k.chars().next() else {
                grammar_error.set(Some(
                    "Each rule must have a symbol. Remove or complete empty rows before applying."
                        .to_string(),
                ));
                return;
            };
            if !seen.insert(c) {
                grammar_error.set(Some(format!(
                    "Duplicate rule symbol '{c}'. Each symbol may appear only once."
                )));
                return;
            }
            rules.push((c, v));
        }
        if update_clean_config(
            config_workspace,
            grammar_error,
            "grammar apply",
            move |clean| clean.set_grammar(&axiom, &rules),
        ) {
            sync_grammar_editor.run(());
        }
    };

    // Grammar Apply is enabled only when grammar has changes, the entry is clean, and
    // no 3D-only symbols are present in 2D mode.
    let grammar_can_apply = move || {
        grammar_is_dirty.get() && (is_3d.get() || !grammar_has_3d_symbols.get()) && !is_dirty.get()
    };

    let try_apply_grammar = move || {
        // The Apply button is disabled when grammar has 3D symbols in 2D mode;
        // this guard is a defensive fallback.
        if grammar_has_3d_symbols.get_untracked() && !is_3d.get_untracked() {
            return;
        }
        do_apply_grammar();
    };

    let try_set_dimensions = move |next: Dimensions| {
        // Defensive: "2D" button is disabled when grammar_has_3d_symbols; guard against bypass.
        if next == Dimensions::TwoD && grammar_has_3d_symbols.get() {
            return;
        }
        update_clean_config(
            config_workspace,
            grammar_error,
            "set dimensions",
            move |clean| clean.set_dimensions(next),
        );
    };

    view! {
        <crate::ui::Disclosure title="L-System" badge=grammar_is_dirty>
            <div style="display:flex;flex-direction:column;gap:5px">
                <span class="section-label">"Dimensions"</span>
                <div title=dirty_tooltip>
                <crate::ui::SegmentedToggle
                    options=vec![(Dimensions::TwoD, "2D"), (Dimensions::ThreeD, "3D")]
                    selected=dimensions
                    on_change=move |key| try_set_dimensions(key)
                    disabled=is_dirty
                    disabled_keys=Signal::derive(move || {
                        if grammar_has_3d_symbols.get() { vec![Dimensions::TwoD] } else { vec![] }
                    })
                />
                </div>
                <Show when=move || grammar_has_3d_symbols.get()>
                    <span class="inline-status warning">
                        "Grammar contains 3D-only symbols (& ^ / \\) — 2D mode is unavailable"
                    </span>
                </Show>
            </div>

            <hr class="section-divider" />

            <span class="section-label">"Grammar"</span>
            <div title=dirty_tooltip>
            <table class="grammar-table">
                <tbody>
                    <tr>
                        <td class="g-symbol">
                            <span class="grammar-axiom-label">"axiom"</span>
                        </td>
                        <td>
                            <input
                                type="text"
                                class="grammar-rhs"
                                prop:value=move || grammar_axiom.get()
                                disabled=is_dirty
                                on:input:target=move |ev| grammar_axiom.set(ev.target().value())
                            />
                        </td>
                        <td class="g-delete"></td>
                    </tr>
                    <For
                        each=move || grammar_rows.get()
                        key=|row| row.id
                        children=move |row: GrammarRow| {
                            view! {
                                <tr>
                                    <td class="g-symbol">
                                        <input
                                            type="text"
                                            class="grammar-combo"
                                            list="grammar-symbols"
                                            maxlength="1"
                                            prop:value=move || row.symbol.get()
                                            disabled=is_dirty
                                            on:input:target=move |ev| row.symbol.set(ev.target().value())
                                        />
                                    </td>
                                    <td>
                                        <input
                                            type="text"
                                            class="grammar-rhs"
                                            prop:value=move || row.rhs.get()
                                            disabled=is_dirty
                                            on:input:target=move |ev| row.rhs.set(ev.target().value())
                                        />
                                    </td>
                                    <td class="g-delete">
                                        <button
                                            type="button"
                                            class="grammar-delete-btn"
                                            disabled=is_dirty
                                            on:click=move |_| {
                                                grammar_rows.update(|rows| rows.retain(|r| r.id != row.id));
                                            }
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
                    grammar_symbols
                        .get()
                        .into_iter()
                        .map(|c| {
                            let s = c.to_string();
                            view! { <option value=s /> }
                        })
                        .collect_view()
                }}
            </datalist>
            </div>

            <div class="btn-row">
                <div title=dirty_tooltip>
                    <button
                        type="button"
                        disabled=is_dirty
                        on:click=move |_| {
                            grammar_rows.update(|rows| {
                                rows.push(GrammarRow {
                                    id: next_grammar_row_id(grammar_row_counter),
                                    symbol: RwSignal::new(String::new()),
                                    rhs: RwSignal::new(String::new()),
                                });
                            });
                        }
                    >"Add rule"</button>
                </div>
                <div title=move || {
                    if is_dirty.get() { "Apply or Revert TOML changes first" }
                    else if grammar_has_3d_symbols.get() && !is_3d.get() {
                        "Contains 3D-only symbols — switch to 3D mode in Dimensions first"
                    } else { "" }
                }>
                    <button
                        type="button"
                        disabled=move || !grammar_can_apply()
                        on:click=move |_| try_apply_grammar()
                    >"Apply"</button>
                </div>
                <button
                    type="button"
                    disabled=move || !grammar_is_dirty.get()
                    on:click=move |_| sync_grammar_editor.run(())
                >"Revert"</button>
            </div>
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
            <div class="spinner-row" title=dirty_tooltip>
                <span class="spinner-label">"Angle (°)"</span>
                <crate::ui::Spinner
                    value=angle
                    step=0.5
                    min=1.0_f32
                    max=180.0_f32
                    decimals=1
                    disabled=is_dirty
                    on_commit=move |v: f32| {
                        update_clean_config(
                            config_workspace, grammar_error,"angle",
                            move |clean| clean.set_angle(v),
                        );
                    }
                />
            </div>

            <div class="spinner-row" title=dirty_tooltip>
                <span class="spinner-label">"Initial heading (°)"</span>
                <crate::ui::Spinner
                    value=Signal::derive(move || generation_config.with(|g| g.initial_heading))
                    step=1.0
                    decimals=1
                    disabled=is_dirty
                    on_commit=move |v: f32| {
                        update_clean_config(
                            config_workspace, grammar_error,"initial_heading",
                            move |clean| clean.set_initial_heading(v),
                        );
                    }
                />
            </div>

            <div class="spinner-row" title=dirty_tooltip>
                <span class="spinner-label">"Iterations"</span>
                <crate::ui::Spinner
                    value=iterations
                    step=1.0
                    max=max_iterations
                    disabled=is_dirty
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
