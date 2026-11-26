//! Small explicit data adapters. No implicit web access or missing-value filling.
use crate::{fields::CurrentGrid, math::Vec3, terrain::HeightGrid};
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Provenance {
    pub source: String,
    pub source_url: String,
    pub licence: String,
    pub retrieved_utc: String,
    pub resolution: String,
    pub measured: bool,
    pub classification: String,
    pub origin_latitude: f64,
    pub origin_longitude: f64,
}
impl Default for Provenance {
    fn default() -> Self {
        Self {
            source: "Synthetic Pelagic terrain".into(),
            source_url: String::new(),
            licence: "Generated locally".into(),
            retrieved_utc: String::new(),
            resolution: "4 m synthetic grid".into(),
            measured: false,
            classification: "synthetic".into(),
            origin_latitude: 15.0,
            origin_longitude: 73.7,
        }
    }
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FlowGridData {
    #[serde(default)]
    pub source: String,
    pub shape: [usize; 3],
    pub origin: Vec3,
    pub spacing: Vec3,
    pub times: Vec<f64>,
    pub values: Vec<Vec3>,
}
impl FlowGridData {
    pub fn field(&self) -> Result<CurrentGrid, &'static str> {
        CurrentGrid::new(
            self.shape,
            self.origin,
            self.spacing,
            self.times.clone(),
            self.values.clone(),
        )
    }
}
/// CSV columns time_s,x_m,y_m,z_m,u_m_s,v_m_s,w_m_s on a complete regular lattice.
pub fn current_csv(text: &str) -> Result<FlowGridData, &'static str> {
    if text.len() > 8_388_608 {
        return Err("flow CSV exceeds 8 MB");
    }
    let mut rows = vec![];
    for line in text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        if line.starts_with("time_s") {
            continue;
        }
        let values = line
            .split(',')
            .map(|s| {
                s.trim()
                    .parse::<f64>()
                    .map_err(|_| "invalid numeric flow row")
            })
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != 7 || values.iter().any(|v| !v.is_finite()) {
            return Err("flow needs seven finite SI columns");
        }
        rows.push(values);
        if rows.len() > 131072 {
            return Err("flow grid exceeds desktop budget");
        }
    }
    let axis = |column: usize| {
        let mut values: Vec<f64> = rows.iter().map(|r| r[column]).collect();
        values.sort_by(f64::total_cmp);
        values.dedup();
        values
    };
    let axes = [axis(1), axis(2), axis(3)];
    let times = axis(0);
    if axes.iter().any(|a| a.len() < 2) || times.is_empty() {
        return Err("flow requires at least two samples per spatial axis");
    }
    let spacing: Vec<f64> = axes.iter().map(|a| a[1] - a[0]).collect();
    for (a, step) in axes.iter().zip(&spacing) {
        if a.windows(2)
            .any(|p| ((p[1] - p[0]) - step).abs() > 1e-7 * step.abs().max(1.0))
        {
            return Err("flow axes must be uniformly spaced");
        }
    }
    let shape = [axes[0].len(), axes[1].len(), axes[2].len()];
    let count = shape
        .iter()
        .try_fold(times.len(), |n, s| n.checked_mul(*s))
        .ok_or("flow dimensions overflow")?;
    if count != rows.len() {
        return Err("missing flow grid samples");
    }
    let mut values = vec![None; count];
    for row in rows {
        let locate = |a: &[f64], v: f64| {
            a.binary_search_by(|x| x.total_cmp(&v))
                .map_err(|_| "flow coordinate missing")
        };
        let t = locate(&times, row[0])?;
        let x = locate(&axes[0], row[1])?;
        let y = locate(&axes[1], row[2])?;
        let z = locate(&axes[2], row[3])?;
        let i = ((t * shape[2] + z) * shape[1] + y) * shape[0] + x;
        if values[i].is_some() {
            return Err("duplicate flow sample");
        }
        values[i] = Some(Vec3::new(row[4], row[5], row[6]));
    }
    let data = FlowGridData {
        source: "User-supplied SI current grid; provenance unverified".into(),
        shape,
        origin: Vec3::new(axes[0][0], axes[1][0], axes[2][0]),
        spacing: Vec3::new(spacing[0], spacing[1], spacing[2]),
        times,
        values: values
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or("incomplete flow grid")?,
    };
    data.field()?;
    Ok(data)
}
/// ESRI ASCII grid, cell-centre interpretation; NODATA is rejected explicitly.
pub fn esri_ascii(text: &str) -> Result<HeightGrid, &'static str> {
    let mut lines = text.lines();
    let mut header = std::collections::BTreeMap::new();
    for _ in 0..6 {
        let line = lines.next().ok_or("missing ASCII grid header")?;
        let mut fields = line.split_whitespace();
        let key = fields.next().ok_or("empty header")?.to_ascii_lowercase();
        let value = fields
            .next()
            .ok_or("missing header value")?
            .parse::<f64>()
            .map_err(|_| "invalid header number")?;
        header.insert(key, value);
    }
    let get = |key: &str| {
        header
            .get(key)
            .copied()
            .ok_or("missing ASCII grid header field")
    };
    let columns = get("ncols")?;
    let rows = get("nrows")?;
    if columns.fract() != 0.0
        || rows.fract() != 0.0
        || !(2.0..=257.0).contains(&columns)
        || !(2.0..=257.0).contains(&rows)
    {
        return Err("ASCII grid dimensions must be 2–257");
    }
    let spacing = get("cellsize")?;
    let x = header
        .get("xllcenter")
        .copied()
        .or_else(|| header.get("xllcorner").map(|v| v + spacing * 0.5))
        .ok_or("missing X origin")?;
    let y = header
        .get("yllcenter")
        .copied()
        .or_else(|| header.get("yllcorner").map(|v| v + spacing * 0.5))
        .ok_or("missing Y origin")?;
    let nodata = get("nodata_value")?;
    let mut heights = lines
        .flat_map(str::split_whitespace)
        .map(|v| v.parse::<f64>().map_err(|_| "invalid ASCII grid elevation"))
        .collect::<Result<Vec<_>, _>>()?;
    if heights.iter().any(|v| !v.is_finite() || *v == nodata) {
        return Err("ASCII grid contains missing data; fill explicitly before import");
    }
    if heights.len() != columns as usize * rows as usize {
        return Err("ASCII grid cell count mismatch");
    }
    let mut flipped = vec![];
    for row in heights.chunks_mut(columns as usize).rev() {
        flipped.extend_from_slice(row);
    }
    HeightGrid::new(columns as usize, rows as usize, [x, y], spacing, flipped)
}
/// NOAA ERDDAP latitude,longitude,altitude CSV to a local isotropic grid.
/// Bilinear resampling; elevations remain unscaled SI values.
pub fn geographic_csv(text: &str) -> Result<(HeightGrid, f64, f64), &'static str> {
    let mut records = vec![];
    for line in text.lines().skip(2) {
        let r = line
            .split(',')
            .map(|s| {
                s.trim()
                    .parse::<f64>()
                    .map_err(|_| "invalid geographic CSV")
            })
            .collect::<Result<Vec<_>, _>>()?;
        if r.len() != 3 || r.iter().any(|v| !v.is_finite()) {
            return Err("geographic CSV needs latitude,longitude,finite elevation");
        }
        records.push(r);
        if records.len() > 66049 {
            return Err("geographic grid exceeds 257² samples");
        }
    }
    let axis = |i: usize| {
        let mut a: Vec<_> = records.iter().map(|r| r[i]).collect();
        a.sort_by(f64::total_cmp);
        a.dedup();
        a
    };
    let lat = axis(0);
    let lon = axis(1);
    if lat.len() < 2 || lon.len() < 2 || lat.len() * lon.len() != records.len() {
        return Err("geographic data must form a complete rectangular grid");
    }
    if lat[0] < -85.0
        || lat[lat.len() - 1] > 85.0
        || lon[0] < -180.0
        || lon[lon.len() - 1] > 180.0
        || lat[lat.len() - 1] - lat[0] > 1.0
        || lon[lon.len() - 1] - lon[0] > 1.0
    {
        return Err("Import a non-polar regional subset no wider than 1 degree per axis.");
    }
    for a in [&lat, &lon] {
        let step = a[1] - a[0];
        if a.windows(2).any(|w| ((w[1] - w[0]) - step).abs() > 1e-8) {
            return Err("Geographic source axes must be regularly spaced.");
        }
    }
    let latitude = (lat[0] + lat[lat.len() - 1]) * 0.5;
    let longitude = (lon[0] + lon[lon.len() - 1]) * 0.5;
    if latitude.abs() > 85.0 {
        return Err("local tangent import excludes polar regions");
    }
    let sx = (lon[1] - lon[0]).to_radians() * 6371000.0 * latitude.to_radians().cos();
    let sy = (lat[1] - lat[0]).to_radians() * 6371000.0;
    let mut heights = vec![f64::NAN; records.len()];
    for r in records {
        let y = lat.binary_search_by(|v| v.total_cmp(&r[0])).unwrap();
        let x = lon.binary_search_by(|v| v.total_cmp(&r[1])).unwrap();
        let i = y * lon.len() + x;
        if heights[i].is_finite() {
            return Err("duplicate geographic cell");
        }
        heights[i] = r[2];
    }
    if heights.iter().any(|v| !v.is_finite()) {
        return Err("missing geographic elevations");
    }
    let spacing = sx.max(sy);
    let width = (sx * (lon.len() - 1) as f64 / spacing).floor() as usize + 1;
    let height = (sy * (lat.len() - 1) as f64 / spacing).floor() as usize + 1;
    if width < 2 || height < 2 {
        return Err("geographic grid is too narrow");
    }
    let mut output = vec![];
    for y in 0..height {
        for x in 0..width {
            let fx = x as f64 * spacing / sx;
            let fy = y as f64 * spacing / sy;
            let ix = (fx as usize).min(lon.len() - 2);
            let iy = (fy as usize).min(lat.len() - 2);
            let (a, b) = (fx - ix as f64, fy - iy as f64);
            let at = |xx, yy| heights[yy * lon.len() + xx];
            output.push(
                (at(ix, iy) * (1.0 - a) + at(ix + 1, iy) * a) * (1.0 - b)
                    + (at(ix, iy + 1) * (1.0 - a) + at(ix + 1, iy + 1) * a) * b,
            );
        }
    }
    Ok((
        HeightGrid::new(
            width,
            height,
            [
                -((lon.len() - 1) as f64) * sx * 0.5,
                -((lat.len() - 1) as f64) * sy * 0.5,
            ],
            spacing,
            output,
        )?,
        latitude,
        longitude,
    ))
}
