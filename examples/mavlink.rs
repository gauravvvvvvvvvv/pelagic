//! Loopback-only MAVLink subset adapter. No ROS/Python runtime is needed.
use std::{
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let bind = args.get(1).map_or("127.0.0.1:14551", String::as_str);
    let peer: SocketAddr = args
        .get(2)
        .map_or("127.0.0.1:14550", String::as_str)
        .parse()?;
    let seconds = args
        .get(3)
        .map(|s| s.parse::<f64>())
        .transpose()?
        .unwrap_or(30.0);
    if !peer.ip().is_loopback()
        || !bind.parse::<SocketAddr>()?.ip().is_loopback()
        || !seconds.is_finite()
        || !(0.1..=3600.0).contains(&seconds)
    {
        return Err("Use loopback endpoints and 0.1–3600 seconds".into());
    }
    let socket = UdpSocket::bind(bind)?;
    socket.set_nonblocking(true)?;
    let mut world = pelagic::scene::world(&pelagic::scene::Scene::default())?;
    let start = Instant::now();
    let mut next = start;
    let mut sequence = 0u8;
    let mut last_command = Instant::now();
    let mut buffer = [0u8; 512];
    let mut accepted = 0;
    while start.elapsed().as_secs_f64() < seconds {
        for _ in 0..64 {
            match socket.recv_from(&mut buffer) {
                Ok((len, source)) => {
                    if source == peer
                        && let Some(command) = pelagic::bridge::manual(&buffer[..len], 40.0)
                    {
                        let body = world.vehicle_mut(0).unwrap();
                        body.force = command.force;
                        body.torque = command.torque;
                        last_command = Instant::now();
                        accepted += 1;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.into()),
            }
        }
        if last_command.elapsed() > Duration::from_secs(1) {
            let v = world.vehicle_mut(0).unwrap();
            v.force = pelagic::math::Vec3::ZERO;
            v.torque = pelagic::math::Vec3::ZERO;
        }
        world.step();
        if world.tick().is_multiple_of(6) {
            let v = &world.vehicles()[0];
            socket.send_to(
                &pelagic::bridge::local_position(v.position, v.velocity, world.time(), sequence),
                peer,
            )?;
            sequence = sequence.wrapping_add(1);
        }
        if world.tick().is_multiple_of(60) {
            socket.send_to(&pelagic::bridge::heartbeat(sequence), peer)?;
            sequence = sequence.wrapping_add(1);
        }
        next += Duration::from_secs_f64(1.0 / 60.0);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    println!(
        "MAVLink adapter: {accepted} accepted commands, {} ticks, NED telemetry sent to {peer}",
        world.tick()
    );
    Ok(())
}
