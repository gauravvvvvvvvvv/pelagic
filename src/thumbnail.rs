//! Small deterministic software previews for library meshes and materials.
//! Used by the editor's Content Browser/Details thumbnails; independent of
//! scene lighting, water and renderer choice so a thumbnail identifies the
//! asset itself.
use crate::{
    assets::{Material, Mesh},
    math::Vec3,
    render::Frame,
};

/// Largest thumbnail edge accepted, in pixels.
pub const MAX_SIZE: usize = 256;

fn sphere() -> Mesh {
    let (rings, segments) = (16usize, 28usize);
    let mut vertices = vec![];
    for r in 0..=rings {
        let theta = std::f64::consts::PI * r as f64 / rings as f64;
        for s in 0..segments {
            let phi = std::f64::consts::TAU * s as f64 / segments as f64;
            vertices.push(Vec3::new(
                theta.sin() * phi.cos(),
                theta.sin() * phi.sin(),
                theta.cos(),
            ));
        }
    }
    let mut triangles = vec![];
    for r in 0..rings {
        for s in 0..segments {
            let a = r * segments + s;
            let b = r * segments + (s + 1) % segments;
            let c = a + segments;
            let d = b + segments;
            triangles.push([a, c, b]);
            triangles.push([b, c, d]);
        }
    }
    Mesh {
        vertices,
        triangles,
        source: "preview sphere".into(),
    }
}

/// Renders `mesh` (or a sphere for a material-only preview) with `material`
/// into a `width`×`height` sRGB frame, using 2× supersampling. `stretch`
/// scales mesh axes (for example an asset's proportions or a body's size).
pub fn render(
    mesh: Option<&Mesh>,
    stretch: [f64; 3],
    material: &Material,
    width: usize,
    height: usize,
) -> Frame {
    let width = width.clamp(8, MAX_SIZE);
    let height = height.clamp(8, MAX_SIZE);
    let smooth = mesh.is_none();
    let stretch = if smooth {
        [1.0; 3]
    } else {
        stretch.map(|v| {
            if v.is_finite() {
                v.clamp(0.02, 1e3)
            } else {
                1.0
            }
        })
    };
    let largest = stretch.iter().copied().fold(0.0, f64::max);
    let fallback;
    let source = match mesh {
        Some(m) => m,
        None => {
            fallback = sphere();
            &fallback
        }
    };
    let scaled;
    let mesh = if stretch == [1.0; 3] {
        source
    } else {
        scaled = Mesh {
            vertices: source
                .vertices
                .iter()
                .map(|v| {
                    Vec3::new(
                        v.x * stretch[0] / largest,
                        v.y * stretch[1] / largest,
                        v.z * stretch[2] / largest,
                    )
                })
                .collect(),
            triangles: source.triangles.clone(),
            source: String::new(),
        };
        &scaled
    };
    let (nw, nh) = (width * 2, height * 2);
    let eye = Vec3::new(-2.3, -3.0, 1.9);
    let forward = (Vec3::ZERO - eye).normalized();
    let right = forward.cross(Vec3::new(0.0, 0.0, 1.0)).normalized();
    let up = right.cross(forward);
    let light = Vec3::new(-0.35, -0.75, 0.9).normalized();
    let half = (light - forward).normalized();
    // Fit the projected silhouette.
    let mut extent = 1e-6f64;
    for v in &mesh.vertices {
        extent = extent.max(v.dot(right).abs()).max(v.dot(up).abs());
    }
    let scale = nw.min(nh) as f64 * 0.42 / extent;
    let project = |v: Vec3| {
        [
            nw as f64 * 0.5 + v.dot(right) * scale,
            nh as f64 * 0.5 - v.dot(up) * scale,
            v.dot(forward),
        ]
    };
    let albedo = material.linear().map(f64::from);
    let metal = material.metallic;
    let shininess = (2.0 / material.roughness.powi(4).max(1e-4) - 2.0).clamp(2.0, 1024.0);
    let shade = |normal: Vec3| -> [f64; 3] {
        let mut nn = normal.normalized();
        if nn.dot(forward) > 0.0 {
            nn = nn * -1.0;
        }
        let diffuse = nn.dot(light).max(0.0);
        let rim = (1.0 + nn.dot(forward)).powi(3) * 0.12;
        let spec = nn.dot(half).max(0.0).powf(shininess) * (shininess + 8.0) / 64.0;
        let mut out = [0.0; 3];
        for c in 0..3 {
            let f0 = 0.04 * (1.0 - metal) + albedo[c] * metal;
            out[c] = albedo[c] * (1.0 - metal) * (0.16 + 0.84 * diffuse)
                + albedo[c] * metal * (0.3 + 0.25 * diffuse)
                + f0 * spec.min(6.0) * diffuse.max(0.15)
                + rim;
        }
        out
    };
    let mut color = vec![[0.0f64; 3]; nw * nh];
    for y in 0..nh {
        let t = y as f64 / nh as f64;
        let bg = [0.020 + 0.012 * t, 0.024 + 0.014 * t, 0.030 + 0.016 * t];
        for x in 0..nw {
            color[y * nw + x] = bg;
        }
    }
    let mut depth = vec![f64::INFINITY; nw * nh];
    for tri in &mesh.triangles {
        let p = tri.map(|i| mesh.vertices[i]);
        let s = p.map(project);
        let area =
            (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[1][1] - s[0][1]) * (s[2][0] - s[0][0]);
        if area.abs() < 1e-12 {
            continue;
        }
        let face = (p[1] - p[0]).cross(p[2] - p[0]);
        let flat = shade(face);
        let x0 = s
            .iter()
            .map(|v| v[0])
            .fold(f64::INFINITY, f64::min)
            .max(0.0) as usize;
        let x1 = (s.iter().map(|v| v[0]).fold(f64::NEG_INFINITY, f64::max) + 1.0).min(nw as f64)
            as usize;
        let y0 = s
            .iter()
            .map(|v| v[1])
            .fold(f64::INFINITY, f64::min)
            .max(0.0) as usize;
        let y1 = (s.iter().map(|v| v[1]).fold(f64::NEG_INFINITY, f64::max) + 1.0).min(nh as f64)
            as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((s[1][0] - px) * (s[2][1] - py) - (s[1][1] - py) * (s[2][0] - px)) / area;
                let w1 = ((s[2][0] - px) * (s[0][1] - py) - (s[2][1] - py) * (s[0][0] - px)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * s[0][2] + w1 * s[1][2] + w2 * s[2][2];
                let i = y * nw + x;
                if z >= depth[i] {
                    continue;
                }
                depth[i] = z;
                color[i] = if smooth {
                    shade(p[0] * w0 + p[1] * w1 + p[2] * w2)
                } else {
                    flat
                };
            }
        }
    }
    let mut rgb = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0f64; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let sample = color[(y * 2 + dy) * nw + x * 2 + dx];
                for (total, value) in sum.iter_mut().zip(sample) {
                    *total += value;
                }
            }
            for total in sum {
                let linear = total * 0.25;
                let mapped = linear / (1.0 + linear * 0.35);
                rgb.push((mapped.max(0.0).powf(1.0 / 2.2).min(1.0) * 255.0).round() as u8);
            }
        }
    }
    Frame { width, height, rgb }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn material_and_mesh_thumbnails_differ_and_are_deterministic() {
        let yellow = Material::new(1, "Yellow", [232, 178, 32], 0.45, 0.0);
        let blue = Material::new(2, "Blue", [40, 80, 220], 0.45, 0.0);
        let a = render(None, [1.0; 3], &yellow, 64, 64);
        assert_eq!(a.rgb, render(None, [1.0; 3], &yellow, 64, 64).rgb);
        assert_ne!(a.rgb, render(None, [1.0; 3], &blue, 64, 64).rgb);
        let centre = (32 * 64 + 32) * 3;
        assert!(a.rgb[centre] > a.rgb[centre + 2], "yellow sphere centre");
        let corner = &a.rgb[0..3];
        assert!(corner.iter().all(|&v| v < 60), "dark backdrop");
        let cube = Mesh::parse_obj(
            "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\nf 1 2 3 4\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8",
            "cube",
        )
        .unwrap();
        let c = render(Some(&cube), [1.0; 3], &yellow, 64, 64);
        assert_eq!(c.width, 64);
        assert_ne!(c.rgb, a.rgb);
        let lit = c.rgb.chunks(3).filter(|p| p[0] > 80).count();
        assert!(lit > 64 * 64 / 5, "cube covers a useful area: {lit}");
        let wide = render(Some(&cube), [1.0; 3], &yellow, 92, 44);
        assert_eq!(
            (wide.width, wide.height, wide.rgb.len()),
            (92, 44, 92 * 44 * 3)
        );
    }
}
