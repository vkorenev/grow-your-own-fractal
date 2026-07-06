use crate::export::download_toml;
use leptos::html::Input;
use leptos::prelude::*;
use lsystem_app_model::{ConfigEntryId, ConfigWorkspace};

#[component]
pub(crate) fn PresetPanel(
    config_workspace: RwSignal<ConfigWorkspace>,
    selected_id: Memo<ConfigEntryId>,
    selected_name: Memo<String>,
    display_options: Memo<Vec<(ConfigEntryId, String)>>,
    differs_from_default: Memo<bool>,
    workspace_error: RwSignal<Option<String>>,
    toml_text: Memo<String>,
    select_current_config: Callback<()>,
) -> impl IntoView {
    let rename_mode = RwSignal::new(false);
    let rename_draft = RwSignal::new(String::new());
    let file_input_ref = NodeRef::<Input>::new();

    let commit_rename = move || {
        let name = rename_draft.get_untracked().trim().to_string();
        match config_workspace.try_update(|ws| ws.selected_mut().rename(&name)) {
            Some(Ok(())) => {
                workspace_error.set(None);
                rename_mode.set(false);
            }
            Some(Err(e)) => workspace_error.set(Some(e.to_string())),
            None => workspace_error.set(Some("Internal error: could not rename.".to_string())),
        }
    };

    let do_reset = move || {
        let result = config_workspace.try_update(|ws| ws.selected_mut().reset_to_default());
        match result {
            Some(true) => select_current_config.run(()),
            Some(false) => {
                log::warn!(
                    "do_reset: no-op for entry without a bundled default; button guard may have been bypassed"
                );
            }
            None => workspace_error.set(Some("Internal error: could not reset.".to_string())),
        }
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
                    let selected = config_workspace.try_update(|workspace| {
                        workspace
                            .select_by_id(id)
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    });
                    match selected {
                        Some(Ok(())) => {}
                        Some(Err(err)) => {
                            log::error!("select preset: rejected id {id}: {err}");
                            workspace_error.set(Some(err));
                        }
                        None => {
                            log::error!(
                                "select preset: config_workspace signal was unavailable"
                            );
                            workspace_error.set(Some(
                                "Internal error: could not select config.".to_string(),
                            ));
                        }
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
            } else {
                view! {
                    <div style="display:contents">
                        <div class="btn-row">
                            <button
                                type="button"
                                on:click=move |_| {
                                    let result = config_workspace.try_update(|workspace| {
                                        workspace.copy().map(|_| ()).map_err(|e| e.to_string())
                                    });
                                    match result {
                                        Some(Ok(())) => {}
                                        Some(Err(msg)) => workspace_error.set(Some(msg)),
                                        None => {
                                            log::error!("copy: config_workspace signal was unavailable");
                                            workspace_error.set(Some(
                                                "Internal error: could not copy config.".to_string(),
                                            ));
                                        }
                                    }
                                }
                            >"Copy"</button>
                            <button
                                type="button"
                                on:click=move |_| {
                                    rename_draft.set(selected_name.get());
                                    rename_mode.set(true);
                                }
                            >"Rename"</button>
                            <button
                                type="button"
                                disabled=move || !differs_from_default.get()
                                on:click=move |_| do_reset()
                            >"Reset"</button>
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
                            let result = config_workspace
                                .try_update(|ws| ws.import_toml(&text).map(|_| ()));
                            match result {
                                Some(Ok(_)) => {}
                                Some(Err(e)) => {
                                    workspace_error.set(Some(e.to_string()));
                                }
                                None => {
                                    log::error!(
                                        "import_toml: config_workspace signal was \
                                         unavailable"
                                    );
                                    workspace_error.set(Some(
                                        "Internal error: could not import config."
                                            .to_string(),
                                    ));
                                }
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
