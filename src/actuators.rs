//! Bounded six-axis wrench allocation. Geometry is body-frame; force is newtons.
use crate::math::Vec3;
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Thruster {
    pub direction: Vec3,
    pub position: Vec3,
    pub limit: f64,
}
impl Thruster {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.direction.is_finite()
            || !self.position.is_finite()
            || (self.direction.length() - 1.0).abs() > 1e-6
            || self.position.length() > 20.0
            || !self.limit.is_finite()
            || !(0.0..=1e5).contains(&self.limit)
        {
            return Err("invalid thruster direction, position or force limit");
        }
        Ok(())
    }
}
pub fn rov() -> Vec<Thruster> {
    let d = std::f64::consts::FRAC_1_SQRT_2;
    let mut t = vec![];
    for (x, y, dy) in [
        (0.4, 0.3, -d),
        (0.4, -0.3, d),
        (-0.4, 0.3, d),
        (-0.4, -0.3, -d),
    ] {
        t.push(Thruster {
            direction: Vec3::new(d, dy, 0.0),
            position: Vec3::new(x, y, 0.0),
            limit: 20.0,
        });
    }
    for x in [-0.4, 0.4] {
        t.push(Thruster {
            direction: Vec3::new(0.0, 0.0, 1.0),
            position: Vec3::new(x, 0.0, 0.0),
            limit: 20.0,
        });
    }
    t
}
/// Bounded least-squares thruster values for a body-frame force and torque.
/// Roll and pitch are passively stable (hydrostatics), so they are only
/// controlled when a roll or pitch torque is requested; otherwise thrusters
/// that unavoidably make a roll or pitch moment (propellers below the centre
/// of mass) would be throttled back to cancel it, starving surge.
pub fn allocate(thrusters: &[Thruster], force: Vec3, torque: Vec3) -> Vec<f64> {
    let mut values = vec![0.0; thrusters.len()];
    let roll = if torque.x != 0.0 { 3.0 } else { 0.0 };
    let pitch = if torque.y != 0.0 { 3.0 } else { 0.0 };
    let columns: Vec<_> = thrusters
        .iter()
        .map(|t| {
            [
                t.direction.x,
                t.direction.y,
                t.direction.z,
                t.position.cross(t.direction).x * roll,
                t.position.cross(t.direction).y * pitch,
                t.position.cross(t.direction).z * 3.0,
            ]
        })
        .collect();
    let target = [
        force.x,
        force.y,
        force.z,
        torque.x * roll,
        torque.y * pitch,
        torque.z * 3.0,
    ];
    let step = 1.0
        / columns
            .iter()
            .flatten()
            .map(|v| v * v)
            .sum::<f64>()
            .max(1e-9);
    for _ in 0..96 {
        let mut error = target;
        for (column, value) in columns.iter().zip(&values) {
            for axis in 0..6 {
                error[axis] -= column[axis] * value;
            }
        }
        for (i, column) in columns.iter().enumerate() {
            let gradient = column.iter().zip(error).map(|(a, b)| a * b).sum::<f64>();
            values[i] =
                (values[i] + step * gradient).clamp(-thrusters[i].limit, thrusters[i].limit);
        }
    }
    values
}
pub fn wrench(thrusters: &[Thruster], values: &[f64]) -> (Vec3, Vec3) {
    let mut force = Vec3::ZERO;
    let mut torque = Vec3::ZERO;
    for (t, value) in thrusters.iter().zip(values) {
        let f = t.direction * *value;
        force = force + f;
        torque = torque + t.position.cross(f);
    }
    (force, torque)
}
