//! Terrain queries shared by rendering, motion and sensors.
use crate::math::Det;
use crate::{math::Vec3, ocean::Bathymetry};

#[derive(Clone, Debug, PartialEq)]
pub struct HeightGrid {
    pub columns: usize,
    pub rows: usize,
    pub origin: [f64; 2],
    pub spacing: f64,
    pub elevations: Vec<f64>,
}
impl HeightGrid {
    pub fn new(
        columns: usize,
        rows: usize,
        origin: [f64; 2],
        spacing: f64,
        elevations: Vec<f64>,
    ) -> Result<Self, &'static str> {
        if columns < 2
            || rows < 2
            || columns.checked_mul(rows) != Some(elevations.len())
            || !spacing.is_finite()
            || spacing <= 0.0
            || !origin.iter().all(|v| v.is_finite())
            || elevations.iter().any(|v| !v.is_finite())
        {
            return Err("invalid height grid");
        }
        Ok(Self {
            columns,
            rows,
            origin,
            spacing,
            elevations,
        })
    }
    /// Outside samples clamp to the nearest boundary. Bounds remain explicit.
    pub fn bounds(&self) -> [f64; 4] {
        [
            self.origin[0],
            self.origin[1],
            self.origin[0] + (self.columns - 1) as f64 * self.spacing,
            self.origin[1] + (self.rows - 1) as f64 * self.spacing,
        ]
    }
    pub fn normal(&self, x: f64, y: f64) -> Vec3 {
        let e = self.spacing * 0.5;
        Vec3::new(
            self.elevation(x - e, y) - self.elevation(x + e, y),
            self.elevation(x, y - e) - self.elevation(x, y + e),
            2.0 * e,
        )
        .normalized()
    }
    pub fn procedural(kind: u8, depth: f64) -> Self {
        let size = 129;
        let spacing = 4.0;
        let mut data = Vec::with_capacity(size * size);
        for j in 0..size {
            for i in 0..size {
                let x = i as f64 * spacing - 256.0;
                let y = j as f64 * spacing - 256.0;
                let rough = 1.8 * (x * 0.067).dsin() * (y * 0.043).dcos()
                    + 0.6 * (x * 0.21 + y * 0.13).dsin();
                let z = match kind {
                    1 => {
                        -depth
                            + (depth + 14.0)
                                * (-((x + 85.0) / 48.0).powi(2) - ((y - 75.0) / 90.0).powi(2))
                                    .dexp()
                            + rough
                    }
                    2 => -depth - 22.0 * (-(x / 28.0).powi(2)).dexp() + rough * 1.5,
                    _ => -depth,
                };
                data.push(z);
            }
        }
        Self::new(size, size, [-256.0, -256.0], spacing, data).expect("procedural dimensions")
    }
    /// Portable text heightmap: rows of finite SI elevations, northing increasing
    /// by row; comma or whitespace separation. Georeferencing supplied separately.
    pub fn parse_csv(text: &str, origin: [f64; 2], spacing: f64) -> Result<Self, String> {
        let mut columns = 0;
        let mut rows = 0;
        let mut elevations = vec![];
        for line in text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        {
            let values: Result<Vec<f64>, _> = line
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|v| !v.is_empty())
                .map(str::parse::<f64>)
                .collect();
            let values = values.map_err(|_| "Terrain needs numeric elevation rows in metres.")?;
            if columns == 0 {
                columns = values.len();
            }
            if columns != values.len() {
                return Err("Terrain rows have different widths.".into());
            }
            rows += 1;
            elevations.extend(values);
            if elevations.len() > 1_048_576 {
                return Err("Terrain exceeds one million samples.".into());
            }
        }
        Self::new(columns, rows, origin, spacing, elevations).map_err(str::to_string)
    }
}
impl Bathymetry for HeightGrid {
    fn elevation(&self, x: f64, y: f64) -> f64 {
        let fx = ((x - self.origin[0]) / self.spacing).clamp(0.0, (self.columns - 1) as f64);
        let fy = ((y - self.origin[1]) / self.spacing).clamp(0.0, (self.rows - 1) as f64);
        let i = (fx as usize).min(self.columns - 2);
        let j = (fy as usize).min(self.rows - 2);
        let u = fx - i as f64;
        let v = fy - j as f64;
        let at = |i, j| self.elevations[j * self.columns + i];
        (at(i, j) * (1.0 - u) + at(i + 1, j) * u) * (1.0 - v)
            + (at(i, j + 1) * (1.0 - u) + at(i + 1, j + 1) * u) * v
    }
    fn grid(&self) -> Option<&HeightGrid> {
        Some(self)
    }
    fn revision(&self) -> u64 {
        self.elevations.iter().chain(self.origin.iter()).fold(
            self.spacing.to_bits() ^ self.columns as u64 ^ ((self.rows as u64) << 32),
            |hash, v| hash.rotate_left(7) ^ v.to_bits(),
        )
    }
}
