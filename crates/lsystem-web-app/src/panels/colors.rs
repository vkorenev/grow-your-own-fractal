use crate::app::{ConfigContext, update_clean_config};
use leptos::prelude::*;
use lsystem_app_model::{
    ColorControlMemory, ConfigDefaults, EditorLineColorConfig, LineColorMode,
    selected_line_color_mode,
};
use lsystem_core::{LineColorConfig, Rgb};

/// A "Default" checkbox + color input pair. Parses the color input and reports
/// parse failures to `error`; all remember-in-memory / config-update logic
/// stays in the callbacks.
#[component]
fn ColorOverrideRow(
    checkbox_id: &'static str,
    color_id: &'static str,
    /// Label shown before the checkbox (e.g. "Start"); None for the background row.
    #[prop(optional)]
    label: Option<&'static str>,
    #[prop(into)] is_default: Signal<bool>,
    #[prop(into)] color: Signal<Rgb>,
    #[prop(into)] disabled: Signal<bool>,
    on_default_change: Callback<bool>,
    on_color_change: Callback<Rgb>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <div class="color-row">
            <label class="check-row" for=checkbox_id>
                {label.map(|l| view! { <span>{l}</span> })}
                <input
                    id=checkbox_id
                    type="checkbox"
                    prop:checked=move || is_default.get()
                    disabled=disabled
                    on:change:target=move |ev| on_default_change.run(ev.target().checked())
                />
                <span>"Default"</span>
            </label>
            <input
                id=color_id
                type="color"
                prop:value=move || color.get().to_string()
                disabled=disabled
                on:input:target=move |ev| {
                    match ev.target().value().parse::<Rgb>() {
                        Ok(c) => on_color_change.run(c),
                        Err(_) => error.set(Some("Invalid color value.".to_string())),
                    }
                }
            />
        </div>
    }
}

#[component]
pub(crate) fn ColorsPanel() -> impl IntoView {
    let ConfigContext {
        config_workspace,
        colors_error,
        editor_color_config,
        control_line_color,
        color_memory,
        is_dirty,
        ..
    } = expect_context();

    let dirty_tooltip = move || {
        if is_dirty.get() {
            "Apply or Revert TOML changes first"
        } else {
            ""
        }
    };

    view! {
        <crate::ui::Disclosure title="Colors">
            <div
                style="display:flex;flex-direction:column;gap:9px"
                title=dirty_tooltip
            >
            <span class="section-label">"Background"</span>
            <ColorOverrideRow
                checkbox_id="background-override"
                color_id="background-color"
                is_default=Signal::derive(move || {
                    editor_color_config.with(|c| c.background.is_none())
                })
                color=Signal::derive(move || {
                    editor_color_config.with(|editor| {
                        editor.background.unwrap_or(ConfigDefaults::embedded().colors.background)
                    })
                })
                disabled=is_dirty
                on_default_change=Callback::new(move |use_default: bool| {
                    if use_default {
                        let current_bg = editor_color_config.with_untracked(|editor| {
                            editor.background.unwrap_or(
                                ConfigDefaults::embedded().colors.background,
                            )
                        });
                        color_memory.update(|memory| {
                            memory.remember_background(current_bg);
                        });
                    }
                    let background = if use_default {
                        None
                    } else {
                        Some(color_memory.get_untracked().background())
                    };
                    update_clean_config(
                        config_workspace,
                        colors_error,
                        "background checkbox",
                        move |clean| clean.set_background(background),
                    );
                })
                on_color_change=Callback::new(move |color: Rgb| {
                    if update_clean_config(
                        config_workspace,
                        colors_error,
                        "background color input",
                        move |clean| clean.set_background(Some(color)),
                    ) {
                        color_memory.update(|memory| {
                            memory.remember_background(color);
                        });
                    }
                })
                error=colors_error
            />

            <hr class="section-divider" />

            <span class="section-label">"Line"</span>
            <div style="display:flex;align-items:center;gap:8px">
            <label for="line-color-mode">"Style"</label>
            <select
                id="line-color-mode"
                style="flex:1"
                prop:value=move || {
                    editor_color_config
                        .with(|editor| {
                            selected_line_color_mode(editor)
                        })
                        .key()
                        .to_string()
                }
                disabled=is_dirty
                on:change:target=move |ev| {
                    let mode_key = ev.target().value();
                    let Some(mode) = LineColorMode::from_key(&mode_key) else {
                        log::error!("unknown line color mode selected: {mode_key}");
                        return;
                    };
                    let editor_line =
                        editor_color_config.with_untracked(|e| e.line);
                    let Some(new_editor_line) = color_memory.try_update(|memory| {
                        memory.remember_line(editor_line);
                        memory.line_for(mode)
                    }) else {
                        log::error!(
                            "line color mode select: line color memory signal was unavailable"
                        );
                        colors_error.set(Some(
                            "Internal error: could not update line color.".to_string(),
                        ));
                        return;
                    };
                    if update_clean_config(
                        config_workspace,
                        colors_error,
                        "line color mode select",
                        move |clean| clean.set_line_color(new_editor_line),
                    ) {
                        color_memory.update(|memory| {
                            memory.remember_line(new_editor_line);
                        });
                    }
                }
            >
                <option value="solid">"Solid"</option>
                <option value="gradient">"Gradient"</option>
                <option value="hue_cycle">"Hue cycle"</option>
            </select>
            </div>

            <div
                class:hidden=move || {
                    !matches!(
                        control_line_color.get(),
                        LineColorConfig::Solid(_)
                    )
                }
                style="display:flex;flex-direction:column;gap:6px"
            >
                <ColorOverrideRow
                    checkbox_id="line-solid-use-default"
                    color_id="line-solid-color"
                    label="Color"
                    is_default=Signal::derive(move || {
                        editor_color_config.with(|c| c.line.is_none())
                    })
                    color=Signal::derive(move || {
                        solid_color_for_mode(control_line_color, color_memory)
                    })
                    disabled=is_dirty
                    on_default_change=Callback::new(move |use_default: bool| {
                        if use_default {
                            let editor_line = editor_color_config.with_untracked(|e| e.line);
                            color_memory.update(|m| m.remember_line(editor_line));
                        }
                        let line_color = if use_default {
                            None
                        } else {
                            Some(EditorLineColorConfig::Solid(
                                color_memory.get_untracked().solid_color(),
                            ))
                        };
                        update_clean_config(
                            config_workspace,
                            colors_error,
                            "solid use-default checkbox",
                            move |clean| clean.set_line_color(line_color),
                        );
                    })
                    on_color_change=Callback::new(move |color: Rgb| {
                        let line_color = Some(EditorLineColorConfig::Solid(color));
                        if update_clean_config(
                            config_workspace,
                            colors_error,
                            "solid line color input",
                            move |clean| clean.set_line_color(line_color),
                        ) {
                            color_memory.update(|memory| {
                                memory.remember_line(Some(EditorLineColorConfig::Solid(color)));
                            });
                        }
                    })
                    error=colors_error
                />
            </div>

            <div
                class:hidden=move || {
                    !matches!(
                        control_line_color.get(),
                        LineColorConfig::Gradient { .. }
                    )
                }
                style="display:flex;flex-direction:column;gap:6px"
            >
                <ColorOverrideRow
                    checkbox_id="line-gradient-start-use-default"
                    color_id="line-gradient-start"
                    label="Start"
                    is_default=Signal::derive(move || {
                        editor_color_config.with(|c| {
                            c.line.map(|l| l.gradient_fields()).unwrap_or_default().0.is_none()
                        })
                    })
                    color=Signal::derive(move || {
                        let (start, _, _) = gradient_fields_for_mode(
                            control_line_color,
                            color_memory,
                        );
                        start
                    })
                    disabled=is_dirty
                    on_default_change=Callback::new(move |use_default: bool| {
                        let editor_line = editor_color_config.with_untracked(|e| e.line);
                        if use_default {
                            color_memory.update(|m| m.remember_line(editor_line));
                        }
                        let (_, editor_end, editor_td) =
                            editor_line.map(|l| l.gradient_fields()).unwrap_or_default();
                        let start = if use_default {
                            None
                        } else {
                            Some(color_memory.get_untracked().gradient_fields().0)
                        };
                        let line_color =
                            Some(EditorLineColorConfig::Gradient {
                                start,
                                end: editor_end,
                                topological_depth: editor_td,
                            });
                        update_clean_config(
                            config_workspace,
                            colors_error,
                            "gradient start use-default",
                            move |clean| clean.set_line_color(line_color),
                        );
                    })
                    on_color_change=Callback::new(move |start: Rgb| {
                        let editor_line = editor_color_config.get_untracked().line;
                        let (_, editor_end, editor_td) =
                            editor_line.map(|l| l.gradient_fields()).unwrap_or_default();
                        let line_color =
                            Some(EditorLineColorConfig::Gradient {
                                start: Some(start),
                                end: editor_end,
                                topological_depth: editor_td,
                            });
                        if update_clean_config(
                            config_workspace,
                            colors_error,
                            "gradient start color input",
                            move |clean| clean.set_line_color(line_color),
                        ) {
                            color_memory.update(|memory| {
                                memory.remember_line(line_color);
                            });
                        }
                    })
                    error=colors_error
                />

                <ColorOverrideRow
                    checkbox_id="line-gradient-end-use-default"
                    color_id="line-gradient-end"
                    label="End"
                    is_default=Signal::derive(move || {
                        editor_color_config.with(|c| {
                            c.line.map(|l| l.gradient_fields()).unwrap_or_default().1.is_none()
                        })
                    })
                    color=Signal::derive(move || {
                        let (_, end, _) = gradient_fields_for_mode(
                            control_line_color,
                            color_memory,
                        );
                        end
                    })
                    disabled=is_dirty
                    on_default_change=Callback::new(move |use_default: bool| {
                        let editor_line = editor_color_config.with_untracked(|e| e.line);
                        if use_default {
                            color_memory.update(|m| m.remember_line(editor_line));
                        }
                        let (editor_start, _, editor_td) =
                            editor_line.map(|l| l.gradient_fields()).unwrap_or_default();
                        let end = if use_default {
                            None
                        } else {
                            Some(color_memory.get_untracked().gradient_fields().1)
                        };
                        let line_color =
                            Some(EditorLineColorConfig::Gradient {
                                start: editor_start,
                                end,
                                topological_depth: editor_td,
                            });
                        update_clean_config(
                            config_workspace,
                            colors_error,
                            "gradient end use-default",
                            move |clean| clean.set_line_color(line_color),
                        );
                    })
                    on_color_change=Callback::new(move |end: Rgb| {
                        let editor_line = editor_color_config.get_untracked().line;
                        let (editor_start, _, editor_td) =
                            editor_line.map(|l| l.gradient_fields()).unwrap_or_default();
                        let line_color =
                            Some(EditorLineColorConfig::Gradient {
                                start: editor_start,
                                end: Some(end),
                                topological_depth: editor_td,
                            });
                        if update_clean_config(
                            config_workspace,
                            colors_error,
                            "gradient end color input",
                            move |clean| clean.set_line_color(line_color),
                        ) {
                            color_memory.update(|memory| {
                                memory.remember_line(line_color);
                            });
                        }
                    })
                    error=colors_error
                />

                <label class="check-row" for="line-gradient-topological-depth">
                    <input
                        id="line-gradient-topological-depth"
                        type="checkbox"
                        prop:checked=move || {
                            let (_, _, topological_depth) = gradient_fields_for_mode(
                                control_line_color,
                                color_memory,
                            );
                            topological_depth
                        }
                        disabled=is_dirty
                        on:change:target=move |ev| {
                            let editor_line = editor_color_config.get_untracked().line;
                            let (editor_start, editor_end, _) =
                                editor_line.map(|l| l.gradient_fields()).unwrap_or_default();
                            let checked = ev.target().checked();
                            let line_color = Some(EditorLineColorConfig::Gradient {
                                start: editor_start,
                                end: editor_end,
                                topological_depth: Some(checked),
                            });
                            if update_clean_config(
                                config_workspace,
                                colors_error,
                                "gradient topological depth toggle",
                                move |clean| clean.set_line_color(line_color),
                            ) {
                                color_memory.update(|memory| {
                                    memory.remember_line(line_color);
                                });
                            }
                        }
                    />
                    <span>"Color by topological depth"</span>
                </label>
            </div>

            <div
                class:hidden=move || {
                    !matches!(
                        control_line_color.get(),
                        LineColorConfig::HueCycle { .. }
                    )
                }
                style="display:flex;flex-direction:column;gap:6px"
            >
                <ColorOverrideRow
                    checkbox_id="line-hue-cycle-use-default"
                    color_id="line-hue-cycle-initial"
                    label="Initial"
                    is_default=Signal::derive(move || {
                        matches!(
                            editor_color_config.with(|c| c.line),
                            None | Some(EditorLineColorConfig::HueCycle {
                                initial: None
                            })
                        )
                    })
                    color=Signal::derive(move || {
                        hue_cycle_initial_for_mode(control_line_color, color_memory)
                    })
                    disabled=is_dirty
                    on_default_change=Callback::new(move |use_default: bool| {
                        if use_default {
                            let editor_line = editor_color_config.with_untracked(|e| e.line);
                            color_memory.update(|m| m.remember_line(editor_line));
                        }
                        let line_color = if use_default {
                            Some(EditorLineColorConfig::HueCycle { initial: None })
                        } else {
                            Some(EditorLineColorConfig::HueCycle {
                                initial: Some(
                                    color_memory.get_untracked().hue_cycle_initial(),
                                ),
                            })
                        };
                        update_clean_config(
                            config_workspace,
                            colors_error,
                            "hue-cycle use-default checkbox",
                            move |clean| clean.set_line_color(line_color),
                        );
                    })
                    on_color_change=Callback::new(move |initial: Rgb| {
                        let line_color =
                            Some(EditorLineColorConfig::HueCycle { initial: Some(initial) });
                        if update_clean_config(
                            config_workspace,
                            colors_error,
                            "hue-cycle initial color input",
                            move |clean| clean.set_line_color(line_color),
                        ) {
                            color_memory.update(|memory| {
                                memory.remember_line(Some(EditorLineColorConfig::HueCycle {
                                    initial: Some(initial),
                                }));
                            });
                        }
                    })
                    error=colors_error
                />
            </div>
            </div>
            {move || colors_error.get().map(|msg| view! {
                <span class="inline-status error">{msg}</span>
            })}
        </crate::ui::Disclosure>
    }
}

fn solid_color_for_mode(
    line_color: Memo<LineColorConfig>,
    memory: RwSignal<ColorControlMemory>,
) -> Rgb {
    match line_color.get() {
        LineColorConfig::Solid(color) => color,
        _ => memory.with(|m| m.solid_color()),
    }
}

fn gradient_fields_from(line: LineColorConfig, fallback: (Rgb, Rgb, bool)) -> (Rgb, Rgb, bool) {
    match line {
        LineColorConfig::Gradient {
            start,
            end,
            topological_depth,
        } => (start, end, topological_depth),
        _ => fallback,
    }
}

fn gradient_fields_for_mode(
    line_color: Memo<LineColorConfig>,
    memory: RwSignal<ColorControlMemory>,
) -> (Rgb, Rgb, bool) {
    gradient_fields_from(line_color.get(), memory.with(|m| m.gradient_fields()))
}

fn hue_cycle_initial_for_mode(
    line_color: Memo<LineColorConfig>,
    memory: RwSignal<ColorControlMemory>,
) -> Rgb {
    match line_color.get() {
        LineColorConfig::HueCycle { initial } => initial,
        _ => memory.with(|m| m.hue_cycle_initial()),
    }
}
