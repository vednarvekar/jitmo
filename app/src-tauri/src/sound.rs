//! The two small sounds of the bar: a soft rising "pop" when it opens and a falling
//! one when it closes.
//!
//! They are made here from sine waves (no recorded sample, so there is nothing to
//! license) and played with the Windows `PlaySound` call from memory, which works
//! without any user click in the page. The setting `bar_sounds` = `off` mutes them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

const RATE: u32 = 44_100;

static ENABLED: AtomicBool = AtomicBool::new(true);
static UP: OnceLock<Vec<u8>> = OnceLock::new();
static DOWN: OnceLock<Vec<u8>> = OnceLock::new();

/// Mute or unmute (from the `bar_sounds` setting).
pub(crate) fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Which sound.
#[derive(Clone, Copy)]
pub(crate) enum Cue {
    Open,
    Close,
}

impl Cue {
    pub(crate) fn from_name(name: &str) -> Option<Cue> {
        match name {
            "open" => Some(Cue::Open),
            "close" => Some(Cue::Close),
            _ => None,
        }
    }
}

/// Play a cue in the background. Never blocks and never fails loudly.
pub(crate) fn play(cue: Cue) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let wav = match cue {
        Cue::Open => UP.get_or_init(|| tone_pair(660.0, 990.0)),
        Cue::Close => DOWN.get_or_init(|| tone_pair(880.0, 560.0)),
    };
    play_wav(wav);
}

/// Two quick soft notes (`a` then `b` Hz), each with a short attack and a smooth
/// decay, with a faint octave overtone for a "glassy" feel. About 0.17 s in all, at
/// low volume.
fn tone_pair(a: f32, b: f32) -> Vec<u8> {
    let note_len = (RATE as f32 * 0.085) as usize;
    let gap = (RATE as f32 * 0.012) as usize;
    let mut samples: Vec<i16> = Vec::with_capacity(note_len * 2 + gap);
    for (n, freq) in [a, b].into_iter().enumerate() {
        for i in 0..note_len {
            let t = i as f32 / RATE as f32;
            let attack = (i as f32 / (RATE as f32 * 0.004)).min(1.0);
            let decay = (-t * 26.0).exp();
            let wave = (2.0 * std::f32::consts::PI * freq * t).sin()
                + 0.25 * (2.0 * std::f32::consts::PI * freq * 2.0 * t).sin();
            let v = wave * attack * decay * 0.16;
            samples.push((v * i16::MAX as f32) as i16);
        }
        if n == 0 {
            samples.extend(std::iter::repeat_n(0, gap));
        }
    }
    wav_bytes(&samples)
}

/// A mono 16-bit PCM WAV file in memory.
fn wav_bytes(samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(windows)]
fn play_wav(wav: &'static [u8]) {
    #[link(name = "winmm")]
    extern "system" {
        fn PlaySoundW(sound: *const u16, module: *mut core::ffi::c_void, flags: u32) -> i32;
    }
    const SND_ASYNC: u32 = 0x0001;
    const SND_NODEFAULT: u32 = 0x0002;
    const SND_MEMORY: u32 = 0x0004;
    // SAFETY: with SND_MEMORY the pointer must stay valid while the sound plays; the
    // buffers live in `static` cells for the whole run of the program.
    unsafe {
        PlaySoundW(
            wav.as_ptr().cast(),
            core::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

#[cfg(not(windows))]
fn play_wav(_wav: &'static [u8]) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cues_are_valid_short_quiet_wav_files() {
        for wav in [tone_pair(660.0, 990.0), tone_pair(880.0, 560.0)] {
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
            assert_eq!(wav.len(), 44 + data_len);
            let seconds = (data_len / 2) as f32 / RATE as f32;
            assert!((0.1..0.3).contains(&seconds), "{seconds}");
            let peak = (44..wav.len() - 1)
                .step_by(2)
                .map(|i| i16::from_le_bytes([wav[i], wav[i + 1]]).unsigned_abs())
                .max()
                .unwrap();
            assert!(peak > 1000 && peak < 12_000, "peak {peak}");
        }
    }

    #[test]
    fn cue_names() {
        assert!(Cue::from_name("open").is_some());
        assert!(Cue::from_name("close").is_some());
        assert!(Cue::from_name("x").is_none());
    }
}
