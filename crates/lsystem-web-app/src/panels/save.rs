use crate::app::RendererState;
use crate::export::{export_png, export_svg};
use leptos::prelude::*;
use lsystem_app_model::{HueRotation, HueRotationDirection};
use lsystem_core::Config;
use lsystem_renderer::animation_export::AnimationParams;
use lsystem_renderer::png_export::{
    MAX_DIMENSION as PNG_MAX_DIMENSION, MIN_DIMENSION as PNG_MIN_DIMENSION,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaveFormat {
    Svg,
    Png,
    Apng,
}

#[component]
pub(crate) fn SavePanel(
    is_3d: Memo<bool>,
    save_format: RwSignal<SaveFormat>,
    effective_save_format: Memo<SaveFormat>,
    png_width: RwSignal<u32>,
    png_height: RwSignal<u32>,
    anim_fps: RwSignal<u16>,
    anim_duration_secs: RwSignal<f32>,
    anim_num_frames: Memo<u32>,
    anim_progress: RwSignal<Option<(u32, u32)>>,
    anim_exporting: RwSignal<bool>,
    export_error: RwSignal<Option<String>>,
    auto_rotate: RwSignal<bool>,
    auto_rotate_speed: RwSignal<f32>,
    hue_rotation: RwSignal<HueRotation>,
    hue_rotation_phase: StoredValue<f32>,
    renderer: RendererState,
    config_for_render: Callback<(), Config>,
) -> impl IntoView {
    view! {
        <crate::ui::Disclosure title="Save image" open=false>
            {move || if is_3d.get() {
                view! {
                    <crate::ui::SegmentedToggle
                        options=vec![(SaveFormat::Png, "PNG"), (SaveFormat::Apng, "APNG")]
                        selected=Signal::derive(move || effective_save_format.get())
                        on_change=move |key| {
                            save_format.set(key);
                            export_error.set(None);
                        }
                    />
                }.into_any()
            } else {
                view! {
                    <crate::ui::SegmentedToggle
                        options=vec![
                            (SaveFormat::Svg, "SVG"),
                            (SaveFormat::Png, "PNG"),
                            (SaveFormat::Apng, "APNG"),
                        ]
                        selected=Signal::derive(move || effective_save_format.get())
                        on_change=move |key| {
                            save_format.set(key);
                            export_error.set(None);
                        }
                    />
                }.into_any()
            }}

            <Show when=move || effective_save_format.get() != SaveFormat::Svg>
                <div class="spinner-row">
                    <span class="spinner-label">"Width (px)"</span>
                    <crate::ui::Spinner
                        value=Signal::derive(move || png_width.get().to_string())
                        step=16.0
                        on_commit=move |s| {
                            if let Ok(v) = s.parse::<u32>() {
                                png_width.set(v.clamp(PNG_MIN_DIMENSION, PNG_MAX_DIMENSION));
                            }
                        }
                    />
                </div>
                <div class="spinner-row">
                    <span class="spinner-label">"Height (px)"</span>
                    <crate::ui::Spinner
                        value=Signal::derive(move || png_height.get().to_string())
                        step=16.0
                        on_commit=move |s| {
                            if let Ok(v) = s.parse::<u32>() {
                                png_height.set(v.clamp(PNG_MIN_DIMENSION, PNG_MAX_DIMENSION));
                            }
                        }
                    />
                </div>
            </Show>

            <Show when=move || effective_save_format.get() == SaveFormat::Apng>
                <div class="spinner-row">
                    <span class="spinner-label">"FPS"</span>
                    <select
                        on:change=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<u16>() {
                                anim_fps.set(v);
                            }
                        }
                        prop:value=move || anim_fps.get().to_string()
                    >
                        <option value="12">"12"</option>
                        <option value="24">"24"</option>
                        <option value="30">"30"</option>
                        <option value="60">"60"</option>
                    </select>
                </div>

                <div class="spinner-row">
                    <span class="spinner-label">"Duration (s)"</span>
                    <crate::ui::Spinner
                        value=Signal::derive(move || format!("{:.1}", anim_duration_secs.get()))
                        step=1.0
                        on_commit=move |s| {
                            if let Ok(v) = s.parse::<f32>() {
                                anim_duration_secs.set(v.max(0.1));
                            }
                        }
                    />
                </div>

                <Show when=move || hue_rotation.with(|m| m.is_enabled())>
                    {move || {
                        let speed = hue_rotation.with(|m| m.speed_degrees_per_second());
                        let loop_secs = 360.0 / speed;
                        view! {
                            <button
                                type="button"
                                on:click=move |_| anim_duration_secs.set(loop_secs)
                            >
                                {format!("Hue loop ({loop_secs:.1}s)")}
                            </button>
                        }
                    }}
                </Show>

                <Show when=move || auto_rotate.get() && is_3d.get()>
                    {move || {
                        let speed = auto_rotate_speed.get();
                        let loop_secs = 360.0 / speed;
                        view! {
                            <button
                                type="button"
                                on:click=move |_| anim_duration_secs.set(loop_secs)
                            >
                                {format!("Orbit loop ({loop_secs:.1}s)")}
                            </button>
                        }
                    }}
                </Show>

                <span class="section-label" style="color:var(--color-muted)">
                    {move || format!("{} frames", anim_num_frames.get())}
                </span>

                {move || anim_progress.get().map(|(n, total)| view! {
                    <span class="inline-status">{format!("Exporting frame {n} / {total}…")}</span>
                })}
            </Show>

            <Show when=move || effective_save_format.get() == SaveFormat::Apng && (anim_num_frames.get() > AnimationParams::MAX_FRAMES)>
                <span class="inline-status warning">
                    {move || format!(
                        "Duration produces {} frames; max is {}.",
                        anim_num_frames.get(),
                        AnimationParams::MAX_FRAMES,
                    )}
                </span>
            </Show>

            <button
                type="button"
                disabled=move || effective_save_format.get() == SaveFormat::Apng && (anim_exporting.get() || anim_num_frames.get() > AnimationParams::MAX_FRAMES)
                on:click=move |_| {
                    export_error.set(None);
                    let fmt = effective_save_format.get_untracked();
                    let config = config_for_render.run(());
                    match fmt {
                        SaveFormat::Svg => {
                            export_svg(config);
                        }
                        SaveFormat::Png => {
                            let Some(Some((device, queue, camera))) =
                                renderer.try_with_value(|opt| {
                                    opt.as_ref().map(|r| {
                                        let (d, q) = r.device_queue();
                                        (d, q, r.camera())
                                    })
                                })
                            else {
                                export_error.set(Some("Cannot save: GPU renderer not ready.".to_string()));
                                return;
                            };
                            export_png(
                                device,
                                queue,
                                camera,
                                config,
                                png_width.get_untracked(),
                                png_height.get_untracked(),
                                move |e| export_error.set(Some(e)),
                            );
                        }
                        SaveFormat::Apng => {
                            let Some(Some((device, queue, camera))) =
                                renderer.try_with_value(|opt| {
                                    opt.as_ref().map(|r| {
                                        let (d, q) = r.device_queue();
                                        (d, q, r.camera())
                                    })
                                })
                            else {
                                export_error.set(Some("Cannot save: GPU renderer not ready.".to_string()));
                                return;
                            };
                            let fps = anim_fps.get_untracked();
                            let num_frames = anim_num_frames.get_untracked();
                            let initial_hue = hue_rotation_phase.get_value();
                            let hue_rotation_dps = hue_rotation.with_untracked(|m| {
                                if m.is_enabled() {
                                    let sign = if m.direction() == HueRotationDirection::Forward { 1.0f32 } else { -1.0 };
                                    sign * m.speed_degrees_per_second()
                                } else {
                                    0.0
                                }
                            });
                            let auto_rotate_dps = if auto_rotate.get_untracked() && is_3d.get() {
                                auto_rotate_speed.get_untracked()
                            } else {
                                0.0
                            };
                            let params = AnimationParams {
                                fps,
                                num_frames,
                                initial_hue_phase_degrees: initial_hue,
                                hue_rotation_dps,
                                auto_rotate_dps,
                            };
                            let width = png_width.get_untracked();
                            let height = png_height.get_untracked();
                            anim_exporting.set(true);
                            anim_progress.set(None);
                            crate::export::export_animation(
                                device,
                                queue,
                                camera,
                                config,
                                width,
                                height,
                                params,
                                move |n, total| anim_progress.set(Some((n, total))),
                                move |err| {
                                    anim_exporting.set(false);
                                    anim_progress.set(None);
                                    if let Some(msg) = err {
                                        export_error.set(Some(msg));
                                    }
                                },
                            );
                        }
                    }
                }
            >"Save"</button>
            {move || export_error.get().map(|m| view! {
                <span class="inline-status error">{m}</span>
            })}
        </crate::ui::Disclosure>
    }
}
