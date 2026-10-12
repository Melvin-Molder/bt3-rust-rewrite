/// `gAdxChannelGainTbl`, by player (`ADX_CH_*`): BGM, MAP, streamed SE 0..1, voice 0..1.
const CHANNEL_GAIN: [i32; 6] = [0x3C, 0x58, 0x60, 0x80, 0x46, 0x46];
/// Default sound option volume.
const OPTION_VOLUME: i32 = 9;
pub const CH_BGM: usize = 0;
pub const CH_MAP: usize = 1;
pub const CH_SE0: usize = 2;
pub const CH_VOICE0: usize = 4;
/// ADXT output volume of silence, in 0.1 dB.
pub const OUTVOL_MUTE: i32 = -960;
/// `ADX_FADE_STEP`: 12 dB.
const FADE_STEP: i32 = 120;

/// `Adx_CalcOutVol`: a player volume (0..0x80) to an ADXT output volume in 0.1 dB.
pub fn out_volume(channel: usize, volume: i32) -> i32 {
    let volume = (volume * CHANNEL_GAIN[channel] * OPTION_VOLUME / 0x480).clamp(0, 0x80);
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
