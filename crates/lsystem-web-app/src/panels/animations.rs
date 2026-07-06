use leptos::prelude::*;
use lsystem_app_model::{HueRotation, HueRotationDirection};
use lsystem_core::LineColorConfig;

#[component]
pub(crate) fn AnimationsPanel(
    is_3d: Memo<bool>,
    auto_rotate: RwSignal<bool>,
    auto_rotate_speed: RwSignal<f32>,
    hue_rotation: RwSignal<HueRotation>,
    control_line_color: Memo<LineColorConfig>,
    animation_error: RwSignal<Option<String>>,
    set_hue_rotation: Callback<Option<HueRotationDirection>>,
) -> impl IntoView {
    view! {
        <crate::ui::Disclosure title="Animations">
            <div style="display:flex;flex-direction:column;gap:6px">
                <span class="section-label">"Auto-rotate"</span>
                <div title=move || if !is_3d.get() { "Switch to 3D mode in Parameters to enable auto-rotate" } else { "" }>
                <crate::ui::SegmentedToggle
                    options=vec![(false, "Off"), (true, "On")]
                    selected=Signal::derive(move || auto_rotate.get())
                    on_change=move |on| auto_rotate.set(on)
                    disabled=Signal::derive(move || !is_3d.get())
                />
                </div>
                <Show when=move || auto_rotate.get() && is_3d.get()>
                    <div class="spinner-row">
                        <span class="spinner-label">"Speed (°/s)"</span>
                        <crate::ui::Spinner
                            value=Signal::derive(move || format!("{:.0}", auto_rotate_speed.get()))
                            step=5.0
                            on_commit=move |s| {
                                if let Ok(v) = s.parse::<f32>() {
                                    auto_rotate_speed.set(v.clamp(5.0, 360.0));
                                }
                            }
                        />
                    </div>
                </Show>
                <Show when=move || !is_3d.get()>
                    <span class="inline-status warning">"Switch to 3D mode to enable auto-rotate"</span>
                </Show>
            </div>

            <hr class="section-divider" />

            <div style="display:flex;flex-direction:column;gap:6px">
                <span class="section-label">"Hue rotation"</span>
                <div title=move || if !matches!(control_line_color.get(), LineColorConfig::HueCycle { .. }) { "Select Hue cycle in Colors to enable hue rotation" } else { "" }>
                <crate::ui::SegmentedToggle
                    options=vec![
                        (None, "Off"),
                        (Some(HueRotationDirection::Forward), "Forward"),
                        (Some(HueRotationDirection::Reverse), "Backward"),
                    ]
                    selected=Signal::derive(move || {
                        hue_rotation.with(|m| m.is_enabled().then(|| m.direction()))
                    })
                    on_change=move |key| set_hue_rotation.run(key)
                    disabled_keys=Signal::derive(move || {
                        if matches!(control_line_color.get(), LineColorConfig::HueCycle { .. }) {
                            vec![]
                        } else {
                            vec![
                                Some(HueRotationDirection::Forward),
                                Some(HueRotationDirection::Reverse),
                            ]
                        }
                    })
                />
                </div>
                <Show when=move || hue_rotation.with(|m| m.is_enabled())>
                    <div class="spinner-row">
                        <span class="spinner-label">"Speed (°/s)"</span>
                        <crate::ui::Spinner
                            value=Signal::derive(move || hue_rotation.with(|m| format!("{:.0}", m.speed_degrees_per_second())))
                            step=1.0
                            on_commit=move |s| {
                                if let Ok(v) = s.parse::<f32>() {
                                    hue_rotation.update(|m| m.set_speed(v));
                                }
                            }
                        />
                    </div>
                </Show>
                <Show when=move || !matches!(control_line_color.get(), LineColorConfig::HueCycle { .. })>
                    <span class="inline-status warning">"Select Hue cycle in Colors to enable hue rotation"</span>
                </Show>
            </div>
            {move || animation_error.get().map(|msg| view! {
                <span class="inline-status error">{msg}</span>
            })}
        </crate::ui::Disclosure>
    }
}
