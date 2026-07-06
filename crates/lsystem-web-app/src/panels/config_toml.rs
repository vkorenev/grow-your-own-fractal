use leptos::prelude::*;
use lsystem_app_model::{ConfigWorkspace, EntryViewMut};

#[component]
pub(crate) fn ConfigTomlPanel(
    config_workspace: RwSignal<ConfigWorkspace>,
    toml_text: Memo<String>,
    toml_error: RwSignal<Option<String>>,
    is_dirty: Memo<bool>,
    grammar_is_dirty: Memo<bool>,
    select_current_config: Callback<()>,
) -> impl IntoView {
    let apply_current = move || {
        if !is_dirty.get_untracked() {
            return;
        }
        let result = config_workspace.try_update(|workspace| {
            workspace
                .selected_mut()
                .apply_draft()
                .map_err(|e| e.to_string())
        });
        match result {
            Some(Ok(())) => select_current_config.run(()),
            Some(Err(msg)) => toml_error.set(Some(msg)),
            None => {
                log::error!("apply: config_workspace signal was unavailable");
                toml_error.set(Some("Internal error: could not apply config.".to_string()));
            }
        }
    };

    let do_revert = move || {
        let result =
            config_workspace.try_update(|workspace| match workspace.selected_mut().view_mut() {
                EntryViewMut::Dirty(dirty) => {
                    dirty.revert();
                    true
                }
                EntryViewMut::Clean(_) => {
                    log::error!("revert fired while entry is clean; UI guards bypassed");
                    false
                }
            });
        match result {
            Some(true) => select_current_config.run(()),
            Some(false) => {}
            None => {
                log::error!("revert: config_workspace signal was unavailable");
                toml_error.set(Some("Internal error: could not revert config.".to_string()));
            }
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
                    let updated = config_workspace.try_update(|workspace| {
                        workspace.selected_mut().set_draft_text(text);
                    });
                    if updated.is_none() {
                        log::error!("textarea input: config_workspace signal was unavailable");
                        toml_error.set(Some(
                            "Internal error: could not update config.".to_string(),
                        ));
                    }
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
