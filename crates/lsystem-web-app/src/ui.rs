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

/// Persistent warning shown at the top of a panel while a draft that the
/// panel's actions would discard is pending — a raw TOML draft for most
/// panels, or a grammar draft for the raw TOML panel itself — since
/// (depending on `message`) using the panel's controls, or applying its
/// own pending draft, will discard that draft. See the "Applied documents
/// and drafts" and "Direct configuration controls" sections of
/// docs/specs/application-workspace.md.
#[component]
pub fn DirtyDraftWarning(
    #[prop(into)] is_dirty: Signal<bool>,
    #[prop(default = "Editing these controls will discard your unapplied TOML changes.")]
    message: &'static str,
) -> impl IntoView {
    view! {
        <Show when=move || is_dirty.get()>
            <span class="inline-status warning">{message}</span>
        </Show>
    }
}

/// Text input that commits its text through `on_commit` only once an edit
/// settles: after `delay` without further input, or when the input loses
/// focus. Until then it shows the uncommitted text, like [`Spinner`]'s draft.
///
/// Committing on blur means a pending edit is committed before any other
/// control's click handler runs, since focus leaves first. If `value`
/// changes from outside while an edit is pending (e.g. the document it
/// edits was replaced), the pending edit is dropped and its timer cancelled,
/// so stale text is never committed into the new value.
#[component]
pub fn DebouncedTextInput(
    #[prop(into)] value: Signal<String>,
    #[prop(into)] on_commit: Callback<String>,
    #[prop(optional)] class: &'static str,
    #[prop(optional)] list: Option<&'static str>,
    #[prop(optional)] maxlength: Option<u32>,
    #[prop(default = std::time::Duration::from_millis(300))] delay: std::time::Duration,
) -> impl IntoView {
    let pending = RwSignal::new(None::<String>);
    let timer = StoredValue::new(None::<TimeoutHandle>);
    let cancel_timer = move || {
        if let Some(handle) = timer.try_update_value(Option::take).flatten() {
            handle.clear();
        }
    };
    let commit = move || {
        cancel_timer();
        if let Some(text) = pending.try_update(Option::take).flatten() {
            on_commit.run(text);
        }
    };

    Effect::watch(
        move || value.track(),
        move |_, _, _: Option<()>| {
            cancel_timer();
            if pending.with_untracked(Option::is_some) {
                pending.set(None);
            }
        },
        false,
    );
    on_cleanup(cancel_timer);

    view! {
        <input
            type="text"
            class=class
            list=list
            maxlength=maxlength
            prop:value=move || pending.get().unwrap_or_else(|| value.get())
            on:input:target=move |ev| {
                pending.set(Some(ev.target().value()));
                cancel_timer();
                match set_timeout_with_handle(commit, delay) {
                    Ok(handle) => timer.set_value(Some(handle)),
                    // Without a timer the edit still commits on blur.
                    Err(err) => log::error!("debounce timer unavailable: {err:?}"),
                }
            }
            on:blur=move |_| commit()
        />
    }
}

/// Numeric value type usable in a [`Spinner`].
pub trait SpinnerValue: Copy + PartialOrd + Send + Sync + 'static {
    fn parse(text: &str) -> Option<Self>;
    fn to_f64(self) -> f64;
    /// Converts a ± stepping result back, rounding/saturating as appropriate.
    fn from_f64(v: f64) -> Self;
    /// Formats for display. `decimals` fixes the fraction width when set.
    fn format(self, decimals: Option<u8>) -> String;
    /// `inputmode` attribute value, so mobile browsers show a numeric keyboard.
    fn input_mode() -> &'static str;
}

impl SpinnerValue for f32 {
    fn parse(text: &str) -> Option<Self> {
        text.trim().parse().ok().filter(|v: &f32| v.is_finite())
    }

    fn to_f64(self) -> f64 {
        f64::from(self)
    }

    fn from_f64(v: f64) -> Self {
        v as f32
    }

    fn format(self, decimals: Option<u8>) -> String {
        match decimals {
            Some(d) => format!("{self:.0$}", usize::from(d)),
            // No trailing zeros, at most 4 decimal places.
            None => {
                let s = format!("{self:.4}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
    }

    fn input_mode() -> &'static str {
        "decimal"
    }
}

impl SpinnerValue for u32 {
    fn parse(text: &str) -> Option<Self> {
        text.trim().parse().ok()
    }

    fn to_f64(self) -> f64 {
        f64::from(self)
    }

    fn from_f64(v: f64) -> Self {
        // `as` saturates at the type bounds, so stepping below 0 lands on 0.
        v.round() as u32
    }

    fn format(self, _decimals: Option<u8>) -> String {
        self.to_string()
    }

    fn input_mode() -> &'static str {
        "numeric"
    }
}

impl SpinnerValue for u16 {
    fn parse(text: &str) -> Option<Self> {
        text.trim().parse().ok()
    }

    fn to_f64(self) -> f64 {
        f64::from(self)
    }

    fn from_f64(v: f64) -> Self {
        // `as` saturates at the type bounds, so stepping below 0 lands on 0.
        v.round() as u16
    }

    fn format(self, _decimals: Option<u8>) -> String {
        self.to_string()
    }

    fn input_mode() -> &'static str {
        "numeric"
    }
}

/// Numeric text input flanked by − and + buttons.
///
/// `value` — the authoritative value (controlled from outside).
/// `on_commit` — called with the parsed, clamped value on Enter, ± click, or blur
///   with valid content. Unparseable content resets the field to `value`.
/// `step` — amount added/subtracted by the ± buttons.
/// `min`/`max` — optional bounds; commits are clamped into them.
/// `decimals` — fixed fraction width for display (floats only).
#[component]
pub fn Spinner<T: SpinnerValue>(
    #[prop(into)] value: Signal<T>,
    on_commit: impl Fn(T) + 'static + Clone,
    #[prop(default = 1.0_f64)] step: f64,
    #[prop(into, optional)] min: MaybeProp<T>,
    #[prop(into, optional)] max: MaybeProp<T>,
    #[prop(optional)] decimals: Option<u8>,
    #[prop(into, default = Signal::stored(false))] disabled: Signal<bool>,
) -> impl IntoView {
    let editing = RwSignal::new(false);
    let draft = RwSignal::new(String::new());
    // While editing, show the user's draft; otherwise mirror the authoritative value.
    let shown = move || {
        if editing.get() {
            draft.get()
        } else {
            value.get().format(decimals)
        }
    };
    let current = move || {
        if editing.get_untracked() {
            T::parse(&draft.get_untracked())
        } else {
            Some(value.get_untracked())
        }
    };

    let commit = move |v: T| {
        let v = match min.get_untracked() {
            Some(lo) if v < lo => lo,
            _ => v,
        };
        let v = match max.get_untracked() {
            Some(hi) if v > hi => hi,
            _ => v,
        };
        on_commit(v);
    };

    let step_by = {
        let commit = commit.clone();
        move |direction: f64| {
            if let Some(v) = current() {
                commit(T::from_f64(v.to_f64() + direction * step));
            }
        }
    };
    let step_down = {
        let step_by = step_by.clone();
        move |_: web_sys::MouseEvent| step_by(-1.0)
    };
    let step_up = move |_: web_sys::MouseEvent| step_by(1.0);
    let commit_enter = commit.clone();
    let commit_blur = commit;

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
                inputmode=T::input_mode()
                class="spinner-input"
                prop:value=shown
                disabled=move || disabled.get()
                on:focus=move |_| {
                    draft.set(value.get_untracked().format(decimals));
                    editing.set(true);
                }
                on:input:target=move |ev| {
                    editing.set(true);
                    draft.set(ev.target().value());
                }
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if ev.key() == "Enter" {
                        if let Some(v) = T::parse(&draft.get_untracked()) {
                            commit_enter(v);
                        }
                        // Refresh the draft from the committed (clamped/formatted) value so
                        // continued typing starts from what is displayed; unparseable
                        // content is discarded the same way.
                        draft.set(value.get_untracked().format(decimals));
                    }
                }
                on:blur=move |_| {
                    editing.set(false);
                    if let Some(v) = T::parse(&draft.get_untracked()) {
                        commit_blur(v);
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

#[cfg(test)]
mod tests {
    use super::SpinnerValue;

    #[test]
    fn u16_spinner_value_enforces_and_saturates_at_type_bounds() {
        assert_eq!(<u16 as SpinnerValue>::parse("0"), Some(0));
        assert_eq!(<u16 as SpinnerValue>::parse("65535"), Some(u16::MAX));
        assert_eq!(<u16 as SpinnerValue>::parse("65536"), None);
        assert_eq!(<u16 as SpinnerValue>::from_f64(-1.0), 0);
        assert_eq!(<u16 as SpinnerValue>::from_f64(65_536.0), u16::MAX);
    }
}
