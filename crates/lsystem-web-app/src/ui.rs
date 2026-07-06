use leptos::prelude::*;

/// Collapsible disclosure section with a chevron header button.
#[component]
pub fn Disclosure(
    title: &'static str,
    #[prop(default = true)] open: bool,
    /// Amber dot shown in the header when true (e.g. unsaved state).
    #[prop(into, default = Signal::stored(false))]
    badge: Signal<bool>,
    children: Children,
) -> impl IntoView {
    let is_open = RwSignal::new(open);
    view! {
        <div class="disclosure">
            <button
                type="button"
                class="disclosure-header"
                on:click=move |_| is_open.update(|v| *v = !*v)
            >
                <span>
                    {title}
                    <Show when=move || badge.get()>
                        <span class="disclosure-badge">"●"</span>
                    </Show>
                </span>
                <span
                    class="disclosure-chevron"
                    class:open=move || is_open.get()
                >"▶"</span>
            </button>
            <div class="disclosure-body" class:hidden=move || !is_open.get()>
                {children()}
            </div>
        </div>
    }
}

/// Text input flanked by − and + buttons.
///
/// `value` — reactive display string (controlled from outside).
/// `on_commit` — called with the new string on Enter, ± click, or blur with a valid value.
///   On blur with unparseable content the field resets to `value`.
/// `step` — amount added/subtracted by ± buttons (parsed as f64 from `value`).
#[component]
pub fn Spinner(
    value: Signal<String>,
    on_commit: impl Fn(String) + 'static + Clone,
    #[prop(default = 1.0_f64)] step: f64,
    #[prop(into, default = Signal::stored(false))] disabled: Signal<bool>,
) -> impl IntoView {
    let editing = RwSignal::new(false);
    let draft = RwSignal::new(String::new());
    // While editing, show the user's draft; otherwise mirror the authoritative value.
    let shown = move || {
        if editing.get() {
            draft.get()
        } else {
            value.get()
        }
    };
    let current_text = move || {
        if editing.get_untracked() {
            draft.get_untracked()
        } else {
            value.get_untracked()
        }
    };

    let step_down = {
        let on_commit = on_commit.clone();
        move |_: web_sys::MouseEvent| {
            if let Ok(n) = current_text().parse::<f64>() {
                on_commit(format_step(n - step));
            }
        }
    };

    let step_up = {
        let on_commit = on_commit.clone();
        move |_: web_sys::MouseEvent| {
            if let Ok(n) = current_text().parse::<f64>() {
                on_commit(format_step(n + step));
            }
        }
    };

    let on_commit_enter = on_commit.clone();
    let on_commit_blur = on_commit.clone();

    view! {
        <div class="spinner">
            <button
                type="button"
                class="spinner-btn"
                disabled=move || disabled.get()
                on:click=step_down
            >"−"</button>
            <input
                type="text"
                class="spinner-input"
                prop:value=shown
                disabled=move || disabled.get()
                on:focus=move |_| {
                    draft.set(value.get_untracked());
                    editing.set(true);
                }
                on:input:target=move |ev| {
                    editing.set(true);
                    draft.set(ev.target().value());
                }
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if ev.key() == "Enter" {
                        on_commit_enter(draft.get_untracked());
                        // Refresh the draft from the committed (clamped/formatted) value so
                        // continued typing starts from what is displayed.
                        draft.set(value.get_untracked());
                    }
                }
                on:blur=move |_| {
                    let text = draft.get_untracked();
                    editing.set(false);
                    if text.parse::<f64>().is_ok() {
                        on_commit_blur(text);
                    }
                }
            />
            <button
                type="button"
                class="spinner-btn"
                disabled=move || disabled.get()
                on:click=step_up
            >"+"</button>
        </div>
    }
}

/// Format a float for spinner display: no trailing zeros, at most 4 decimal places.
fn format_step(n: f64) -> String {
    let s = format!("{n:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Segmented toggle (pill group). `options` is `(key, label)` pairs.
/// Fires `on_change(key)` when a non-active option is clicked.
/// `disabled` disables all buttons. `disabled_keys` disables specific option keys.
#[component]
pub fn SegmentedToggle<K>(
    options: Vec<(K, &'static str)>,
    #[prop(into)] selected: Signal<K>,
    on_change: impl Fn(K) + 'static + Clone,
    #[prop(into, default = Signal::stored(false))] disabled: Signal<bool>,
    #[prop(into, default = Signal::stored(Vec::new()))] disabled_keys: Signal<Vec<K>>,
) -> impl IntoView
where
    K: PartialEq + Copy + Send + Sync + 'static,
{
    view! {
        <div class="seg-toggle">
            {options.into_iter().map(|(key, label)| {
                let on_change = on_change.clone();
                view! {
                    <button
                        type="button"
                        class:seg-active=move || selected.get() == key
                        disabled=move || {
                            disabled.get()
                                || selected.get() == key
                                || disabled_keys.get().contains(&key)
                        }
                        on:click=move |_| on_change(key)
                    >
                        {label}
                    </button>
                }
            }).collect_view()}
        </div>
    }
}
