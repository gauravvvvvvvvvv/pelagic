//! MAVLink 1 subset: MANUAL_CONTROL in, LOCAL_POSITION_NED and HEARTBEAT out.
//! Caller owns transport and scheduling. No networking is started by the library.
use crate::math::Vec3;
pub fn crc(bytes: &[u8], extra: u8) -> u16 {
    let mut crc = 0xffffu16;
    for &byte in bytes.iter().chain(std::iter::once(&extra)) {
        let mut tmp = byte ^ (crc as u8);
        tmp ^= tmp << 4;
        crc = (crc >> 8) ^ ((tmp as u16) << 8) ^ ((tmp as u16) << 3) ^ ((tmp as u16) >> 4);
    }
    crc
}
pub fn packet(message: u8, payload: &[u8], extra: u8, sequence: u8) -> Vec<u8> {
    assert!(payload.len() <= 255);
    let mut frame = vec![0xfe, payload.len() as u8, sequence, 1, 1, message];
    frame.extend_from_slice(payload);
    frame.extend_from_slice(&crc(&frame[1..], extra).to_le_bytes());
    frame
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Manual {
    pub force: Vec3,
    pub torque: Vec3,
}
pub fn manual(frame: &[u8], thrust: f64) -> Option<Manual> {
    if frame.len() != 19
        || frame[0] != 0xfe
        || frame[1] != 11
        || frame[5] != 69
        || frame[16] != 1
        || !thrust.is_finite()
        || thrust < 0.0
        || thrust > 1e6
    {
        return None;
    }
    if u16::from_le_bytes([frame[17], frame[18]]) != crc(&frame[1..17], 243) {
        return None;
    }
    let axis = |i: usize| {
        let v = i16::from_le_bytes([frame[6 + i * 2], frame[7 + i * 2]]);
        if v == i16::MAX {
            Some(0.0)
        } else if (-1000..=1000).contains(&v) {
            Some(v as f64 / 1000.0)
        } else {
            None
        }
    };
    // MAVLink body convention forward/right/down; Pelagic body +Y left, +Z up.
    Some(Manual {
        force: Vec3::new(axis(0)? * thrust, -axis(1)? * thrust, -axis(2)? * thrust),
        torque: Vec3::new(0.0, 0.0, -axis(3)? * thrust * 0.1),
    })
}
pub fn local_position(position: Vec3, velocity: Vec3, time: f64, sequence: u8) -> Vec<u8> {
    let mut payload = vec![];
    payload.extend_from_slice(&((time.max(0.0) * 1000.0) as u32).to_le_bytes());
    for v in [
        position.y,
        position.x,
        -position.z,
        velocity.y,
        velocity.x,
        -velocity.z,
    ] {
        payload.extend_from_slice(&(v as f32).to_le_bytes());
    }
    packet(32, &payload, 185, sequence)
}
pub fn heartbeat(sequence: u8) -> Vec<u8> {
    let mut payload = vec![];
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&[12, 8, 0, 4, 3]);
    packet(0, &payload, 50, sequence)
}
