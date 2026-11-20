//! W4-media: Klangdaten für den Musik-Mock. Statt Platzhalterbytes liefert `/Audio/{id}/universal` eine kurze,
//! echte WAV-Datei (Sinuston, je Titel andere Tonhöhe), damit der Audio-Player im Test wirklich dekodiert.

/// Erzeugt eine WAV-Datei (PCM, 16 Bit, mono, 8 kHz) mit einem Sinuston.
pub fn wav(seconds: f32, freq: f32) -> Vec<u8> {
    let rate = 8000u32;
    let n = (rate as f32 * seconds) as usize;
    let mut data = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / rate as f32;
        let fade = (1.0 - (t / seconds - 0.5).abs() * 0.4).clamp(0.0, 1.0);
        let v = (t * freq * std::f32::consts::TAU).sin() * 0.2 * fade;
        data.extend_from_slice(&((v * 32767.0) as i16).to_le_bytes());
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

/// Tonhöhe und Länge zu einer Titel-Id: kurz (Länge aus `LFX_MOCK_TRACK_SECS`, Standard 2 s).
pub fn track_wav(id: &str) -> Vec<u8> {
    let h = id
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(u32::from(b)));
    let secs = std::env::var("LFX_MOCK_TRACK_SECS")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(2.0);
    wav(secs, 220.0 + (h % 8) as f32 * 40.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_consistent() {
        let w = wav(0.5, 440.0);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..12], b"WAVE");
        let data_len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(w.len(), 44 + data_len);
        assert_eq!(data_len, 8000);
        assert_ne!(track_wav("a"), track_wav("b"));
    }
}
