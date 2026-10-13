//! The fighters' pad vibration (`BtlChar_SetVibration`, `BtlChar_SetSmallVibration`, `BtlChar_Vibrate`,
//! `BtlCharApi_RumbleNear`, `BtlChar_UpdateVibration`, src/battle/btl_char_util.c; `Pad_AddVibration` /
//! `Pad_SendActuator`, src/sys/pad.c). The big motor runs at the asked power (a weaker request does not replace a
//! running stronger one), modulated between 50 % and 100 % by a sine of six cycles a second; the small motor is on
//! every other frame while its time lasts. A CPU fighter's pad (or one whose vibration the save turns off; a fresh
//! save has both on) gets nothing.

use crate::btl_chr::Chr;

/// `BtlCharVib`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Vib {
    pub enabled: bool,
    pub power: f32,
    pub time: i32,
    pub small_time: i32,
    phase: i32,
    toggle: u32,
}

impl Default for Vib {
    /// `SaveData.flags & (2 << pad)` of a fresh save (`SAVE_FLAG_DEFAULT` 7): on.
    fn default() -> Self {
        Self {
            enabled: true,
            power: 0.0,
            time: 0,
            small_time: 0,
            phase: 0,
            toggle: 0,
        }
    }
}

impl Vib {
    /// The rest of `BtlCharVib` cleared (power, times, phase, toggle), as a stage change does.
    pub(crate) fn clear(&mut self) {
        *self = Self {
            enabled: self.enabled,
            ..Self::default()
        };
    }
}

/// `Pad.vibPower` / `vibSmall` as a frame's requests add up (`Pad_AddVibration`: the power summed, at most 1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PadVib {
    pub power: f32,
    pub small: bool,
}

impl PadVib {
    fn add(&mut self, small: bool, power: f32) {
        self.power += power;
        self.small |= small;
        if 1.0 < self.power {
            self.power = 1.0;
        }
    }

    /// `Pad_SendActuator`'s `vibData`: the big motor's power (0..127) shifted left, the small motor in bit 0;
    /// the request is cleared.
    pub(crate) fn send(&mut self) -> u8 {
        let power = ((self.power * 127.0) as i32).clamp(0, 0x7F);
        let data = (power << 1) as u8 | u8::from(self.small);
        *self = Self::default();
        data
    }
}

impl Chr {
    /// `BtlChar_SetVibration`: the big motor at `power` (at most 1) for `seconds`, unless a stronger one runs.
    pub(crate) fn set_vibration(&mut self, power: f32, seconds: f32) {
        let power = power.min(1.0);
        let vib = &mut self.vib;
        if vib.time <= 0 || vib.power < power {
            vib.power = power;
            vib.time = (seconds * 30.0) as i32;
        }
    }

    /// `BtlChar_SetSmallVibration`: the small motor for at least `seconds`.
    pub(crate) fn set_small_vibration(&mut self, seconds: f32) {
        let frames = (seconds * 30.0) as i32;
        if self.vib.small_time < frames {
            self.vib.small_time = frames;
        }
    }

    /// `BtlChar_Vibrate`: both motors.
    pub(crate) fn vibrate(&mut self, power: f32, seconds: f32) {
        self.set_vibration(power, seconds);
        self.set_small_vibration(seconds);
    }

    /// `BtlChar_UpdateVibration`: this frame's request to the fighter's pad (none while disabled, in a replay or
    /// for the CPU), the times counted down.
    pub(crate) fn update_vibration(&mut self, pad: &mut PadVib, replay: bool) {
        let cpu = self.input.inject_on;
        let vib = &mut self.vib;
        if !vib.enabled || replay || cpu {
            vib.time = 0;
            vib.small_time = 0;
            return;
        }
        let mut power = 0.0;
        if vib.time > 0 {
            let half = 0.5f32;
            power = vib.power
                * (((vib.phase as f32 * 37.699_112 / 30.0).sin() * half + half) * half + half);
            vib.phase += 1;
        } else {
            vib.phase = 0;
        }
        let mut small = false;
        if vib.small_time > 0 {
            small = vib.toggle % 2 == 0;
            vib.toggle = vib.toggle.wrapping_add(1);
        } else {
            vib.toggle = 0;
        }
        pad.add(small, power);
        vib.time = (vib.time - 1).max(0);
        vib.small_time = (vib.small_time - 1).max(0);
    }
}

/// `BtlCharApi_RumbleNear`: every fighter within `far` of `pos` vibrates, in proportion to its nearness (full
/// within `near`); the small motor too when it is more than halfway in.
pub(crate) fn rumble_near(
    chars: [&mut Chr; 2],
    pos: [f32; 3],
    near: f32,
    far: f32,
    power: f32,
    time: f32,
) {
    for chr in chars {
        let p = chr.pose.pos;
        let d = [p[0] - pos[0], p[1] - pos[1], p[2] - pos[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let rate = 1.0 - (len - near) / (far - near);
        if rate < 0.0 {
            continue;
        }
        let rate = rate.min(1.0);
        let t = time * rate;
        chr.set_vibration(power * rate, t);
        if rate > 0.5 {
            chr.set_small_vibration(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_data_packs_the_two_motors() {
        let mut p = PadVib::default();
        p.add(true, 0.6);
        p.add(false, 0.6);
        assert_eq!(p.send(), 0x7F << 1 | 1);
        assert_eq!(p.send(), 0);
    }
}
