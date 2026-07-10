use crate::app::ConfigContext;
use leptos::prelude::*;
use lsystem_app_model::EntryViewMut;

#[component]
pub(crate) fn ConfigTomlPanel() -> impl IntoView {
    let ConfigContext {
        config_workspace,
        toml_text,
        toml_error,
        is_dirty,
        grammar,
        select_current_config,
        ..
    } = expect_context();
    let grammar_is_dirty = grammar.is_dirty;

    let apply_current = move || {
        if !is_dirty.get_untracked() {
            return;
        }
        let result = config_workspace.write().selected_mut().apply_draft();
        match result {
            Ok(()) => select_current_config.run(()),
            Err(e) => toml_error.set(Some(e.to_string())),
        }
    };

    let do_revert = move || {
        let reverted = match config_workspace.write().selected_mut().view_mut() {
            EntryViewMut::Dirty(dirty) => {
                dirty.revert();
                true
            }
            EntryViewMut::Clean(_) => {
                log::error!("revert fired while entry is clean; UI guards bypassed");
                false
            }
        };
        if reverted {
            select_current_config.run(());
        }
    };

    let grammar_dirty_tooltip = move || {
        if grammar_is_dirty.get() {
            "Apply or Revert grammar changes first"
        } else {
            ""
        }
    };

    view! {
        <crate::ui::Disclosure title="Edit Config" open=false
            badge=is_dirty>
            <div title=grammar_dirty_tooltip>
            <textarea
                id="config"
                spellcheck="false"
                prop:value=move || toml_text.get()
                disabled=grammar_is_dirty
                on:input:target=move |ev| {
                    let text = ev.target().value();
                    config_workspace.update(|workspace| {
                        workspace.selected_mut().set_draft_text(text);
                    });
                }
            />
            </div>
            <div title=grammar_dirty_tooltip>
            <div class="btn-row">
                <button
                    type="button"
                    disabled=move || !is_dirty.get() || grammar_is_dirty.get()
                    on:click=move |_| apply_current()
                >
                    "Apply"
                </button>
                <button
                    type="button"
                    disabled=move || !is_dirty.get() || grammar_is_dirty.get()
                    on:click=move |_| do_revert()
                >
                    "Revert"
                </button>
            </div>
            </div>
            {move || toml_error.get().map(|msg| view! {
                <span class="inline-status error">{msg}</span>
            })}
        </crate::ui::Disclosure>
    }
}
