fn main() -> Result<(), Box<dyn std::error::Error>> {
    let wgsl_file = "src/shader.wgsl";
    println!("cargo:rerun-if-changed={wgsl_file}");

    let wgsl_source = std::fs::read_to_string(wgsl_file)?;

    let options = wgsl_to_wgpu::WriteOptions {
        derive_bytemuck_vertex: true,
        derive_encase_host_shareable: true,
        matrix_vector_types: wgsl_to_wgpu::MatrixVectorTypes::Glam,
        validate: Some(Default::default()),
        ..Default::default()
    };

    let text =
        wgsl_to_wgpu::create_shader_modules(&wgsl_source, options, wgsl_to_wgpu::demangle_identity)
            .inspect_err(|error| error.emit_to_stderr_with_path(&wgsl_source, wgsl_file))
            .map_err(|_| "failed to generate WGSL bindings")?;

    let out_dir = std::env::var("OUT_DIR")?;
    std::fs::write(
        std::path::Path::new(&out_dir).join("shader_bindings.rs"),
        text,
    )?;
    Ok(())
}
