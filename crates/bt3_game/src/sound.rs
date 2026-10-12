use std::sync::atomic::{AtomicU32, Ordering};

/// The save's sound options, which the audio reads as it plays (`gSaveData`: `flags` bit 0 the voice set,
/// `volume[3]` at 0x16A4: the sound mode, 0 stereo or 1 mono, then the music's and the voices' and effects'
/// volumes, 0..9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundOptions {
    pub mono: bool,
    pub bgm_volume: i32,
    pub se_volume: i32,
    /// `SAVE_FLAG_VOICE`: the second voice set.
    pub alt_voice: bool,
}

impl Default for SoundOptions {
    /// A fresh save's: stereo, volumes 9, the second voice set.
    fn default() -> Self {
        Self {
            mono: false,
            bgm_volume: 9,
            se_volume: 9,
            alt_voice: true,
        }
    }
}

impl SoundOptions {
    fn pack(self) -> u32 {
        u32::from(self.mono)
            | u32::from(self.alt_voice) << 1
            | (self.bgm_volume.clamp(0, 0xFF) as u32) << 8
            | (self.se_volume.clamp(0, 0xFF) as u32) << 16
    }

    fn unpack(v: u32) -> Self {
        Self {
            mono: v & 1 != 0,
            alt_voice: v & 2 != 0,
            bgm_volume: (v >> 8 & 0xFF) as i32,
            se_volume: (v >> 16 & 0xFF) as i32,
        }
    }
}

/// The options in force: the loaded save's (`gSaveData`), shared by the audio threads.
static OPTIONS: AtomicU32 = AtomicU32::new(0x0909_02);

/// The sound options in force.
pub fn options() -> SoundOptions {
    SoundOptions::unpack(OPTIONS.load(Ordering::Relaxed))
}

/// Takes `o` as the options in force; true when they changed.
pub fn set_options(o: SoundOptions) -> bool {
    OPTIONS.swap(o.pack(), Ordering::Relaxed) != o.pack()
}

/// `gAdxChannelGainTbl`, by player (`ADX_CH_*`): BGM, MAP, streamed SE 0..1, voice 0..1.
const CHANNEL_GAIN: [i32; 6] = [0x3C, 0x58, 0x60, 0x80, 0x46, 0x46];
pub const CH_BGM: usize = 0;
pub const CH_MAP: usize = 1;
pub const CH_SE0: usize = 2;
pub const CH_VOICE0: usize = 4;
/// ADXT output volume of silence, in 0.1 dB.
pub const OUTVOL_MUTE: i32 = -960;
/// `ADX_FADE_STEP`: 12 dB.
const FADE_STEP: i32 = 120;

/// `Adx_CalcOutVol`: a player volume (0..0x80) to an ADXT output volume in 0.1 dB, with the music volume option
/// for the two music players and the voice option for the others.
pub fn out_volume(channel: usize, volume: i32) -> i32 {
    let o = options();
    let option = if channel <= CH_MAP {
        o.bgm_volume
    } else {
        o.se_volume
    };
    let volume = (volume * CHANNEL_GAIN[channel] * option / 0x480).clamp(0, 0x80);
    if volume == 0 {
        return OUTVOL_MUTE;
    }
    let s = ((0x80 - volume) as f32 * 0.0078125).sin();
    (s * s * 0.28 * -960.0) as i32
}

/// `Adx_FadeOutStep`.
pub fn fade_out_step(level: i32) -> i32 {
    if level > OUTVOL_MUTE {
        (level - FADE_STEP).max(OUTVOL_MUTE)
    } else {
        level
    }
}

/// Bevy volume of an ADXT output volume.
pub fn volume(level: i32) -> bevy::audio::Volume {
    if level <= OUTVOL_MUTE {
        bevy::audio::Volume::SILENT
    } else {
        bevy::audio::Volume::Decibels(level as f32 / 10.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{CH_BGM, CH_SE0, CH_VOICE0, OUTVOL_MUTE, fade_out_step, out_volume};

    #[test]
    fn music_and_menu_voice_levels() {
        assert_eq!(out_volume(CH_BGM, 0x40), -129);
        // Voice players 4 / 5 have gain 0x46, streamed SE player 2 gain 0x60.
        assert_eq!(out_volume(CH_VOICE0, 0x80), -51);
        assert_eq!(out_volume(CH_VOICE0 + 1, 0x80), -51);
        assert_eq!(out_volume(CH_SE0, 0x40), -92);
        assert_eq!(out_volume(CH_BGM, 0), OUTVOL_MUTE);
    }

    #[test]
    fn fade_steps_twelve_decibels_down_to_mute() {
        assert_eq!(fade_out_step(-129), -249);
        assert_eq!(fade_out_step(-900), OUTVOL_MUTE);
        assert_eq!(fade_out_step(OUTVOL_MUTE), OUTVOL_MUTE);
    }
}

#[cfg(test)]
mod option_tests {
    use super::*;

    #[test]
    fn the_options_pack_and_default_to_a_fresh_saves() {
        assert_eq!(
            SoundOptions::unpack(SoundOptions::default().pack()),
            SoundOptions::default()
        );
        assert_eq!(SoundOptions::unpack(0x0909_02), SoundOptions::default());
        let o = SoundOptions {
            mono: true,
            bgm_volume: 3,
            se_volume: 0,
            alt_voice: false,
        };
        assert_eq!(SoundOptions::unpack(o.pack()), o);
    }
}
