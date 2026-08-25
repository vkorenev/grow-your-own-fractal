use crate::app::{ConfigContext, ROTATION_STEP_DEG, RenderContext};
use leptos::prelude::*;

#[component]
pub(crate) fn CameraPanel() -> impl IntoView {
    let ConfigContext { is_3d, .. } = expect_context();
    let RenderContext {
        camera_reset,
        camera_orbit,
        camera_roll,
        camera_ready,
        ..
    } = expect_context();
    let disabled = move || !camera_ready.get();

    view! {
        <crate::ui::Disclosure title="Camera">
            <div class="btn-row">
                <button type="button" disabled=disabled on:click=move |_| camera_reset.run(())>
                    "Reset view"
                </button>
            </div>
            <Show when=move || is_3d.get()>
                <hr class="section-divider" />
                <span class="section-label">"Orbit"</span>
                <div class="btn-row">
                    <button
                        type="button"
                        disabled=disabled
                        on:click=move |_| camera_orbit.run((-ROTATION_STEP_DEG, 0.0))
                    >"◀"</button>
                    <button
                        type="button"
                        disabled=disabled
                        on:click=move |_| camera_orbit.run((ROTATION_STEP_DEG, 0.0))
                    >"▶"</button>
                    <button
                        type="button"
                        disabled=disabled
                        on:click=move |_| camera_orbit.run((0.0, ROTATION_STEP_DEG))
                    >"▲"</button>
                    <button
                        type="button"
                        disabled=disabled
                        on:click=move |_| camera_orbit.run((0.0, -ROTATION_STEP_DEG))
                    >"▼"</button>
                </div>
                <span class="section-label">"Roll"</span>
                <div class="btn-row">
                    <button
                        type="button"
                        disabled=disabled
                        on:click=move |_| camera_roll.run(-ROTATION_STEP_DEG)
                    >"↺"</button>
                    <button
                        type="button"
                        disabled=disabled
                        on:click=move |_| camera_roll.run(ROTATION_STEP_DEG)
                    >"↻"</button>
                </div>
            </Show>
        </crate::ui::Disclosure>
    }
}
