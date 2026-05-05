// To add a 3D interpreter: add turtle/turtle3d.rs with Segments3D<I> analogous
// to Segments2D, then dispatch in lib.rs::generate based on cfg.dimensions.
// The `dimensions` field in Config is already parsed and validated.
pub(crate) mod turtle2d;
