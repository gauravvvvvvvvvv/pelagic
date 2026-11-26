//! A short, original CPU-synthesised ocean swell and subdued minor chord.
//! No recording, playback service, codec, or background audio thread is needed.
pub const DURATION: f64 = 4.0;
const RATE: u32 = 44100;

pub fn startup_wave() -> Vec<u8> {
    let samples = (DURATION * RATE as f64) as usize;
    let data_len = samples as u32 * 2;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    let mut seed = 0x70656c61_u32;
    let mut low = 0.0_f64;
    let mut lower = 0.0_f64;
    for i in 0..samples {
        let t = i as f64 / RATE as f64;
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let white = seed as f64 / u32::MAX as f64 * 2.0 - 1.0;
        low += 0.075 * (white - low);
        lower += 0.009 * (low - lower);
        // A gentle attack, then a long release; the final 100 ms are silent.
        // Silence before the window transition prevents an audible hard stop.
        let attack = smooth(t / 0.8);
        let release = 1.0 - smooth((t - 1.35) / 2.55);
        let envelope = attack * release * release;
        let chord = [110.0, 130.8128, 164.8138]
            .iter()
            .map(|f| (std::f64::consts::TAU * f * t).sin())
            .sum::<f64>()
            / 3.0;
        let swell = (low - lower) * 0.28 + chord * 0.035;
        let pcm =
            (swell * envelope * i16::MAX as f64).clamp(i16::MIN as f64, i16::MAX as f64) as i16;
        bytes.extend_from_slice(&pcm.to_le_bytes());
    }
    bytes
}

fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cue_is_short_quiet_and_fades_to_silence() {
        let wave = startup_wave();
        assert_eq!(&wave[..4], b"RIFF");
        assert_eq!(wave.len(), 44 + (DURATION * RATE as f64) as usize * 2);
        let pcm: Vec<i16> = wave[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(pcm[0], 0);
        assert_eq!(pcm[pcm.len() - 1], 0);
        assert!(pcm.iter().any(|s| s.abs() > 100));
        assert!(pcm.iter().all(|s| s.abs() < 8000));
        let rms = |start: f64, end: f64| {
            let section = &pcm[(start * RATE as f64) as usize..(end * RATE as f64) as usize];
            (section.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / section.len() as f64).sqrt()
        };
        assert!(rms(3.0, 3.5) < rms(2.0, 2.5));
        assert!(rms(3.5, 3.9) < rms(3.0, 3.5));
        assert!(pcm[(3.9 * RATE as f64) as usize..].iter().all(|s| *s == 0));
    }
}
