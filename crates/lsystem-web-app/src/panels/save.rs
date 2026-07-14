use crate::app::{ConfigContext, RenderContext};
use crate::export::{export_animation, export_png, export_svg};
use leptos::prelude::*;
use lsystem_app_model::HueRotationDirection;
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
pub(crate) fn SavePanel() -> impl IntoView {
    let ConfigContext { is_3d, .. } = expect_context();
    let RenderContext {
        renderer,
        auto_rotate,
        auto_rotate_speed,
        hue_rotation,
        hue_rotation_phase,
        config_for_render,
        ..
    } = expect_context();

    let save_format = RwSignal::new(SaveFormat::Png);
    let effective_save_format = Memo::new(move |_| {
        let fmt = save_format.get();
        if is_3d.get() && fmt == SaveFormat::Svg {
            SaveFormat::Png
        } else {
            fmt
        }
    });
    let png_width = RwSignal::new(800u32);
    let png_height = RwSignal::new(800u32);
    let anim_fps: RwSignal<u16> = RwSignal::new(30);
    let anim_duration_secs: RwSignal<f32> = RwSignal::new(4.0);
    let anim_num_frames =
        Memo::new(move |_| (anim_duration_secs.get() * anim_fps.get() as f32).round() as u32);
    let anim_progress: RwSignal<Option<(u32, u32)>> = RwSignal::new(None);

    // The whole export runs as one Action: `pending()` replaces a hand-rolled
    // "exporting" flag and `value()` carries the outcome of the last export.
    let export_action = Action::new_unsync(move |&(): &()| {
        let fmt = effective_save_format.get_untracked();
        let config = config_for_render.run(());
        let width = png_width.get_untracked();
        let height = png_height.get_untracked();
        // None while the GPU renderer is still initializing.
        let gpu = renderer
            .try_with_value(|opt| {
                opt.as_ref().map(|r| {
                    let (device, queue) = r.device_queue();
                    (device, queue, r.camera())
                })
            })
            .flatten();
        let hue_rotation_dps = hue_rotation.with_untracked(|m| {
            if m.is_enabled() {
                let sign = if m.direction() == HueRotationDirection::Forward {
                    1.0f32
                } else {
                    -1.0
                };
                sign * m.speed_degrees_per_second()
            } else {
                0.0
            }
        });
        let auto_rotate_dps = if auto_rotate.get_untracked() && is_3d.get_untracked() {
            auto_rotate_speed.get_untracked()
        } else {
            0.0
        };
        let params = AnimationParams {
            fps: anim_fps.get_untracked(),
            num_frames: anim_num_frames.get_untracked(),
            initial_hue_phase_degrees: hue_rotation_phase.get_value(),
            hue_rotation_dps,
            auto_rotate_dps,
        };
        async move {
            match fmt {
                SaveFormat::Svg => export_svg(config),
                SaveFormat::Png | SaveFormat::Apng => {
                    let Some((device, queue, camera)) = gpu else {
                        return Err("Cannot save: GPU renderer not ready.".to_string());
                    };
                    if fmt == SaveFormat::Png {
                        export_png(device, queue, camera, config, width, height).await
                    } else {
                        let result = export_animation(
                            device,
                            queue,
                            camera,
                            config,
                            width,
                            height,
                            params,
                            move |n, total| anim_progress.set(Some((n, total))),
                        )
                        .await;
                        anim_progress.set(None);
                        result
                    }
                }
            }
        }
    });
    let export_pending = export_action.pending();
    let export_error = move || export_action.value().get().and_then(Result::err);
    let clear_export_error = move || export_action.value().set(None);

    view! {
        <crate::ui::Disclosure title="Save image" open=false>
            {move || if is_3d.get() {
                view! {
                    <crate::ui::SegmentedToggle
                        options=vec![(SaveFormat::Png, "PNG"), (SaveFormat::Apng, "APNG")]
                        selected=Signal::derive(move || effective_save_format.get())
                        on_change=move |key| {
                            save_format.set(key);
                            clear_export_error();
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
                            clear_export_error();
                        }
                    />
                }.into_any()
            }}

            <Show when=move || effective_save_format.get() != SaveFormat::Svg>
                <div class="spinner-row">
                    <span class="spinner-label">"Width (px)"</span>
                    <crate::ui::Spinner
                        value=png_width
                        step=16.0
                        min=PNG_MIN_DIMENSION
                        max=PNG_MAX_DIMENSION
                        on_commit=move |v: u32| png_width.set(v)
                    />
                </div>
                <div class="spinner-row">
                    <span class="spinner-label">"Height (px)"</span>
                    <crate::ui::Spinner
                        value=png_height
                        step=16.0
                        min=PNG_MIN_DIMENSION
                        max=PNG_MAX_DIMENSION
                        on_commit=move |v: u32| png_height.set(v)
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
                        value=anim_duration_secs
                        step=1.0
                        min=0.1_f32
                        decimals=1
                        on_commit=move |v: f32| anim_duration_secs.set(v)
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
                disabled=move || effective_save_format.get() == SaveFormat::Apng && (export_pending.get() || anim_num_frames.get() > AnimationParams::MAX_FRAMES)
                on:click=move |_| {
                    clear_export_error();
                    export_action.dispatch(());
                }
            >"Save"</button>
            {move || export_error().map(|m| view! {
                <span class="inline-status error">{m}</span>
            })}
        </crate::ui::Disclosure>
    }
}
