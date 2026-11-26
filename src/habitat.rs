//! Seabed habitat classes on a regular grid in scene metres (visual only;
//! habitat never changes physics). Real-world scenes take coral reef and
//! seagrass extents from the UNEP-WCMC global distribution maps; synthetic
//! scenes may generate them by rule.
use serde_json::{Value, json};

/// Warm-water coral reef.
pub const REEF: u8 = 1;
/// Seagrass meadow.
pub const SEAGRASS: u8 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Habitat {
    pub columns: usize,
    pub rows: usize,
    /// Centre of cell (0, 0), scene metres (X east, Y north).
    pub origin: [f64; 2],
    pub spacing: f64,
    /// Row-major classes, row 0 at `origin[1]` (south).
    pub classes: Vec<u8>,
    /// Where the classes came from (dataset and licence, or "synthetic").
    pub source: String,
}
impl Habitat {
    pub fn new(
        columns: usize,
        rows: usize,
        origin: [f64; 2],
        spacing: f64,
        classes: Vec<u8>,
        source: &str,
    ) -> Result<Self, &'static str> {
        if !(1..=1024).contains(&columns)
            || !(1..=1024).contains(&rows)
            || columns * rows != classes.len()
            || !spacing.is_finite()
            || spacing <= 0.0
            || !origin.iter().all(|v| v.is_finite())
            || classes.iter().any(|c| *c > SEAGRASS)
        {
            return Err("invalid habitat grid");
        }
        Ok(Self {
            columns,
            rows,
            origin,
            spacing,
            classes,
            source: source.chars().take(300).collect(),
        })
    }
    /// Class at a scene point (nearest cell; none outside the grid).
    pub fn at(&self, x: f64, y: f64) -> u8 {
        let i = ((x - self.origin[0]) / self.spacing).round();
        let j = ((y - self.origin[1]) / self.spacing).round();
        if i < 0.0 || j < 0.0 || i >= self.columns as f64 || j >= self.rows as f64 {
            return 0;
        }
        self.classes[j as usize * self.columns + i as usize]
    }
    /// Fraction of cells of a class.
    pub fn fraction(&self, class: u8) -> f64 {
        self.classes.iter().filter(|c| **c == class).count() as f64 / self.classes.len() as f64
    }
    pub fn any(&self) -> bool {
        self.classes.iter().any(|c| *c != 0)
    }
    /// Run-length encoding: "count*class" pairs separated by commas.
    fn encode(&self) -> String {
        let mut out = String::new();
        let mut i = 0;
        while i < self.classes.len() {
            let c = self.classes[i];
            let mut n = 1;
            while i + n < self.classes.len() && self.classes[i + n] == c {
                n += 1;
            }
            if !out.is_empty() {
                out.push(',');
            }
            out.push_str(&format!("{n}*{c}"));
            i += n;
        }
        out
    }
    pub fn value(&self) -> Value {
        json!({"columns":self.columns,"rows":self.rows,"origin":self.origin,"spacing":self.spacing,"classes":self.encode(),"source":self.source})
    }
    pub fn from_value(v: &Value) -> Result<Self, String> {
        let n = |k: &str| v[k].as_u64().ok_or(format!("Invalid habitat {k}"));
        let columns = n("columns")? as usize;
        let rows = n("rows")? as usize;
        if columns == 0 || rows == 0 || columns > 1024 || rows > 1024 {
            return Err("Invalid habitat size".into());
        }
        let origin = v["origin"]
            .as_array()
            .filter(|a| a.len() == 2)
            .and_then(|a| Some([a[0].as_f64()?, a[1].as_f64()?]))
            .ok_or("Invalid habitat origin")?;
        let spacing = v["spacing"].as_f64().ok_or("Invalid habitat spacing")?;
        let mut classes = Vec::with_capacity(columns * rows);
        for run in v["classes"]
            .as_str()
            .ok_or("Invalid habitat classes")?
            .split(',')
            .filter(|s| !s.is_empty())
        {
            let (count, class) = run.split_once('*').ok_or("Invalid habitat run")?;
            let count: usize = count.parse().map_err(|_| "Invalid habitat run")?;
            let class: u8 = class.parse().map_err(|_| "Invalid habitat class")?;
            if classes.len() + count > columns * rows {
                return Err("Habitat runs exceed the grid".into());
            }
            classes.extend(std::iter::repeat_n(class, count));
        }
        Self::new(
            columns,
            rows,
            origin,
            spacing,
            classes,
            v["source"].as_str().unwrap_or(""),
        )
        .map_err(str::to_string)
    }
}

/// Satellite colours on a regular grid (sRGB bytes, row 0 south), draped on
/// land in the 3D view. Visual only.
#[derive(Clone, Debug, PartialEq)]
pub struct Imagery {
    pub columns: usize,
    pub rows: usize,
    pub origin: [f64; 2],
    pub spacing: f64,
    pub rgb: Vec<u8>,
    pub source: String,
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                B64[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

fn decode64(text: &str) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0;
    for c in text.bytes().filter(|c| *c != b'=') {
        let v = B64.iter().position(|b| *b == c).ok_or("invalid base64")? as u32;
        buffer = buffer << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
}

impl Imagery {
    pub fn new(
        columns: usize,
        rows: usize,
        origin: [f64; 2],
        spacing: f64,
        rgb: Vec<u8>,
        source: &str,
    ) -> Result<Self, &'static str> {
        if !(2..=1024).contains(&columns)
            || !(2..=1024).contains(&rows)
            || rgb.len() != columns * rows * 3
            || !spacing.is_finite()
            || spacing <= 0.0
            || !origin.iter().all(|v| v.is_finite())
        {
            return Err("invalid imagery grid");
        }
        Ok(Self {
            columns,
            rows,
            origin,
            spacing,
            rgb,
            source: source.chars().take(300).collect(),
        })
    }
    /// Linear-light colour at a point (bilinear; None outside the grid).
    pub fn linear(&self, x: f64, y: f64) -> Option<[f32; 3]> {
        let fx = (x - self.origin[0]) / self.spacing;
        let fy = (y - self.origin[1]) / self.spacing;
        if fx < 0.0 || fy < 0.0 || fx > (self.columns - 1) as f64 || fy > (self.rows - 1) as f64 {
            return None;
        }
        let (i, j) = (
            (fx as usize).min(self.columns - 2),
            (fy as usize).min(self.rows - 2),
        );
        let (tx, ty) = (fx - i as f64, fy - j as f64);
        let at = |i: usize, j: usize, c: usize| {
            let v = self.rgb[(j * self.columns + i) * 3 + c] as f64 / 255.0;
            v.powf(2.2)
        };
        Some(std::array::from_fn(|c| {
            (at(i, j, c) * (1.0 - tx) * (1.0 - ty)
                + at(i + 1, j, c) * tx * (1.0 - ty)
                + at(i, j + 1, c) * (1.0 - tx) * ty
                + at(i + 1, j + 1, c) * tx * ty) as f32
        }))
    }
    pub fn value(&self) -> Value {
        json!({"columns":self.columns,"rows":self.rows,"origin":self.origin,"spacing":self.spacing,"rgb":encode64(&self.rgb),"source":self.source})
    }
    pub fn from_value(v: &Value) -> Result<Self, String> {
        let n = |k: &str| {
            v[k].as_u64()
                .map(|x| x as usize)
                .ok_or(format!("Invalid imagery {k}"))
        };
        let origin = v["origin"]
            .as_array()
            .filter(|a| a.len() == 2)
            .and_then(|a| Some([a[0].as_f64()?, a[1].as_f64()?]))
            .ok_or("Invalid imagery origin")?;
        let rgb = decode64(v["rgb"].as_str().ok_or("Invalid imagery colours")?)?;
        Self::new(
            n("columns")?,
            n("rows")?,
            origin,
            v["spacing"].as_f64().ok_or("Invalid imagery spacing")?,
            rgb,
            v["source"].as_str().unwrap_or(""),
        )
        .map_err(str::to_string)
    }
}

#[cfg(test)]
mod imagery_tests {
    use super::*;
    #[test]
    fn imagery_round_trips_and_samples_linear_colour() {
        let rgb: Vec<u8> = (0..4 * 3 * 3).map(|i| (i * 7) as u8).collect();
        let img = Imagery::new(4, 3, [0.0, 0.0], 10.0, rgb, "test").unwrap();
        assert_eq!(Imagery::from_value(&img.value()).unwrap(), img);
        let c = img.linear(0.0, 0.0).unwrap();
        assert!((c[1] - (7.0f64 / 255.0).powf(2.2) as f32).abs() < 1e-6);
        assert!(img.linear(-1.0, 0.0).is_none());
        assert_eq!(decode64(&encode64(b"pelagic!")).unwrap(), b"pelagic!");
        assert!(Imagery::new(2, 2, [0.0; 2], 1.0, vec![0; 5], "").is_err());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn habitat_round_trips_and_answers_point_queries() {
        let mut classes = vec![0u8; 20 * 10];
        for c in classes.iter_mut().skip(40).take(25) {
            *c = REEF;
        }
        classes[199] = SEAGRASS;
        let h = Habitat::new(20, 10, [-95.0, -45.0], 10.0, classes, "test").unwrap();
        let back = Habitat::from_value(&h.value()).unwrap();
        assert_eq!(back, h);
        // Cell (0, 2) is reef: x = -95, y = -25.
        assert_eq!(h.at(-95.0, -25.0), REEF);
        assert_eq!(h.at(95.0, 45.0), SEAGRASS);
        assert_eq!(h.at(1e4, 0.0), 0);
        assert!((h.fraction(REEF) - 25.0 / 200.0).abs() < 1e-12);
        assert!(Habitat::new(2, 2, [0.0; 2], 1.0, vec![0, 0, 0, 9], "").is_err());
        let mut bad = h.value();
        bad["classes"] = json!("500*1");
        assert!(Habitat::from_value(&bad).is_err());
    }
}
