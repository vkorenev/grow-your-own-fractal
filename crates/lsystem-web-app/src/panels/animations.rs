use crate::app::{ConfigContext, RenderContext};
use leptos::prelude::*;
use lsystem_app_model::{
    HUE_ROTATION_MAX_SPEED_DEGREES_PER_SECOND, HUE_ROTATION_MIN_SPEED_DEGREES_PER_SECOND,
    HueRotationDirection,
};
use lsystem_core::LineColorConfig;

#[component]
pub(crate) fn AnimationsPanel() -> impl IntoView {
    let ConfigContext {
        is_3d,
        control_line_color,
        ..
    } = expect_context();
    let RenderContext {
        auto_rotate,
        auto_rotate_speed,
        hue_rotation,
        animation_error,
        set_hue_rotation,
        ..
    } = expect_context();

    view! {
        <crate::ui::Disclosure title="Animations">
            <div style="display:flex;flex-direction:column;gap:6px">
                <span class="section-label">"Auto-rotate"</span>
                <crate::ui::SegmentedToggle
                    options=vec![(false, "Off"), (true, "On")]
                    selected=Signal::derive(move || auto_rotate.get())
                    on_change=move |on| auto_rotate.set(on)
                    disabled=Signal::derive(move || !is_3d.get())
                />
                <Show when=move || auto_rotate.get() && is_3d.get()>
                    <div class="spinner-row">
                        <span class="spinner-label">"Speed (°/s)"</span>
                        <crate::ui::Spinner
                            value=auto_rotate_speed
                            step=5.0
                            min=5.0_f32
                            max=360.0_f32
                            decimals=0
                            on_commit=move |v: f32| auto_rotate_speed.set(v)
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
                <Show when=move || hue_rotation.with(|m| m.is_enabled())>
                    <div class="spinner-row">
                        <span class="spinner-label">"Speed (°/s)"</span>
                        <crate::ui::Spinner
                            value=Signal::derive(move || hue_rotation.with(|m| m.speed_degrees_per_second()))
                            step=1.0
                            min=HUE_ROTATION_MIN_SPEED_DEGREES_PER_SECOND
                            max=HUE_ROTATION_MAX_SPEED_DEGREES_PER_SECOND
                            decimals=0
                            on_commit=move |v: f32| hue_rotation.update(|m| m.set_speed(v))
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
