//! Fixed spherical direction/triangle table for 3D template bounds summaries.
//!
//! Samples a Class-I geodesic octahedron: the octahedron `±X, ±Y, ±Z`, each
//! of its 8 faces subdivided into a barycentric grid of frequency
//! [`N_FREQ`], every grid vertex projected outward to the unit sphere. This
//! gives `TABLE_DIRECTION_COUNT = 4*N_FREQ^2 + 2` directions and `8*N_FREQ^2` small spherical
//! triangles. See `docs/plans/3d-spherical-support-table.md` ("The
//! direction table and O(1) location") for the derivation: why radial
//! (gnomonic) projection locates the exact containing triangle with no
//! approximation, and why the direction set must be antipodally symmetric
//! (every direction's negation is also a sampled direction — required by
//! the antipodal-reflection safety net in `template.rs`).
//!
//! This module is purely geometric and independent of any template: it maps
//! a unit direction to a spherical triangle plus the coefficients
//! expressing that direction as a nonnegative combination of the triangle's
//! three vertex directions, whenever that direction truly lies in the
//! triangle. Callers combine those coefficients with per-template support
//! values; this module knows nothing about templates.

use std::collections::HashMap;
use std::sync::LazyLock;

use glam::Vec3;

/// Barycentric subdivision frequency. Each octahedron face is subdivided
/// into `N_FREQ^2` small triangles. Tunable like `k` (`bounds.rs`) and `M`
/// (`SUPPORT_DIRECTION_COUNT_2D`); `5` gives 102 directions and 200
/// triangles, the closest clean value to the design doc's "about 98".
const N_FREQ: i32 = 5;

/// Total sampled directions: `4 * N_FREQ^2 + 2` (the Class-I octahedral
/// vertex count; matches Euler's formula `V - E + F = 2` with
/// `F = 8 * N_FREQ^2`, `E = 12 * N_FREQ^2`).
pub(crate) const TABLE_DIRECTION_COUNT: usize = 4 * (N_FREQ as usize) * (N_FREQ as usize) + 2;

/// A 3D vector of `f64` components, used for the construction and query math
/// that needs precision headroom beyond the stored `f32` directions.
type V3 = [f64; 3];

/// Sign-canonical dedup key, see [`signed_key`].
type DedupKey = (i32, i32, i32);

fn v3_from(v: Vec3) -> V3 {
    [f64::from(v.x), f64::from(v.y), f64::from(v.z)]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// One small spherical triangle: its three vertex direction indices (into
/// [`SphereTable::directions`]) and the precomputed inverse of the 3x3
/// matrix whose columns are those vertices' direction vectors, stored as its
/// three rows (`inverse_rows[r]` is row `r` of `U^-1`, so
/// `dot(inverse_rows[r], v)` is coefficient `r` in `v = l0*u0 + l1*u1 +
/// l2*u2`). Precomputing the inverse makes a query a fixed 3x3 matrix-vector
/// product, not a per-query linear solve.
struct Triangle {
    vertices: [usize; 3],
    inverse_rows: [V3; 3],
}

fn build_triangle(directions: &[Vec3; TABLE_DIRECTION_COUNT], vertices: [usize; 3]) -> Triangle {
    let u = vertices.map(|index| v3_from(directions[index]));
    // U^-1's rows are the reciprocal basis of columns u0,u1,u2: row0 =
    // cross(u1,u2)/det, row1 = cross(u2,u0)/det, row2 = cross(u0,u1)/det,
    // where det is their scalar triple product. Verified by dotting each
    // row against each u_i (docs/plans/3d-spherical-support-table.md).
    let det = dot(u[0], cross(u[1], u[2]));
    assert!(
        det.is_finite() && det.abs() > 1.0e-9,
        "triangle vertices {vertices:?} must be nonsingular (det = {det})"
    );
    let inverse_rows = [
        scale(cross(u[1], u[2]), 1.0 / det),
        scale(cross(u[2], u[0]), 1.0 / det),
        scale(cross(u[0], u[1]), 1.0 / det),
    ];
    Triangle {
        vertices,
        inverse_rows,
    }
}

/// The two small triangles covering barycentric cell `(i, j)` within one
/// octahedron face. `down` is `None` exactly when `i + j == N_FREQ - 1`
/// (the last valid row for this `i`): geometrically, `b_x + b_y + b_z == 1`
/// with every component `>= 0` forces `b_x + b_y <= 1`, so a point in a cell
/// on that row can never fall in the "down" half of the unit-square split
/// (`frac_x + frac_y > 1`) — `down` is therefore never dereferenced by a
/// correct locator. [`locate`]'s own tests confirm this rather than assume
/// it.
struct CellTriangles {
    up: Triangle,
    down: Option<Triangle>,
}

struct SphereTable {
    directions: [Vec3; TABLE_DIRECTION_COUNT],
    /// `antipode[i]` is the index of `-directions[i]`, also a sampled
    /// direction by construction (see module docs).
    antipode: [usize; TABLE_DIRECTION_COUNT],
    /// One entry per octant, ordered by the same 3-bit sign-index `locate`
    /// computes (`bit0 = x >= 0`, `bit1 = y >= 0`, `bit2 = z >= 0`). Each
    /// octant's cells are indexed `i * N_FREQ + j` for `i, j` in
    /// `0..N_FREQ`; `None` where `i + j > N_FREQ - 1`, which the locator's
    /// own clamp (`j <= N_FREQ - 1 - i`) never produces, so it is left
    /// unpopulated rather than filled with an invalid placeholder triangle.
    octants: [Vec<Option<CellTriangles>>; 8],
}

fn octant_signs(octant: usize) -> (i32, i32, i32) {
    let sx = if octant & 1 != 0 { 1 } else { -1 };
    let sy = if octant & 2 != 0 { 1 } else { -1 };
    let sz = if octant & 4 != 0 { 1 } else { -1 };
    (sx, sy, sz)
}

/// Sign-canonical dedup key for barycentric-index triple `(i, j, k)` in the
/// octant with signs `(sx, sy, sz)`: a zero index carries a fixed neutral
/// sign (`0`) regardless of the octant's sign, so grid vertices shared by
/// multiple octant faces (where an index is `0`) collide to the same key.
/// Two `(octant, i, j, k)` combinations produce the same 3D point if and
/// only if they produce the same key.
fn signed_key(i: i32, j: i32, k: i32, sx: i32, sy: i32, sz: i32) -> DedupKey {
    (
        if i == 0 { 0 } else { sx * i },
        if j == 0 { 0 } else { sy * j },
        if k == 0 { 0 } else { sz * k },
    )
}

fn direction_for(i: i32, j: i32, k: i32, sx: i32, sy: i32, sz: i32) -> Vec3 {
    let n = f64::from(N_FREQ);
    let p = [
        f64::from(sx) * f64::from(i) / n,
        f64::from(sy) * f64::from(j) / n,
        f64::from(sz) * f64::from(k) / n,
    ];
    let length = dot(p, p).sqrt();
    Vec3::new(
        (p[0] / length) as f32,
        (p[1] / length) as f32,
        (p[2] / length) as f32,
    )
}

/// Builds the deduplicated direction array and its antipodal pairing by
/// enumerating every `(octant, i, j, k)` barycentric grid vertex once. The
/// antipodal octant (`(-sx, -sy, -sz)`) with the same `(i, j, k)` always
/// generates the exact negated key (zero components stay zero; nonzero
/// components flip sign with the octant), so every direction's antipode key
/// is guaranteed to have been generated too.
fn build_directions_and_antipode() -> (
    Vec<Vec3>,
    [usize; TABLE_DIRECTION_COUNT],
    HashMap<DedupKey, usize>,
) {
    let mut directions: Vec<Vec3> = Vec::with_capacity(TABLE_DIRECTION_COUNT);
    let mut index_to_key: Vec<DedupKey> = Vec::with_capacity(TABLE_DIRECTION_COUNT);
    let mut key_to_index: HashMap<DedupKey, usize> = HashMap::with_capacity(TABLE_DIRECTION_COUNT);

    for octant in 0..8usize {
        let (sx, sy, sz) = octant_signs(octant);
        for i in 0..=N_FREQ {
            for j in 0..=(N_FREQ - i) {
                let k = N_FREQ - i - j;
                let key = signed_key(i, j, k, sx, sy, sz);
                key_to_index.entry(key).or_insert_with(|| {
                    directions.push(direction_for(i, j, k, sx, sy, sz));
                    index_to_key.push(key);
                    directions.len() - 1
                });
            }
        }
    }
    assert_eq!(
        directions.len(),
        TABLE_DIRECTION_COUNT,
        "class-I octahedral subdivision at n = {N_FREQ} must produce exactly TABLE_DIRECTION_COUNT = {TABLE_DIRECTION_COUNT} directions, got {}",
        directions.len()
    );

    let antipode: Vec<usize> = index_to_key
        .iter()
        .map(|&(kx, ky, kz)| {
            *key_to_index
                .get(&(-kx, -ky, -kz))
                .expect("every direction's negated key is generated by the antipodal octant")
        })
        .collect();
    let antipode: [usize; TABLE_DIRECTION_COUNT] = antipode
        .try_into()
        .unwrap_or_else(|v: Vec<usize>| panic!("length checked above, got {}", v.len()));

    (directions, antipode, key_to_index)
}

fn build_octant_cells(
    directions: &[Vec3; TABLE_DIRECTION_COUNT],
    key_to_index: &HashMap<DedupKey, usize>,
    octant: usize,
) -> Vec<Option<CellTriangles>> {
    let (sx, sy, sz) = octant_signs(octant);
    let index_of = |i: i32, j: i32, k: i32| -> usize {
        *key_to_index
            .get(&signed_key(i, j, k, sx, sy, sz))
            .expect("every barycentric grid vertex used by a triangle must have been generated")
    };

    let mut cells = Vec::with_capacity((N_FREQ * N_FREQ) as usize);
    for i in 0..N_FREQ {
        for j in 0..N_FREQ {
            if i + j > N_FREQ - 1 {
                cells.push(None);
                continue;
            }
            let k = N_FREQ - i - j;
            let up = build_triangle(
                directions,
                [
                    index_of(i, j, k),
                    index_of(i + 1, j, k - 1),
                    index_of(i, j + 1, k - 1),
                ],
            );
            let down = (i + j < N_FREQ - 1).then(|| {
                build_triangle(
                    directions,
                    [
                        index_of(i + 1, j, k - 1),
                        index_of(i, j + 1, k - 1),
                        index_of(i + 1, j + 1, k - 2),
                    ],
                )
            });
            cells.push(Some(CellTriangles { up, down }));
        }
    }
    cells
}

fn build_table() -> SphereTable {
    let (directions, antipode, key_to_index) = build_directions_and_antipode();
    let directions: [Vec3; TABLE_DIRECTION_COUNT] = directions
        .try_into()
        .unwrap_or_else(|v: Vec<Vec3>| panic!("length checked above, got {}", v.len()));
    let octants =
        std::array::from_fn(|octant| build_octant_cells(&directions, &key_to_index, octant));
    SphereTable {
        directions,
        antipode,
        octants,
    }
}

static TABLE: LazyLock<SphereTable> = LazyLock::new(build_table);

/// The `TABLE_DIRECTION_COUNT` sampled unit directions.
pub(crate) fn directions() -> &'static [Vec3; TABLE_DIRECTION_COUNT] {
    &LazyLock::force(&TABLE).directions
}

/// `antipode()[i]` is the index of `-directions()[i]`.
pub(crate) fn antipode() -> &'static [usize; TABLE_DIRECTION_COUNT] {
    &LazyLock::force(&TABLE).antipode
}

/// Locates the spherical triangle containing `direction` (assumed close to
/// unit length; the algorithm is scale-invariant, see module docs) and
/// returns, for each of its three vertices, the coefficient `l` such that
/// `direction = sum(l_i * directions()[index_i])` — exactly, for the
/// vertices of the located triangle, regardless of whether `direction`
/// truly lies inside it (a coefficient can be negative if not; callers must
/// handle that, see `template.rs`'s antipodal-reflection combination).
///
/// O(1): octant selection, a radial (gnomonic) projection to the flat
/// octahedron face, and a barycentric-grid cell lookup, all arithmetic — no
/// search or hashing at query time.
pub(crate) fn locate(direction: Vec3) -> [(f64, usize); 3] {
    let v = v3_from(direction);
    debug_assert!(
        v[0].is_finite() && v[1].is_finite() && v[2].is_finite(),
        "locate() requires a finite direction"
    );

    let octant = usize::from(v[0] >= 0.0)
        | (usize::from(v[1] >= 0.0) << 1)
        | (usize::from(v[2] >= 0.0) << 2);
    let (fx, fy, fz) = (v[0].abs(), v[1].abs(), v[2].abs());
    let sum = fx + fy + fz;
    debug_assert!(sum > 0.0, "locate() requires a nonzero direction");
    let n = f64::from(N_FREQ);
    let (nx, ny) = (n * fx / sum, n * fy / sum);

    let i = (nx.floor() as i32).clamp(0, N_FREQ - 1);
    let j = (ny.floor() as i32).clamp(0, N_FREQ - 1 - i);
    let (frac_x, frac_y) = (nx - f64::from(i), ny - f64::from(j));
    let up = frac_x + frac_y <= 1.0;

    let table = LazyLock::force(&TABLE);
    let cell = table.octants[octant][(i * N_FREQ + j) as usize]
        .as_ref()
        .expect("the clamp above keeps (i, j) inside the populated triangular domain");
    // `down` is geometrically absent only where `i + j == N_FREQ - 1` (see
    // `CellTriangles`'s docs), which the exact-arithmetic version of the
    // `up` test above always satisfies. Floating-point rounding in `nx`/`ny`
    // can still land `frac_x + frac_y` a hair above 1.0 right at that
    // boundary row, computing `up == false` where geometry says `up ==
    // true`. This is a location *misclassification*, not a safety issue —
    // containment does not depend on picking the true triangle (see the
    // antipodal-reflection argument in `template.rs`) — so fall back to
    // `up`, the only triangle that legitimately exists in this cell,
    // instead of panicking.
    let triangle = if up {
        &cell.up
    } else {
        cell.down.as_ref().unwrap_or(&cell.up)
    };

    std::array::from_fn(|slot| (dot(triangle.inverse_rows[slot], v), triangle.vertices[slot]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOTAL_TRIANGLES: usize = 8 * (N_FREQ as usize) * (N_FREQ as usize);

    #[test]
    fn direction_count_matches_the_class_i_octahedral_formula() {
        assert_eq!(TABLE_DIRECTION_COUNT, 102);
        assert_eq!(directions().len(), TABLE_DIRECTION_COUNT);
    }

    #[test]
    fn directions_are_unit_length() {
        for direction in directions() {
            assert!(
                (direction.length_squared() - 1.0).abs() <= 2.0e-6,
                "{direction:?} is not unit length"
            );
        }
    }

    #[test]
    fn antipode_is_a_complete_involution_with_exact_negation() {
        let antipode = antipode();
        let directions = directions();
        for index in 0..TABLE_DIRECTION_COUNT {
            let partner = antipode[index];
            assert_eq!(
                antipode[partner], index,
                "antipode is not an involution at {index}"
            );
            assert_eq!(
                directions[partner], -directions[index],
                "antipode {partner} is not the exact negation of direction {index}"
            );
        }
    }

    #[test]
    fn populated_triangle_count_matches_8n_squared() {
        assert_eq!(TOTAL_TRIANGLES, 200);
        let table = LazyLock::force(&TABLE);
        let mut count = 0;
        for octant in &table.octants {
            for i in 0..N_FREQ {
                for j in 0..N_FREQ {
                    let Some(cell) = &octant[(i * N_FREQ + j) as usize] else {
                        continue;
                    };
                    count += 1; // `up` is always present on a populated cell.
                    match &cell.down {
                        Some(_) => {
                            count += 1;
                            assert!(
                                i + j < N_FREQ - 1,
                                "down present outside its expected domain at ({i}, {j})"
                            );
                        }
                        None => assert_eq!(
                            i + j,
                            N_FREQ - 1,
                            "down missing outside the last valid row at ({i}, {j})"
                        ),
                    }
                }
            }
        }
        assert_eq!(count, TOTAL_TRIANGLES);
    }

    #[test]
    fn every_populated_triangle_has_distinct_vertices_and_a_nonsingular_matrix() {
        let table = LazyLock::force(&TABLE);
        for octant in &table.octants {
            for cell in octant.iter().flatten() {
                for triangle in [Some(&cell.up), cell.down.as_ref()].into_iter().flatten() {
                    let [a, b, c] = triangle.vertices;
                    assert!(
                        a != b && b != c && a != c,
                        "triangle {:?} repeats a vertex",
                        triangle.vertices
                    );
                    for row in triangle.inverse_rows {
                        for component in row {
                            assert!(
                                component.is_finite(),
                                "triangle {:?} has a non-finite inverse",
                                triangle.vertices
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn locate_never_panics_or_produces_non_finite_coefficients() {
        for probe in crate::test_util::probe_directions_3d(2000) {
            for (coefficient, index) in locate(probe) {
                assert!(
                    coefficient.is_finite(),
                    "non-finite coefficient for {probe:?}"
                );
                assert!(index < TABLE_DIRECTION_COUNT);
            }
        }
    }

    #[test]
    fn located_triangle_coefficients_are_close_to_nonnegative() {
        // Tightness check, not a safety requirement: safety is proven
        // structurally by the antipodal-reflection argument in
        // `template.rs`, exercised directly by that module's own tests.
        // This test would catch a systematically broken locator that no
        // containment test can see (see plan Context).
        const EPSILON: f64 = 1.0e-6;
        for probe in crate::test_util::probe_directions_3d(2000) {
            for (coefficient, _) in locate(probe) {
                assert!(
                    coefficient >= -EPSILON,
                    "coefficient {coefficient} for {probe:?} is not close to nonnegative"
                );
            }
        }
    }

    #[test]
    fn locate_handles_octant_boundaries_and_base_vertices() {
        let tricky = [
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            -Vec3::X,
            -Vec3::Y,
            -Vec3::Z,
            Vec3::new(1.0, 1.0, 0.0).normalize(),
            Vec3::new(1.0, 0.0, 1.0).normalize(),
            Vec3::new(0.0, 1.0, 1.0).normalize(),
            Vec3::new(1.0, 1.0, 1.0).normalize(),
            Vec3::new(1.0e-6, 1.0, 1.0e-6).normalize(),
        ];
        for probe in tricky {
            for (coefficient, index) in locate(probe) {
                assert!(
                    coefficient.is_finite(),
                    "non-finite coefficient for {probe:?}"
                );
                assert!(index < TABLE_DIRECTION_COUNT);
            }
        }
    }
}
