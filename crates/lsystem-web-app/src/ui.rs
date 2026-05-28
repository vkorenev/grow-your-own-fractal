#![allow(dead_code)]
use leptos::prelude::*;

/// Collapsible disclosure section with a chevron header button.
#[component]
pub fn Disclosure(
    title: &'static str,
    #[prop(default = true)] open: bool,
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
                <span>{title}</span>
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
/// `on_commit` — called with the new string on Enter or ± click only; blur does not commit.
///   On blur with unparseable content the field resets to `value`.
/// `step` — amount added/subtracted by ± buttons (parsed as f64 from `value`).
#[component]
pub fn Spinner(
    value: Signal<String>,
    on_commit: impl Fn(String) + 'static + Clone,
    #[prop(default = 1.0_f64)] step: f64,
    #[prop(default = false)] disabled: bool,
) -> impl IntoView {
    let displayed = RwSignal::new(value.get_untracked());

    // Keep displayed in sync when the authoritative value changes from outside
    Effect::new(move |_| displayed.set(value.get()));

    let step_down = {
        let on_commit = on_commit.clone();
        move |_: web_sys::MouseEvent| {
            if let Ok(n) = displayed.get_untracked().parse::<f64>() {
                on_commit(format_step(n - step));
            }
        }
    };

    let step_up = {
        let on_commit = on_commit.clone();
        move |_: web_sys::MouseEvent| {
            if let Ok(n) = displayed.get_untracked().parse::<f64>() {
                on_commit(format_step(n + step));
            }
        }
    };

    view! {
        <div class="spinner">
            <button
                type="button"
                class="spinner-btn"
                disabled=disabled
                on:click=step_down
            >"−"</button>
            <input
                type="text"
                class="spinner-input"
                prop:value=move || displayed.get()
                disabled=disabled
                on:input:target=move |ev| displayed.set(ev.target().value())
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if ev.key() == "Enter" {
                        on_commit(displayed.get_untracked());
                    }
                }
                on:blur=move |_| {
                    // If the field contains something that won't parse, reset to last good value
                    if displayed.get_untracked().parse::<f64>().is_err() {
                        displayed.set(value.get_untracked());
                    }
                }
            />
            <button
                type="button"
                class="spinner-btn"
                disabled=disabled
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
/// `disabled` accepts a plain bool, a closure, or a signal via `#[prop(into)]`.
#[component]
pub fn SegmentedToggle(
    options: Vec<(&'static str, &'static str)>,
    selected: Signal<&'static str>,
    on_change: impl Fn(&'static str) + 'static + Clone,
    #[prop(into, default = Signal::stored(false))] disabled: Signal<bool>,
) -> impl IntoView {
    view! {
        <div class="seg-toggle">
            {options.into_iter().map(|(key, label)| {
                let on_change = on_change.clone();
                view! {
                    <button
                        type="button"
                        class:seg-active=move || selected.get() == key
                        disabled=move || disabled.get() || selected.get() == key
                        on:click=move |_| on_change(key)
                    >
                        {label}
                    </button>
                }
            }).collect_view()}
        </div>
    }
}

/// Inline amber warning box with a confirm action and a cancel button.
#[component]
pub fn WarningPrompt(
    message: &'static str,
    confirm_label: &'static str,
    on_confirm: impl Fn() + 'static,
    on_cancel: impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <div class="warning-prompt">
            <span>{message}</span>
            <div class="warning-prompt-actions">
                <button
                    type="button"
                    class="warning-confirm-btn"
                    on:click=move |_| on_confirm()
                >{confirm_label}</button>
                <button
                    type="button"
                    class="warning-cancel-btn"
                    on:click=move |_| on_cancel()
                >"Cancel"</button>
            </div>
        </div>
    }
}
