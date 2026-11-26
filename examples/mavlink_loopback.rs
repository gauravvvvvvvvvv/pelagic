//! Real UDP process-to-process smoke test on private ephemeral loopback ports.
use std::{net::UdpSocket, time::Duration};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let peer = UdpSocket::bind("127.0.0.1:0")?;
    peer.set_read_timeout(Some(Duration::from_secs(2)))?;
    let reserve = UdpSocket::bind("127.0.0.1:0")?;
    let bind = reserve.local_addr()?;
    drop(reserve);
    let adapter = std::env::current_exe()?.with_file_name(if cfg!(windows) {
        "mavlink.exe"
    } else {
        "mavlink"
    });
    let mut child = std::process::Command::new(adapter)
        .args([
            bind.to_string(),
            peer.local_addr()?.to_string(),
            "1.5".into(),
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    let mut bytes = [0u8; 512];
    let mut telemetry = 0;
    let mut commands = 0;
    for _ in 0..8 {
        let (n, source) = peer.recv_from(&mut bytes)?;
        assert_eq!(source, bind);
        if bytes[5] == 32 {
            assert_eq!(n, 36);
            assert_eq!(
                u16::from_le_bytes([bytes[n - 2], bytes[n - 1]]),
                pelagic::bridge::crc(&bytes[1..n - 2], 185)
            );
            telemetry += 1;
        }
        let mut payload = vec![];
        for value in [500i16, 0, 0, 0] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.push(1);
        peer.send_to(&pelagic::bridge::packet(69, &payload, 243, commands), bind)?;
        commands += 1;
    }
    let status = child.wait()?;
    assert!(status.success());
    assert!(telemetry >= 6);
    let mut output = String::new();
    use std::io::Read;
    if let Some(mut stdout) = child.stdout.take() {
        stdout.read_to_string(&mut output)?;
    }
    assert!(output.contains("8 accepted commands"), "{output}");
    println!(
        "UDP loopback verified: {telemetry} telemetry frames, {commands} control packets. {}",
        output.trim()
    );
    Ok(())
}
