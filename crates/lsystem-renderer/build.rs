fn main() -> Result<(), Box<dyn std::error::Error>> {
    let shaders_dir = "src/shaders";
    println!("cargo:rerun-if-changed={shaders_dir}");

    let mut compiler = wesl::Wesl::new(shaders_dir);
    compiler.set_mangler(wesl::ManglerKind::None);

    let options = wgsl_to_wgpu::WriteOptions {
        derive_bytemuck_vertex: true,
        derive_encase_host_shareable: true,
        matrix_vector_types: wgsl_to_wgpu::MatrixVectorTypes::Glam,
        validate: Some(Default::default()),
        ..Default::default()
    };

    let out_dir = std::env::var("OUT_DIR")?;
    let out_dir = std::path::Path::new(&out_dir);

    for (module, name) in [
        ("package::shader_2d", "shader_2d"),
        ("package::shader_3d", "shader_3d"),
    ] {
        let wgsl = compiler.compile(&module.parse()?)?.to_string();

        let wgsl_path = out_dir.join(format!("{name}.wgsl"));
        std::fs::write(&wgsl_path, &wgsl)?;

        let text =
            wgsl_to_wgpu::create_shader_modules(&wgsl, options, wgsl_to_wgpu::demangle_identity)
                .inspect_err(|error| error.emit_to_stderr_with_path(&wgsl, &wgsl_path))
                .map_err(|_| format!("failed to generate WGSL bindings for `{name}`"))?;

        std::fs::write(out_dir.join(format!("{name}_bindings.rs")), text)?;
    }

    Ok(())
}
