//! The two small sounds of the bar: a soft rising "pop" when it opens and a falling
//! one when it closes.
//!
//! They are made here from sine glides (no recorded sample, so there is nothing to
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

static OPEN_EARLY: AtomicBool = AtomicBool::new(false);

/// Play the open sound right now and remember that it was played, so the later "bar is
/// shown" step does not play it twice.
pub(crate) fn play_open_early() {
    OPEN_EARLY.store(true, Ordering::Relaxed);
    play(Cue::Open);
}

/// Play the open sound unless [`play_open_early`] already did for this opening.
pub(crate) fn play_open_unless_early() {
    if !OPEN_EARLY.swap(false, Ordering::Relaxed) {
        play(Cue::Open);
    }
}

/// Wake the audio device at startup with a silent clip. The first sound after the
/// device has been idle can lose its first moments; this takes that hit instead.
pub(crate) fn warm_up() {
    static SILENCE: OnceLock<Vec<u8>> = OnceLock::new();
    if ENABLED.load(Ordering::Relaxed) {
        play_wav(SILENCE.get_or_init(|| wav_bytes(&vec![0i16; (RATE as usize) / 5])));
    }
}

/// Play a cue in the background. Never blocks and never fails loudly.
pub(crate) fn play(cue: Cue) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let wav = match cue {
        Cue::Open => UP.get_or_init(|| pop(430.0, 920.0, 0.26, 0.12)),
        Cue::Close => DOWN.get_or_init(|| pop(780.0, 360.0, 0.2, 0.0)),
    };
    play_wav(wav);
}

static ALARMING: AtomicBool = AtomicBool::new(false);
static ALARM: OnceLock<Vec<u8>> = OnceLock::new();
/// The alarm gives up on its own after this long, so a forgotten one cannot ring all night.
const ALARM_MAX_SECS: u64 = 90;

/// Ring until [`stop_alarm`] (or [`ALARM_MAX_SECS`]): a bright four-note chime, again and
/// again. Not muted by `bar_sounds`: an alarm nobody hears is a missed reminder.
pub(crate) fn start_alarm() {
    if ALARMING.swap(true, Ordering::SeqCst) {
        return; // already ringing
    }
    std::thread::spawn(|| {
        // The volume change (started earlier, in parallel with the bar appearing) is done
        // before the first note.
        if let Some(prep) = PREP.lock().ok().and_then(|mut p| p.take()) {
            let _ = prep.join();
        }
        let wav = ALARM.get_or_init(alarm_wav);
        let started = std::time::Instant::now();
        while ALARMING.load(Ordering::SeqCst) && started.elapsed().as_secs() < ALARM_MAX_SECS {
            play_wav(wav);
            // The clip is about 1.1 s; wait it out in small steps so a stop is quick.
            for _ in 0..16 {
                if !ALARMING.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        ALARMING.store(false, Ordering::SeqCst);
        restore_system_volume();
    });
}

/// What the system volume was before the alarm raised it: (percent, was muted).
static SAVED_VOLUME: std::sync::Mutex<Option<(String, bool)>> = std::sync::Mutex::new(None);
static PREP: std::sync::Mutex<Option<std::thread::JoinHandle<()>>> = std::sync::Mutex::new(None);

/// Begin raising the system volume to 100% and unmuting, in the background. An alarm that
/// plays at a whisper (or into a muted PC) is a missed reminder. [`start_alarm`] waits for
/// it; the old level comes back when the alarm stops.
pub(crate) fn prepare_alarm() {
    if let Ok(mut p) = PREP.lock() {
        *p = Some(std::thread::spawn(raise_system_volume));
    }
}

#[cfg(windows)]
fn raise_system_volume() {
    use jitmo_connectors::system_controls::{PowerShell, ScriptRunner, VOLUME_SCRIPT};
    let run =
        |op: &str, n: &str| PowerShell.run(VOLUME_SCRIPT, &[("JITMO_OP", op), ("JITMO_N", n)]);
    // Never overwrite what an earlier alarm already saved.
    let Ok(mut saved) = SAVED_VOLUME.lock() else {
        return;
    };
    if saved.is_none() {
        if let Ok(out) = run("get", "0") {
            let (pct, state) = out.split_once(' ').unwrap_or((out.as_str(), ""));
            *saved = Some((pct.to_string(), state == "muted"));
        }
    }
    let _ = run("unmute", "0");
    let _ = run("set", "100");
}

#[cfg(windows)]
fn restore_system_volume() {
    use jitmo_connectors::system_controls::{PowerShell, ScriptRunner, VOLUME_SCRIPT};
    let Some((pct, muted)) = SAVED_VOLUME.lock().ok().and_then(|mut s| s.take()) else {
        return;
    };
    let run =
        |op: &str, n: &str| PowerShell.run(VOLUME_SCRIPT, &[("JITMO_OP", op), ("JITMO_N", n)]);
    let _ = run("set", &pct);
    if muted {
        let _ = run("mute", "0");
    }
}

#[cfg(not(windows))]
fn raise_system_volume() {}

#[cfg(not(windows))]
fn restore_system_volume() {}

/// Is an alarm ringing? The bar must not be put away by anything but the user's own
/// dismissal while it is.
pub(crate) fn is_alarming() -> bool {
    ALARMING.load(Ordering::SeqCst)
}

/// Silence the alarm at once.
pub(crate) fn stop_alarm() {
    if ALARMING.swap(false, Ordering::SeqCst) {
        stop_wav();
    }
}

/// Four bell-like notes (A5 A5 A5 E6), each with a fast attack and a ringing decay.
fn alarm_wav() -> Vec<u8> {
    let pi2 = 2.0 * std::f32::consts::PI;
    let notes = [880.0f32, 880.0, 880.0, 1318.5];
    let step = (RATE as f32 * 0.22) as usize;
    let len = (RATE as f32 * 0.4) as usize;
    let total = step * (notes.len() - 1) + len;
    let mut mixed = vec![0.0f32; total];
    for (n, freq) in notes.iter().enumerate() {
        for i in 0..len {
            let t = i as f32 / RATE as f32;
            let attack = (i as f32 / (RATE as f32 * 0.004)).min(1.0);
            let env = attack * (-t * 7.0).exp();
            let tone = (pi2 * freq * t).sin() + 0.35 * (pi2 * freq * 2.0 * t).sin();
            mixed[n * step + i] += tone * env;
        }
    }
    let samples: Vec<i16> = mixed
        .iter()
        .map(|v| (v * 0.34 * i16::MAX as f32) as i16)
        .collect();
    wav_bytes(&samples)
}

/// A rounded "bubble" pop: a sine that glides from `from` to `to` Hz over about 90 ms
/// (a soft pitch curve, not a beep), with a quick attack, a smooth decay, a faint
/// overtone, and one quiet echo for a glassy tail. About 0.22 s in all.
fn pop(from: f32, to: f32, gain: f32, lead_secs: f32) -> Vec<u8> {
    let pi2 = 2.0 * std::f32::consts::PI;
    let sweep = RATE as f32 * 0.09;
    let body = (RATE as f32 * 0.16) as usize;
    let echo_at = (RATE as f32 * 0.075) as usize;
    // A short silent lead-in: a sleeping audio device wakes during it, not during the pop.
    let lead = (RATE as f32 * lead_secs) as usize;
    let total = lead + body + echo_at;
    let mut dry = vec![0.0f32; body];
    let mut phase = 0.0f32;
    for (i, out) in dry.iter_mut().enumerate() {
        let progress = (i as f32 / sweep).min(1.0);
        // Exponential glide: equal steps in pitch, so it sounds even.
        let freq = from * (to / from).powf(progress);
        phase += pi2 * freq / RATE as f32;
        let t = i as f32 / RATE as f32;
        let attack = (i as f32 / (RATE as f32 * 0.006)).min(1.0);
        let decay = (-t * 20.0).exp();
        *out = (phase.sin() + 0.18 * (phase * 2.0).sin()) * attack * decay;
    }
    let mut mixed = vec![0.0f32; total];
    for (i, v) in dry.iter().enumerate() {
        mixed[lead + i] += v;
        mixed[lead + i + echo_at] += v * 0.22;
    }
    let samples: Vec<i16> = mixed
        .iter()
        .map(|v| (v * gain * i16::MAX as f32) as i16)
        .collect();
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

#[cfg(windows)]
fn stop_wav() {
    #[link(name = "winmm")]
    extern "system" {
        fn PlaySoundW(sound: *const u16, module: *mut core::ffi::c_void, flags: u32) -> i32;
    }
    // SAFETY: a null sound stops whatever PlaySound is playing; no pointer is read.
    unsafe {
        PlaySoundW(core::ptr::null(), core::ptr::null_mut(), 0);
    }
}

#[cfg(not(windows))]
fn play_wav(_wav: &'static [u8]) {}

#[cfg(not(windows))]
fn stop_wav() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cues_are_valid_short_quiet_wav_files() {
        for wav in [pop(430.0, 920.0, 0.26, 0.12), pop(780.0, 360.0, 0.2, 0.0)] {
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
            assert_eq!(wav.len(), 44 + data_len);
            let seconds = (data_len / 2) as f32 / RATE as f32;
            assert!((0.1..0.45).contains(&seconds), "{seconds}");
            let peak = (44..wav.len() - 1)
                .step_by(2)
                .map(|i| i16::from_le_bytes([wav[i], wav[i + 1]]).unsigned_abs())
                .max()
                .unwrap();
            assert!(peak > 1000 && peak < 12_000, "peak {peak}");
        }
    }

    #[test]
    fn the_alarm_is_a_valid_clip_about_a_second_long() {
        let wav = alarm_wav();
        assert_eq!(&wav[..4], b"RIFF");
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(wav.len(), 44 + data_len);
        let seconds = (data_len / 2) as f32 / RATE as f32;
        assert!((0.9..1.3).contains(&seconds), "{seconds}");
    }

    #[test]
    fn cue_names() {
        assert!(Cue::from_name("open").is_some());
        assert!(Cue::from_name("close").is_some());
        assert!(Cue::from_name("x").is_none());
    }
}
