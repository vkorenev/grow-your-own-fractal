use crate::app::ConfigContext;
use crate::export::download_toml;
use leptos::html::Input;
use leptos::prelude::*;
use lsystem_app_model::{ConfigEntryId, ConfigWorkspaceError};

#[component]
pub(crate) fn PresetPanel() -> impl IntoView {
    let ConfigContext {
        config_workspace,
        selected_id,
        selected_name,
        display_options,
        selected_is_bundled,
        can_reset,
        workspace_error,
        persistence_warning,
        toml_text,
        select_current_config,
        ..
    } = expect_context();

    let rename_mode = RwSignal::new(false);
    // The custom entry awaiting removal confirmation. Per the specification only a
    // selection change dismisses it, so it is cleared by a watcher keyed on `selected_id`
    // alone (unlike the rename form, draft edits must not dismiss it) and confirming
    // removes only this id.
    let pending_removal = RwSignal::new(None::<ConfigEntryId>);
    let rename_draft = RwSignal::new(String::new());
    let file_input_ref = NodeRef::<Input>::new();

    // Rename is local UI state, but raw TOML and direct controls mutate the shared
    // workspace from sibling panels. Close the form only when the selected entry's actual
    // state changes; merely dropping a write guard after a failed mutation must leave the
    // user's rejected input available for correction.
    Effect::watch(
        move || {
            config_workspace.with(|workspace| {
                let selected = workspace.selected();
                (
                    selected.id(),
                    selected.is_dirty(),
                    selected.draft_text().into_owned(),
                )
            })
        },
        move |current, previous, _: Option<()>| {
            if previous != Some(current) {
                rename_mode.set(false);
            }
        },
        false,
    );

    Effect::watch(
        move || selected_id.get(),
        move |_, _, _: Option<()>| pending_removal.set(None),
        false,
    );

    let commit_rename = move || {
        let name = rename_draft.get_untracked().trim().to_string();
        let result = config_workspace.write().selected_mut().rename(&name);
        match result {
            Ok(()) => {
                workspace_error.set(None);
                rename_mode.set(false);
            }
            Err(e) => workspace_error.set(Some(e.to_string())),
        }
    };

    // Reset is a workspace action, not a direct control: `can_reset` keeps it enabled while
    // a raw TOML draft is pending, and it deliberately discards that draft.
    let do_reset = move || {
        if !can_reset.get_untracked() {
            log::warn!("do_reset: nothing to reset; button guard may have been bypassed");
            return;
        }
        let reset = config_workspace.write().selected_mut().reset_to_default();
        if reset {
            select_current_config.run(());
        } else {
            log::warn!(
                "do_reset: no-op for entry without a bundled default; button guard may have been bypassed"
            );
        }
    };

    let begin_remove = move || {
        if selected_is_bundled.get_untracked() {
            log::error!("begin_remove: bundled entry; button guard may have been bypassed");
            workspace_error.set(Some(ConfigWorkspaceError::CannotRemoveBundled.to_string()));
            return;
        }
        rename_mode.set(false);
        pending_removal.set(Some(selected_id.get_untracked()));
    };

    // Removal changes `selected_id`, so the watcher in `App` (`select_current_config`)
    // resyncs the editors and clears panel errors; nothing to resync here.
    let confirm_remove = move |confirmed_id: ConfigEntryId| {
        pending_removal.set(None);
        if config_workspace.with_untracked(|workspace| workspace.selected_id()) != confirmed_id {
            log::error!("confirm_remove: {confirmed_id} is no longer selected");
            workspace_error.set(Some("Internal error: could not remove config.".to_string()));
            return;
        }
        let result = config_workspace.write().remove_selected();
        if let Err(e) = result {
            workspace_error.set(Some(e.to_string()));
        }
    };

    let selected_label = move || {
        let id = selected_id.get();
        display_options.with(|options| {
            options
                .iter()
                .find(|(option_id, _)| *option_id == id)
                .map(|(_, label)| label.clone())
                .unwrap_or_default()
        })
    };

    view! {
        <div class="preset-row">
            <select
                class:hidden=move || rename_mode.get()
                prop:value=move || selected_id.get().to_string()
                on:change:target=move |ev| {
                    let raw = ev.target().value();
                    let Ok(id) = raw.parse::<ConfigEntryId>() else {
                        log::error!("select preset: invalid id in option value: {raw:?}");
                        workspace_error.set(Some(
                            "Internal error: could not select config.".to_string(),
                        ));
                        return;
                    };
                    let selected = config_workspace.write().select_by_id(id);
                    if let Err(err) = selected {
                        log::error!("select preset: rejected id {id}: {err}");
                        workspace_error.set(Some(err.to_string()));
                    }
                }
            >
                {move || {
                    display_options
                        .get()
                        .into_iter()
                        .map(|(id, label)| {
                            view! {
                                <option value=id.to_string()>{label}</option>
                            }
                        })
                        .collect_view()
                }}
            </select>

            <input
                type="text"
                class="rename-input"
                class:hidden=move || !rename_mode.get()
                prop:value=move || rename_draft.get()
                on:input:target=move |ev| rename_draft.set(ev.target().value())
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if ev.key() == "Enter" { commit_rename(); }
                    if ev.key() == "Escape" { rename_mode.set(false); }
                }
            />
        </div>

        <div class="preset-actions">
            {move || if rename_mode.get() {
                view! {
                    <div class="btn-row">
                        <button
                            type="button"
                            disabled=move || rename_draft.get().trim().is_empty()
                            on:click=move |_| commit_rename()
                        >"Save"</button>
                        <button type="button" on:click=move |_| rename_mode.set(false)>"Cancel"</button>
                    </div>
                }.into_any()
            } else if let Some(confirmed_id) =
                pending_removal.get().filter(|id| *id == selected_id.get())
            {
                view! {
                    <div style="display:contents">
                        <span class="inline-status warning">
                            {move || format!(
                                "Remove \"{}\"? Any unapplied changes will be discarded.",
                                selected_label()
                            )}
                        </span>
                        <div class="btn-row">
                            <button
                                type="button"
                                on:click=move |_| confirm_remove(confirmed_id)
                            >"Remove"</button>
                            <button
                                type="button"
                                on:click=move |_| pending_removal.set(None)
                            >"Cancel"</button>
                        </div>
                    </div>
                }.into_any()
            } else {
                view! {
                    <div style="display:contents">
                        <div class="btn-row">
                            <button
                                type="button"
                                on:click=move |_| {
                                    let result = config_workspace.write().copy().map(|_| ());
                                    if let Err(e) = result {
                                        workspace_error.set(Some(e.to_string()));
                                    }
                                }
                            >"Copy"</button>
                            <button
                                type="button"
                                on:click=move |_| {
                                    rename_draft.set(config_workspace.with_untracked(|workspace| {
                                        workspace.selected().name_for_rename().into_owned()
                                    }));
                                    pending_removal.set(None);
                                    rename_mode.set(true);
                                }
                            >"Rename"</button>
                            // Bundled entries can only be reset; custom entries can only be
                            // removed.
                            {move || if selected_is_bundled.get() {
                                view! {
                                    <button
                                        type="button"
                                        disabled=move || !can_reset.get()
                                        on:click=move |_| do_reset()
                                    >"Reset"</button>
                                }.into_any()
                            } else {
                                view! {
                                    <button
                                        type="button"
                                        on:click=move |_| begin_remove()
                                    >"Remove"</button>
                                }.into_any()
                            }}
                        </div>
                        <hr class="section-divider" />
                        <div class="btn-row">
                            <button
                                type="button"
                                on:click=move |_| {
                                    if let Some(el) = file_input_ref.get_untracked() {
                                        el.click();
                                    } else {
                                        workspace_error.set(Some(
                                            "Internal error: upload input unavailable.".to_string(),
                                        ));
                                    }
                                }
                            >"Open"</button>
                            <button
                                type="button"
                                on:click=move |_| {
                                    workspace_error.set(None);
                                    let name = selected_name.get_untracked();
                                    download_toml(&name, &toml_text.get_untracked());
                                }
                            >"Save"</button>
                        </div>
                    </div>
                }.into_any()
            }}
        </div>
        {move || workspace_error.get().map(|msg| view! {
            <span class="inline-status error">{msg}</span>
        })}
        // Coarse, one-way persistence health notice. Mounted only once persistence has
        // failed, so a healthy session reserves no space for it (`.inline-status` has a
        // minimum height and the desktop container adds a flex gap per child). It is not
        // interactive and never names the change that failed.
        {move || persistence_warning.get().then(|| view! {
            <span
                class="inline-status warning"
                role="status"
                title="Browser storage is not fully working, so recent changes may not be saved."
            >
                "Changes may not be saved"
            </span>
        })}
        <input
            type="file"
            accept=".toml"
            style="display:none"
            node_ref=file_input_ref
            on:change=move |_| {
                let Some(input) = file_input_ref.get_untracked() else {
                    log::error!("import_toml on:change: file_input_ref was None");
                    workspace_error.set(Some(
                        "Internal error: upload input unavailable.".to_string(),
                    ));
                    return;
                };
                let Some(files) = input.files() else {
                    log::error!(
                        "import_toml on:change: input.files() returned None"
                    );
                    return;
                };
                let Some(file) = files.get(0) else { return };
                let file = gloo_file::File::from(file);
                wasm_bindgen_futures::spawn_local(async move {
                    match gloo_file::futures::read_as_text(&file).await {
                        Ok(text) => {
                            let result =
                                config_workspace.write().import_toml(&text).map(|_| ());
                            if let Err(e) = result {
                                workspace_error.set(Some(e.to_string()));
                            }
                        }
                        Err(e) => {
                            workspace_error
                                .set(Some(format!("Failed to read file: {e}")));
                        }
                    }
                    if let Some(input) = file_input_ref.get_untracked() {
                        input.set_value("");
                    }
                });
            }
        />
    }
}
