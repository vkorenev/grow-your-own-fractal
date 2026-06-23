# Shader Tooling Direction

## Goals

The renderer currently relies on manually synchronized Rust and WGSL definitions. The main goal is to make that relationship safer at compile time while reducing repeated shader code between the 2D and 3D render paths.

Target outcomes:

- Validate shader source during Rust builds instead of discovering WGSL errors at runtime.
- Generate or validate Rust representations of WGSL structs used for uniforms, storage buffers, and vertex inputs.
- Generate bind group and pipeline-layout boilerplate where practical so binding changes fail at compile time.
- Reduce duplicated WGSL logic shared by `shader.wgsl` and `shader3d.wgsl`.
- Keep the existing renderer architecture recognizable: separate 2D and 3D line pipelines, explicit wgpu resource ownership, native and wasm support.

## Current Problems

### Manual Rust/WGSL Struct Sync

The Rust structs in `crates/lsystem-renderer/src/line_renderer.rs` mirror WGSL structs and entry-point inputs in `shader.wgsl` and `shader3d.wgsl`.

Current examples:

- `Transform` mirrors the 2D uniform struct in `shader.wgsl`.
- `Mvp` mirrors the 3D uniform struct in `shader3d.wgsl`.
- `ColorParams` mirrors an identical WGSL struct in both shader files.
- `Segment2D`, `Segment3D`, and the topological-depth variants must match vertex `@location` inputs and `vertex_attr_array!` declarations.

`encase` ensures the Rust uniform types follow WGSL host-shareable layout rules, and `bytemuck` ensures vertex instance types are byte-castable. Neither tool proves that the WGSL side has the same field names, field order, bindings, or vertex attribute expectations.

### Manual Bind Group and Layout Sync

Both line pipelines manually define bind group layouts with:

- binding 0: per-dimension transform uniform (`Transform` or `Mvp`)
- binding 1: shared `ColorParams` uniform

The WGSL declarations and Rust `BindGroupLayoutEntry` values must remain aligned manually. A binding index, visibility, or buffer type mismatch is currently more likely to appear as a wgpu validation error than a Rust compile error.

### Shader Duplication

`shader.wgsl` and `shader3d.wgsl` duplicate most color logic:

- `ColorParams`
- `hsv_to_rgb`
- `VertexOutput`
- `color_for_traversal`
- `color_for_depth`
- normal/depth color dispatch
- fragment shader

The meaningful differences are the transform uniform, 2D vs 3D vertex input type, and clip-position calculation. Color behavior changes must currently be edited in two files.

## Tool Overview

### encase

`encase` serializes Rust values into buffers using WGSL host-shareable layout rules. It is useful for uniforms and storage buffers because it handles padding and alignment according to shader layout rules instead of relying only on `repr(C)` and raw byte casts.

Current use:

- `lsystem-renderer` writes `ColorParams`, `Transform`, and `Mvp` uniform buffers through those structs' `encase::ShaderType` implementations.
- The public renderer uniform structs use `glam` vector/matrix types and let `encase` apply WGSL padding.
- Segment instance buffers still use `bytemuck` and explicit `VertexBufferLayout` declarations because vertex attributes are governed by wgpu's vertex input layout, not WGSL uniform layout.

Limits:

- It does not parse our WGSL files.
- It does not prove that `ColorParams` in Rust is the same as `ColorParams` in WGSL.
- It is not the main solution for vertex buffer layouts, which are controlled by `VertexBufferLayout` and entry-point `@location` inputs.

Source: <https://docs.rs/encase/latest/encase/>

### wgsl_to_wgpu

`wgsl_to_wgpu` generates Rust bindings from WGSL during a build script. It can generate Rust structs for shader types, shader-module helpers, bind group helpers, pipeline-layout helpers, entry-point helpers, vertex-buffer-layout helpers, constants, and optional `encase` or `bytemuck` derives.

Useful for us:

- WGSL can become the source of truth for host-shared types.
- Changes to uniforms, bind groups, or shader inputs can produce Rust compile errors.
- Current releases use `naga ^29` and `wgpu-types ^29`, matching this workspace's wgpu generation.
- It supports `derive_encase_host_shareable`, which pairs well with uniform-buffer serialization.
- It generates `VERTEX_ATTRIBUTES`, `vertex_buffer_layout`, `VertexEntry`, and `vertex_state` helpers for vertex inputs.

Limits:

- It does not provide shader imports or a module system directly.
- If shader code is split with WESL or another preprocessor, the final processed WGSL must be fed into `wgsl_to_wgpu`.
- It assumes static buffer offsets for generated buffer bindings; dynamic-offset cases need handwritten layout/bind group code.
- For separate primitive `@location` vertex parameters, it generates one vertex buffer layout per parameter. That does not match the renderer's current single interleaved instance buffers unless the shader inputs are refactored into WGSL input structs or the renderer switches to multiple vertex buffers.
- It may need manual overrides or handwritten code for performance-sensitive cases.

Source: <https://docs.rs/crate/wgsl_to_wgpu/latest>

### wgsl-bindgen

`wgsl-bindgen` also generates type-safe Rust bindings from WGSL for wgpu. It is powered by `naga-oil`, supports shader imports, and generates Rust types, constants, bind group helpers, vertex attribute setup, and pipeline layout helpers.

Useful for us:

- One tool can address shader imports, generated host types, bind groups, pipeline layouts, and vertex attribute helpers.
- It supports `encase`, `bytemuck`, `serde`, and custom type maps.
- Current releases use `naga ^29`, `naga_oil ^0.22`, and `wgpu-types ^29`, which fits the current workspace better than older tools.

Limits:

- It is a broader and more opinionated codegen surface than `wgsl_to_wgpu`.
- Adopting all generated helpers may require more changes to `line_renderer.rs`.
- Like `wgsl_to_wgpu`, primitive `@location` vertex parameters generate separate vertex-buffer layouts rather than the current interleaved instance-buffer layout. WGSL input structs or handwritten vertex layouts are needed if the current buffer shape is preserved.
- Its docs are less complete than `wgsl_to_wgpu`, so a small prototype should verify generated API shape before committing to it.

Source: <https://docs.rs/wgsl_bindgen/latest/wgsl_bindgen/>

### WESL

WESL is a strict superset of WGSL that adds shader-language features such as imports, conditional compilation, and packages. Existing WGSL is valid WESL. The Rust `wesl` crate can compile WESL to WGSL in `build.rs`.

Useful for us:

- Split shared color code into a reusable shader module.
- Keep 2D and 3D entry points separate while importing common definitions/functions.
- Potentially use conditional compilation for future shader variants.

Limits:

- WESL does not generate Rust bindings.
- It only solves shader-to-shader duplication unless paired with `wgsl_to_wgpu` or another binding generator.
- It adds a second build-time shader step if paired with `wgsl_to_wgpu`.
- `wesl 0.4.0` uses its own WGSL parser (`wgsl-parse`), not naga directly. The optional `naga-ext` feature enables naga-specific AST extensions but is not required for basic WESL→WGSL compilation. Naga validation of the emitted WGSL only occurs when `wgsl_to_wgpu` processes the output, not during the WESL step itself.

Sources:

- <https://wesl-lang.dev/>
- <https://docs.rs/wesl/latest/wesl/>

### include-wgsl-oil

`include-wgsl-oil` is an attribute macro that runs the `naga-oil` preprocessor at Rust compile time and exposes shader types, constants, globals, and processed source to Rust. It also supports an `@export` convention for WGSL structs and can derive `encase::ShaderType` on exported structs.

Useful for us:

- Combines compile-time shader preprocessing with Rust-visible shader facts.
- Can expose WGSL structs as Rust structs.
- Supports `naga-oil` imports.

Limits:

- Latest published version is `0.2.9` from April 2025 and depends on older `naga ^24` and `naga_oil ^0.17`.
- It does not appear to generate the same level of bind group and pipeline-layout helper code as `wgsl_to_wgpu` or `wgsl-bindgen`.
- `@export` is specific to `include-wgsl-oil`, not general WGSL or general `naga-oil` syntax.
- Given the older dependency stack and weaker bind-group story, it should be treated as background context rather than the preferred implementation path.

Source: <https://docs.rs/crate/include-wgsl-oil/latest>

## Proposed Solutions

### Phase 0: Naga-Only Build Validation

Before adopting any codegen tool, a cheap first step is to add a `build.rs` that parses both WGSL files with `naga` and fails the build on errors. No generated Rust, no tooling lock-in, no changes to `line_renderer.rs`.

```toml
# lsystem-renderer/Cargo.toml
[build-dependencies]
naga = { version = "29", features = ["wgsl-in"] }
```

```rust
// lsystem-renderer/build.rs
fn main() {
    for shader in ["src/shader.wgsl", "src/shader3d.wgsl"] {
        println!("cargo:rerun-if-changed={shader}");
        let src = std::fs::read_to_string(shader).unwrap();
        naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{shader}: {e}"));
    }
}
```

This delivers the first acceptance criterion (WGSL errors discovered during Rust builds) immediately and can remain in place even after adopting `wgsl-bindgen`. The struct/binding sync problems remain; WGSL typos and type errors become compile failures.

### Initial Recommended Path: Prototype wgsl-bindgen

The initial recommendation was to start with `wgsl-bindgen` because it addresses both main goals with one tool:

- Shader imports for deduplicating common WGSL color code.
- Generated Rust structs for shared uniforms and vertex inputs.
- Generated bind group helpers.
- Generated pipeline layout helpers.
- Generated vertex attribute setup, where the generated buffer layout matches the renderer's buffer shape.
- Optional `encase` serialization for host-shareable types.

Proposed shader structure:

- Move shared color declarations and functions into a common shader module.
- Keep distinct 2D and 3D entry-point files because projection and vertex input dimensions differ.
- Keep `ColorParams` defined once in the shared module if the generator can expose imported structs cleanly; otherwise define it in the entry module and avoid duplicating functions first.

Proposed Rust integration:

- Add a `build.rs` for `lsystem-renderer` that generates bindings from the shader entry points.
- Prefer generated files under `OUT_DIR` and include them from a small handwritten Rust module, unless the generator requires checked-in source output for stable ergonomics.
- Replace handwritten bind group layout creation with generated layout helpers if the generated API fits the current pipeline setup.
- Keep handwritten `vertex_attr_array!` declarations unless the WGSL vertex inputs are first refactored to structs matching the renderer's interleaved instance-buffer records.
- Use `encase` for uniform writes once generated host-shared structs derive `ShaderType`.
- Keep handwritten resource ownership, upload scheduling, and draw calls in `LinePipeline2D` and `LinePipeline3D`.

Acceptance criteria for the prototype:

- A shader change to `ColorParams` causes Rust compilation to fail until Rust-side initialization is updated.
- A shader binding index/type change causes Rust compilation to fail until bind group setup is updated.
- Shared color logic is no longer duplicated between 2D and 3D shader entry points.
- Native `cargo check` and wasm clippy still work with generated bindings.

### Alternative: WESL + wgsl_to_wgpu

Use WESL for shader composition and `wgsl_to_wgpu` for generated Rust bindings.

This is attractive if `wgsl-bindgen` proves too invasive or its generated API does not fit the current renderer. The separation is clean:

- WESL handles imports and duplicated shader code.
- `wgsl_to_wgpu` handles Rust structs, bind groups, layouts, entry-point helpers, and optional `encase` derives from the final WGSL output.

Tradeoff: the build pipeline has two shader tooling steps instead of one.

### Not Recommended: include-wgsl-oil

Do not choose `include-wgsl-oil` for this project unless the more current generators fail a critical requirement.

It is useful historical and conceptual context, but its current dependency stack is behind the project's `wgpu`/`naga` generation, and it does not appear to cover generated bind group and pipeline-layout helpers as directly as `wgsl-bindgen` or `wgsl_to_wgpu`.

## Implementation Notes for a Future Change

- Start with the smallest shader pair: the existing 2D and 3D line shaders.
- Preserve the current public renderer API unless generated types force a narrow internal rename.
- Avoid checking generated code into the repo unless needed for reviewability or tool limitations.
- Keep fallback validation simple: build failure is the main safety mechanism.
- Update `AGENTS.md` if shader build workflow or common commands change.
- Update CI only if new build dependencies require explicit commands beyond existing `cargo check`, `cargo clippy`, and `trunk build` paths.

## Open Questions

- Does `wgsl-bindgen` generate clean bindings when common structs/functions are imported through `naga-oil` modules?
- ~~Can generated vertex-layout helpers model the current instance-buffer records exactly, including topological-depth variants?~~ **Answered: not with the current primitive `@location` parameters.** Both `wgsl-bindgen` and `wgsl_to_wgpu` generate per-entry helpers, but primitive parameters produce separate vertex-buffer layouts. Directly matching the current interleaved `Segment2D`, `Segment3D`, and topological-depth records requires WGSL input structs or continued handwritten vertex layouts.
- Is generated bind group code ergonomic enough for both 2D and 3D pipelines, or should we use generated constants/types while keeping some handwritten wgpu setup?
- Should generated shader bindings live in `OUT_DIR` or be checked in for easier review?

---

*The following section was added after a detailed comparison of both approaches against this project's specific shader structure.*

## Tool Comparison: wgsl-bindgen vs. WESL + wgsl_to_wgpu

### Generation scope

| Capability | wgsl-bindgen | wgsl_to_wgpu |
|---|---|---|
| Rust structs for uniforms | ✓ | ✓ |
| Bind group helpers | ✓ | ✓ |
| Pipeline layout helpers | ✓ | ✓ |
| Vertex attribute / buffer-layout helpers | ✓ (per entry point) | ✓ (per entry point) |
| Pipeline creation functions | partial helpers | partial helpers |
| Shader imports/modules | ✓ (naga-oil) | ✗ (needs WESL) |
| encase / bytemuck derives | ✓ | ✓ |

Both tools generate vertex-buffer-layout helpers. The important limitation for this project is not whether helpers exist, but whether the generated buffer shape matches the renderer's current buffers.

### Current vertex input shape

Each shader in this project has two vertex entry points with different vertex
input structs:

- `vs_main`: `Segment2D` or `Segment3D` with `@location(0) start` and
  `@location(1) end`
- `vs_depth_main`: the matching topological-depth segment struct with
  `@location(0) start`, `@location(1) end`, and
  `@location(2) topological_depth`

Both `wgsl-bindgen` and `wgsl_to_wgpu` generate per-entry-point helpers, and
both can represent multiple vertex entry points in one shader module. The
current renderer keeps the interleaved instance-buffer ownership handwritten:

- `line_renderer.rs` currently builds one `VertexBufferLayout` for `Segment2D`, `Segment3D`, and their topological-depth variants.
- The WGSL entry points now take input structs matching the current interleaved
  Rust instance-buffer records, so future generator prototypes can evaluate
  their vertex-layout helpers without first changing renderer behavior.

Generated vertex helpers are still optional; the existing handwritten
`vertex_attr_array!` declarations remain the active renderer implementation.

### WESL's validation gap

`wesl 0.4.0` uses its own WGSL parser (`wgsl-parse`), not naga. Naga validation of the emitted WGSL only fires when `wgsl_to_wgpu` processes the output — not during the WESL compilation step itself. This is a slightly looser feedback loop than `wgsl-bindgen`, but not a meaningful practical difference in a `build.rs` context.

### Updated recommendation

The initial recommendation above favors `wgsl-bindgen` on the basis that it addresses both goals with one tool. A closer comparison no longer supports rejecting it because of a two-entry-point risk: current `wgsl-bindgen` supports multiple vertex entries, and `wgsl_to_wgpu` also generates vertex helpers.

The real decision is migration style:

**Prototype `wgsl-bindgen` if a single shader tool is preferred.** It can cover naga-oil imports, generated host types, bind groups, pipeline layouts, shader-module helpers, and vertex helpers in one build step. The tradeoff is a broader generated API surface and a more opinionated integration shape.

**Prefer WESL + `wgsl_to_wgpu` if staged migration and explicit renderer ownership matter more.** WESL can first remove duplicated shader code with little or no Rust change. `wgsl_to_wgpu` can then generate host-shared structs, bind groups, pipeline layouts, shader-module helpers, and vertex reference helpers from the processed WGSL. This keeps shader composition and Rust binding generation as separate choices.

**Do not make generated vertex layouts an acceptance criterion for either path yet.** Vertex input structs now model `Segment2D`, `Segment3D`, and the depth variants, but the renderer still intentionally uses handwritten `vertex_attr_array!` declarations until a generator prototype proves that replacing them is worthwhile.

**Revised recommendation:** start with the lowest-risk split that matches the desired migration. Choose `wgsl-bindgen` for a one-tool prototype; choose WESL + `wgsl_to_wgpu` for a more incremental path. In both cases, treat vertex-layout generation as optional until a prototype proves that generated layouts match the renderer's needs cleanly.
