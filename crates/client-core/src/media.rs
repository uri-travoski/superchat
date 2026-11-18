//! M-30: details of voice messages and audio files that the sender computes before uploading: the duration and a
//! waveform (64 loudness values). Decoding uses symphonia (WAV, AAC in MP4/M4A, MP3, Ogg Vorbis, FLAC), so the same
//! code works for the Linux recorder (WAV), the Android recorder (M4A) and audio files picked by hand.

use proto::AttachmentMeta;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const PEAK_COUNT: usize = 64;

/// Duration and waveform of an audio file, `None` when it cannot be decoded.
pub fn analyze_audio(path: &Path) -> Option<AttachmentMeta> {
    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).ok()?;
    let mut format = probed.format;
    let track = format.default_track()?.clone();
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).ok()?;
    let rate = track.codec_params.sample_rate.unwrap_or(16_000) as u64;
    // Loudness (RMS) of each chunk of about 20 ms.
    let mut chunk_rms: Vec<f32> = vec![];
    let mut total_frames: u64 = 0;
    let (mut acc, mut n) = (0f64, 0usize);
    let mut chunk_len;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(_)) | Err(SymError::ResetRequired) => break,
            Err(_) => break,
        };
        if packet.track_id() != track.id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        let ch = spec.channels.count().max(1);
        chunk_len = (spec.rate as u64 / 50).max(1) as usize;
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        for frame in buf.samples().chunks(ch) {
            let v = frame.iter().copied().sum::<f32>() / ch as f32;
            acc += (v * v) as f64;
            n += 1;
            total_frames += 1;
            if n >= chunk_len {
                chunk_rms.push((acc / n as f64).sqrt() as f32);
                acc = 0.0;
                n = 0;
            }
        }
    }
    if n > 0 {
        chunk_rms.push((acc / n as f64).sqrt() as f32);
    }
    if total_frames == 0 || chunk_rms.is_empty() {
        return None;
    }
    let duration_ms = (total_frames * 1000 / rate.max(1)) as u32;
    Some(AttachmentMeta { duration_ms: Some(duration_ms), peaks: bucket_peaks(&chunk_rms, PEAK_COUNT), ..Default::default() })
}

/// Reduces loudness values to `count` bars (each the loudest of its slice), scaled so the loudest bar is 255.
pub fn bucket_peaks(rms: &[f32], count: usize) -> Vec<u8> {
    if rms.is_empty() {
        return vec![];
    }
    let mut out = vec![0f32; count];
    for (i, o) in out.iter_mut().enumerate() {
        let a = i * rms.len() / count;
        let b = (((i + 1) * rms.len() / count).max(a + 1)).min(rms.len());
        *o = rms[a.min(rms.len() - 1)..b].iter().cloned().fold(0.0, f32::max);
    }
    let max = out.iter().cloned().fold(0.0, f32::max);
    if max <= 0.0 {
        return vec![8; count];
    }
    // A square root keeps quiet speech visible next to loud peaks.
    out.iter().map(|v| (((v / max).sqrt()) * 255.0).round().clamp(8.0, 255.0) as u8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
        let mut v = vec![];
        let data_len = (samples.len() * 2) as u32;
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * 2).to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            v.extend_from_slice(&s.to_le_bytes());
        }
        v
    }

    #[test]
    fn waveform_follows_the_loudness_and_duration_is_exact() {
        // 2 seconds at 16 kHz: a quiet first half, silence, then a loud tone in the last quarter.
        let rate = 16_000u32;
        let mut s = vec![];
        for i in 0..(rate * 2) {
            let t = i as f32 / rate as f32;
            let amp = if t < 0.5 { 3000.0 } else if t < 1.5 { 0.0 } else { 20000.0 };
            s.push((amp * (t * 2.0 * std::f32::consts::PI * 440.0).sin()) as i16);
        }
        let dir = std::env::temp_dir().join(format!("scwave-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("tone.wav");
        std::fs::write(&p, wav(&s, rate)).unwrap();
        let m = analyze_audio(&p).expect("decodes");
        assert_eq!(m.duration_ms, Some(2000));
        assert_eq!(m.peaks.len(), PEAK_COUNT);
        let first = &m.peaks[..PEAK_COUNT / 4];
        let quiet = &m.peaks[PEAK_COUNT / 4..PEAK_COUNT * 3 / 4];
        let loud = &m.peaks[PEAK_COUNT * 3 / 4..];
        assert!(loud.iter().all(|p| *p > 200), "loud tone {loud:?}");
        assert!(quiet.iter().all(|p| *p < 40), "silence {quiet:?}");
        assert!(first.iter().all(|p| *p > 60 && *p < 200), "soft tone {first:?}");
        assert!(analyze_audio(&dir.join("missing.wav")).is_none());
        std::fs::write(dir.join("junk.wav"), b"not audio at all").unwrap();
        assert!(analyze_audio(&dir.join("junk.wav")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
