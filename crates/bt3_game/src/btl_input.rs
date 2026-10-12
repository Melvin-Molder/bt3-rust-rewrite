//! Per-fighter battle input (src/battle/btl_input.c): the pad (or the CPU's virtual pad) turned into battle
//! button bits and command bits, their frame counters, and `BtlInput_TestAction`, the "is action input `id`
//! happening" query of the fighter state machine.
//!
//! Once per unpaused frame `sample` builds the record from the pad (`BtlInput_Sample` / `BtlInput_BuildRecord`);
//! later in the frame `update` fetches it through the 8-entry ring (same frame: the ring is empty before the
//! push), gates it by the battle state and derives held / pressed / released, the smoothed stick and the counters.

use crate::btl_flags::Flags;

pub(crate) const GUARD: u32 = 0x1;
pub(crate) const DASH: u32 = 0x2;
pub(crate) const BLAST: u32 = 0x4;
pub(crate) const RUSH: u32 = 0x8;
pub(crate) const UP: u32 = 0x10;
pub(crate) const DOWN: u32 = 0x20;
pub(crate) const LEFT: u32 = 0x40;
pub(crate) const RIGHT: u32 = 0x80;
pub(crate) const RS_LEFT: u32 = 0x100;
pub(crate) const CHARGE: u32 = 0x200;
pub(crate) const RS_RIGHT: u32 = 0x400;
pub(crate) const ASCEND: u32 = 0x800;
pub(crate) const DESCEND: u32 = 0x1000;
pub(crate) const R3: u32 = 0x2000;
pub(crate) const LOCKON: u32 = 0x4000;
pub(crate) const L3R3: u32 = 0x8000;
pub(crate) const RS_UP: u32 = 0x1_0000;
pub(crate) const RS_DOWN: u32 = 0x2_0000;
pub(crate) const ANY_FACE: u32 = 0x10_0000;
pub(crate) const ASCEND_TAP2: u32 = 0x20_0000;
pub(crate) const DESCEND_TAP2: u32 = 0x40_0000;
pub(crate) const BLAST2: u32 = 0x100_0000;
pub(crate) const RS_UP2: u32 = 0x200_0000;
pub(crate) const RS_DOWN2: u32 = 0x400_0000;
pub(crate) const DIR_MASK: u32 = 0xF0;
/// `BTL_BUTTON_BITS`: entries of the mask table (bit 31 is never set).
const BUTTON_BITS: usize = 31;

const C_DASH_HELD: u32 = 0x1;
const C_DASH_CHARGE_P: u32 = 0x2;
const C_DASH_P: u32 = 0x4;
const C_DASH_CHARGE_H: u32 = 0x8;
const C_GUARD_H: u32 = 0x10;
const C_RUSH_TAP: u32 = 0x20;
const C_RUSH_HOLD6: u32 = 0x40;
const C_RUSH_H: u32 = 0x80;
const C_RUSH_NOT_H: u32 = 0x100;
const C_CHARGE_BLAST: u32 = 0x200;
const C_CHARGE_BLAST_U: u32 = 0x400;
const C_CHARGE_BLAST_D: u32 = 0x800;
const C_SWITCH: u32 = 0x1000;
const C_UNUSED2000: u32 = 0x2000;
const C_CHARGE_GUARD: u32 = 0x4000;
const C_CHARGE_GUARD_U: u32 = 0x8000;
const C_DASH_P2: u32 = 0x1_0000;
const C_DASH_TAP2: u32 = 0x2_0000;
const C_DASH_P3: u32 = 0x4_0000;
const C_GUARD_SIDE: u32 = 0x8_0000;
const C_GUARD_H2: u32 = 0x10_0000;
const C_BLAST_P: u32 = 0x20_0000;
const C_LOCKON_P: u32 = 0x40_0000;
const C_DASH_P4: u32 = 0x80_0000;
const C_RUSH_P: u32 = 0x100_0000;
const C_DIR_P: u32 = 0x200_0000;
const C_DIR_LEFT: u32 = 0x400_0000;
const C_DIR_RIGHT: u32 = 0x800_0000;
const C_DIR_UP: u32 = 0x1000_0000;
const C_DIR_DOWN: u32 = 0x2000_0000;
const C_DIR_ANY: u32 = 0x3C00_0000;

/// `BTLB_FILTER_MASK` / `BTLC_FILTER_MASK`: what gets through while the READY notice is up.
const FILTER_MASK: u32 = 0x008F_18F3;
const C_FILTER_MASK: u32 = 0x3C01_0011;

const STICK_CENTER: u8 = 0x7F;
/// `BTL_TAP_FRAMES`: window of the double taps and of RUSH_TAP.
const TAP_FRAMES: i8 = 7;
/// `BTL_COUNT_MAX`: the frame counters saturate here.
const COUNT_MAX: i8 = 100;
const RING: usize = 8;

/// Raw pad bits (`PAD_*`, include/sys/pad.h).
pub(crate) mod pad {
    pub(crate) const SELECT: u32 = 0x1;
    pub(crate) const L3: u32 = 0x2;
    pub(crate) const R3: u32 = 0x4;
    pub(crate) const START: u32 = 0x8;
    pub(crate) const UP: u32 = 0x10;
    pub(crate) const RIGHT: u32 = 0x20;
    pub(crate) const DOWN: u32 = 0x40;
    pub(crate) const LEFT: u32 = 0x80;
    pub(crate) const L2: u32 = 0x100;
    pub(crate) const R2: u32 = 0x200;
    pub(crate) const L1: u32 = 0x400;
    pub(crate) const R1: u32 = 0x800;
    pub(crate) const TRIANGLE: u32 = 0x1000;
    pub(crate) const CIRCLE: u32 = 0x2000;
    pub(crate) const CROSS: u32 = 0x4000;
    pub(crate) const SQUARE: u32 = 0x8000;
    pub(crate) const RSTICK_LEFT: u32 = 0x10_0000;
    pub(crate) const RSTICK_RIGHT: u32 = 0x20_0000;
    pub(crate) const RSTICK_DOWN: u32 = 0x40_0000;
    pub(crate) const RSTICK_UP: u32 = 0x80_0000;
}

/// What `BtlInput_Sample` reads from `gPad[chr->pad]`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PadState {
    /// `Pad.held`: `PAD_*` bits.
    pub held: u32,
    /// `Pad.left`: -1..1, + right / down, inside the unit circle.
    pub left: [f32; 2],
    /// `Pad.status`: 0 connected.
    pub status: i32,
}

/// `Pad_StickToFloat` with `PAD_STICK_DEADZONE` (50 of 127.5), then `Pad_Update`'s clamp to the unit circle.
pub(crate) fn stick_from_raw(x: u8, y: u8) -> [f32; 2] {
    let axis = |raw: u8| {
        let mut v = f32::from(raw) - 127.5;
        if 0.0 < v {
            v = (v - 50.0).max(0.0);
        } else {
            v = (v + 50.0).min(0.0);
        }
        v / (127.5 - 50.0)
    };
    let mut s = [axis(x), axis(y)];
    let len = s[0] * s[0] + s[1] * s[1];
    if 1.0 < len {
        let inv = 1.0 / len.sqrt();
        s = [s[0] * inv, s[1] * inv];
    }
    s
}

/// `BtlInputCounters`: per bit, frames held / not held / since pressed / since released, saturating at 100.
#[derive(Clone, Copy, Debug)]
struct Counters {
    held: [i8; 32],
    not_held: [i8; 32],
    since_press: [i8; 32],
    since_release: [i8; 32],
    prev_since_press: [i8; 32],
}

impl Default for Counters {
    fn default() -> Self {
        Self {
            held: [0; 32],
            not_held: [0; 32],
            since_press: [0; 32],
            since_release: [0; 32],
            prev_since_press: [0; 32],
        }
    }
}

/// `BtlInput_TickCounter`.
fn tick(cond: bool, count: &mut i8) {
    if cond {
        if *count < COUNT_MAX {
            *count += 1;
        }
    } else {
        *count = 0;
    }
}

impl Counters {
    /// `BtlInput_TickCounters`.
    fn tick(&mut self, held: u32, pressed: u32, released: u32) {
        self.prev_since_press = self.since_press;
        for i in 0..32 {
            let bit = 1 << i;
            tick(held & bit != 0, &mut self.held[i]);
            tick(held & bit == 0, &mut self.not_held[i]);
            tick(pressed & bit == 0, &mut self.since_press[i]);
            tick(released & bit == 0, &mut self.since_release[i]);
        }
    }
}

/// `BtlInput_StickToByte`.
fn stick_to_byte(v: f32) -> u8 {
    ((v * 0.5 + 0.5) * 254.0).clamp(0.0, 254.0) as i32 as u8
}

/// `BtlInput_ByteToStick`.
fn byte_to_stick(b: u8) -> f32 {
    f32::from(b) * (1.0 / 254.0) * 2.0 - 1.0
}

/// `BtlUtil_WrapAngle`.
pub(crate) fn wrap_angle(a: f32) -> f32 {
    let mut a = a;
    while a >= std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    while a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}

/// The battle state the input reads (`Battle_GetWork()->flags`, the stage, time stop).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InputGate {
    /// `BATTLE_FLAG_DEMO`: every fighter's fetched input is neutral.
    pub block: bool,
    /// `BATTLE_FLAG_READY`: the fetched input is masked (neutral on stages 4 and 27).
    pub filter: bool,
    /// `BtlChar_IsStage4Or27`.
    pub stage_4_or_27: bool,
    /// `BtlChars_IsTimeStopped`.
    pub time_stopped: bool,
}

/// `BtlCharInput` and the fighter's injected (CPU) input.
#[derive(Clone, Debug)]
pub(crate) struct Input {
    raw_held: u32,
    raw_stick: [u8; 2],
    pad_status: i32,
    mask: [u32; BUTTON_BITS],
    rec_count: Counters,
    rec_cmd_count: Counters,
    held: u32,
    prev: u32,
    pressed: u32,
    released: u32,
    cmd_held: u32,
    cmd_prev: u32,
    cmd_pressed: u32,
    cmd_released: u32,
    stick: [f32; 2],
    stick_prev: [f32; 2],
    stick_raw: [f32; 2],
    stick_raw_prev: [f32; 2],
    stick_turn: f32,
    count: Counters,
    cmd_count: Counters,
    ring: [(u32, u32, [u8; 2]); RING],
    ring_write: usize,
    ring_read: usize,
    /// Bits of the action inputs queried this frame, per `want` (`BtlInput_SetFrameBit`).
    frame_bits: [u8; 30],
    rec_buttons: u32,
    rec_commands: u32,
    rec_stick: [u8; 2],
    /// `chr->injectOn`: the record comes from the fields below (the CPU side).
    pub(crate) inject_on: bool,
    pub(crate) inject_buttons: u32,
    pub(crate) inject_stick: [f32; 2],
    /// What `BtlInput_TestSwitch` tests this frame (from the fighter's flags and technique, set before sampling).
    pub(crate) switch: SwitchTest,
    /// `chr->techClass` (>= 2 enables BTLC_SWITCH).
    pub(crate) tech_class: i32,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            raw_held: 0,
            raw_stick: [0; 2],
            pad_status: 0,
            mask: [0; BUTTON_BITS],
            rec_count: Counters::default(),
            rec_cmd_count: Counters::default(),
            held: 0,
            prev: 0,
            pressed: 0,
            released: 0,
            cmd_held: 0,
            cmd_prev: 0,
            cmd_pressed: 0,
            cmd_released: 0,
            stick: [0.0; 2],
            stick_prev: [0.0; 2],
            stick_raw: [0.0; 2],
            stick_raw_prev: [0.0; 2],
            stick_turn: 0.0,
            count: Counters::default(),
            cmd_count: Counters::default(),
            ring: [(0, 0, [0; 2]); RING],
            ring_write: 0,
            ring_read: 0,
            frame_bits: [0; 30],
            rec_buttons: 0,
            rec_commands: 0,
            rec_stick: [0; 2],
            inject_on: false,
            inject_buttons: 0,
            inject_stick: [0.0; 2],
            tech_class: -1,
            switch: SwitchTest::None,
        }
    }
}

/// `BtlInput_TestSwitch` for `chr->techClass >= 2`: flag 0xA2 (a charge) fires on letting go of the blast
/// button, 0xA3 (repeated shots) on pressing it, else the prompt row's direction with the rush button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SwitchTest {
    None,
    Release,
    Press,
    Dir(i32),
}

/// `SaveData.key[player]`: the action of each physical button (Circle, Cross, Square, Triangle, L1, L2, R1, R2).
pub(crate) const DEFAULT_KEYS: [u8; 8] = [2, 1, 0, 3, 4, 5, 6, 7];

impl Input {
    /// `BtlInput_Init` (`BtlInput_BuildMaskTable`) with the save's key config.
    pub(crate) fn init(&mut self, keys: &[u8; 8]) {
        let key = |action: u8| {
            let bits = [
                pad::CIRCLE,
                pad::CROSS,
                pad::SQUARE,
                pad::TRIANGLE,
                pad::L1,
                pad::L2,
                pad::R1,
                pad::R2,
            ];
            keys.iter()
                .position(|&k| k == action)
                .map_or(0, |i| bits[i])
        };
        let m = &mut self.mask;
        m[3] = key(0);
        m[2] = key(3);
        m[0] = key(2);
        m[1] = key(1);
        m[11] = key(6);
        m[12] = key(7);
        m[9] = key(5);
        m[14] = key(4);
        m[13] = pad::R3;
        m[15] = pad::L3 | pad::R3;
        m[4] = pad::UP;
        m[5] = pad::DOWN;
        m[6] = pad::LEFT;
        m[7] = pad::RIGHT;
        m[20] = m[3] | m[2] | m[0] | m[1];
        m[23] = pad::SELECT;
        m[16] = pad::RSTICK_UP;
        m[17] = pad::RSTICK_DOWN;
        m[18] = pad::RSTICK_LEFT;
        m[19] = pad::RSTICK_RIGHT;
        m[25] = pad::RSTICK_UP;
        m[26] = pad::RSTICK_DOWN;
        m[8] = pad::RSTICK_LEFT;
        m[21] = 0;
        m[22] = 0;
        m[24] = m[2];
        m[10] = pad::RSTICK_RIGHT;
        m[27] = pad::CIRCLE;
        m[28] = pad::CROSS;
        m[29] = pad::TRIANGLE;
        m[30] = pad::SQUARE;
    }

    /// `BtlInput_RemapButtons`: bit i when any bit of mask[i] is down (all of them for L3 + R3).
    fn remap(&self, raw: u32) -> u32 {
        let mut out = 0;
        for (i, &mask) in self.mask.iter().enumerate() {
            let bit = 1 << i;
            let down = if bit & L3R3 != 0 {
                raw & mask == mask
            } else {
                raw & mask != 0
            };
            if down {
                out |= bit;
            }
        }
        out
    }

    /// `BtlInput_MergeStick`: a d-pad bit forces the stick to its end, else the stick's dominant axis sets one.
    fn merge_stick(buttons: u32, stick: &mut [u8; 2]) -> u32 {
        let mut buttons = buttons;
        if buttons & DIR_MASK != 0 {
            if buttons & LEFT != 0 {
                stick[0] = 0;
            }
            if buttons & RIGHT != 0 {
                stick[0] = 0xFF;
            }
            if buttons & UP != 0 {
                stick[1] = 0;
            }
            if buttons & DOWN != 0 {
                stick[1] = 0xFF;
            }
        } else {
            let dx = (i32::from(stick[0]) - i32::from(STICK_CENTER)).abs();
            let dy = (i32::from(stick[1]) - i32::from(STICK_CENTER)).abs();
            if dx >= 2 || dy >= 2 {
                if dy < dx {
                    buttons |= if stick[0] > STICK_CENTER { RIGHT } else { LEFT };
                } else {
                    buttons |= if stick[1] > STICK_CENTER { DOWN } else { UP };
                }
            }
        }
        buttons
    }

    /// `BtlInput_AddDoubleTaps`.
    fn add_double_taps(&self, buttons: u32, prev: u32) -> u32 {
        let mut buttons = buttons;
        let pressed = buttons & !prev;
        if matches!(self.pad_status, 0 | 1) {
            if pressed & ASCEND != 0 && self.rec_count.prev_since_press[11] < TAP_FRAMES {
                buttons |= ASCEND_TAP2;
            }
            if pressed & DESCEND != 0 && self.rec_count.prev_since_press[12] < TAP_FRAMES {
                buttons |= DESCEND_TAP2;
            }
        }
        buttons
    }

    /// `BtlInput_BuildCommands`.
    fn build_commands(&self, held: u32, prev: u32) -> u32 {
        let released = !held & prev;
        let pressed = held & !prev;
        let mut cmd = 0;
        if held & DASH != 0 {
            cmd = if held & CHARGE != 0 {
                C_DASH_CHARGE_H
            } else {
                C_DASH_HELD
            };
        }
        if pressed & DASH != 0 {
            cmd |= C_DASH_P3 | C_DASH_P | C_DASH_P4;
            if held & CHARGE != 0 {
                cmd |= C_DASH_CHARGE_P;
            }
            if held & LEFT != 0 {
                cmd |= C_DIR_LEFT | C_DASH_P2;
            }
            if held & RIGHT != 0 {
                cmd |= C_DIR_RIGHT | C_DASH_P2;
            }
            if held & DOWN != 0 {
                cmd |= C_DIR_DOWN | C_DASH_P2;
            }
            if held & UP != 0 {
                cmd |= C_DIR_UP | C_DASH_P2;
            }
            if held & DIR_MASK == 0 {
                cmd |= C_DIR_UP | C_DASH_P2;
                if self.rec_count.prev_since_press[1] < TAP_FRAMES {
                    cmd |= C_DASH_TAP2;
                }
            }
        }
        if held & GUARD != 0 {
            cmd |= C_GUARD_H2 | C_GUARD_H;
            if pressed & (LEFT | RIGHT) != 0 {
                cmd |= C_GUARD_SIDE;
            }
        }
        if held & CHARGE != 0 {
            if pressed & BLAST != 0 {
                cmd |= if held & DOWN != 0 {
                    C_CHARGE_BLAST_D
                } else if held & UP != 0 {
                    C_CHARGE_BLAST_U
                } else {
                    C_CHARGE_BLAST
                };
            }
            if pressed & GUARD != 0 {
                cmd |= if held & UP != 0 {
                    C_CHARGE_GUARD_U
                } else {
                    C_CHARGE_GUARD
                };
            }
        }
        if released & RUSH != 0 && self.rec_count.since_press[3] < TAP_FRAMES {
            cmd |= C_RUSH_TAP;
        }
        if held & RUSH != 0 {
            if pressed & RUSH != 0 {
                cmd |= C_RUSH_P;
            }
            if self.rec_count.since_press[3] == 6 {
                cmd |= C_RUSH_HOLD6;
            }
            cmd |= C_RUSH_H;
            if held & UP != 0 {
                cmd |= C_DIR_UP;
            } else if held & DOWN != 0 {
                cmd |= C_DIR_DOWN;
            } else if held & LEFT != 0 {
                cmd |= C_DIR_LEFT;
            } else if held & RIGHT != 0 {
                cmd |= C_DIR_RIGHT;
            }
        } else {
            cmd |= C_RUSH_NOT_H;
        }
        if pressed & BLAST != 0 && held & CHARGE == 0 {
            cmd |= C_BLAST_P
                | if held & UP != 0 {
                    C_DIR_UP
                } else if held & LEFT != 0 {
                    C_DIR_LEFT
                } else if held & RIGHT != 0 {
                    C_DIR_RIGHT
                } else {
                    C_DIR_DOWN
                };
        }
        if pressed & LOCKON != 0 {
            cmd |= C_LOCKON_P;
        }
        if pressed & DIR_MASK != 0 {
            cmd |= C_DIR_P;
        }
        // BtlInput_TestSwitch: the technique's button command.
        let switch = match self.switch {
            SwitchTest::None => false,
            SwitchTest::Release => held & BLAST == 0,
            SwitchTest::Press => pressed & BLAST != 0,
            SwitchTest::Dir(dir) => {
                let want = match dir {
                    1 => DOWN,
                    0 => UP,
                    2 => LEFT,
                    3 => RIGHT,
                    _ => 0,
                };
                want != 0 && held & want != 0 && pressed & RUSH != 0
            }
        };
        if switch {
            cmd |= C_SWITCH;
        }
        cmd
    }

    /// `BtlInput_Sample` + `BtlInput_BuildRecord`. `active`: flag 2 or 3 is up, 0x136 is not and time runs (else
    /// the record is neutral). The replay hook records nothing here.
    pub(crate) fn sample(&mut self, pad: &PadState, active: bool, time_stopped: bool) {
        self.pad_status = pad.status;
        self.raw_held = pad.held;
        self.raw_stick = [stick_to_byte(pad.left[0]), stick_to_byte(pad.left[1])];
        let prev_buttons = self.rec_buttons;
        let prev_commands = self.rec_commands;
        let mut stick;
        let (mut buttons, mut commands);
        if self.inject_on {
            buttons = self.inject_buttons;
            if buttons & GUARD != 0 {
                buttons |= ANY_FACE;
            }
            if buttons & BLAST != 0 {
                buttons |= BLAST2;
            }
            if buttons & RS_UP != 0 {
                buttons |= RS_UP2;
            }
            if buttons & RS_DOWN != 0 {
                buttons |= RS_DOWN2;
            }
            stick = [
                stick_to_byte(self.inject_stick[0]),
                stick_to_byte(self.inject_stick[1]),
            ];
            buttons = Self::merge_stick(buttons, &mut stick);
            commands = self.build_commands(buttons, prev_buttons);
        } else {
            buttons = self.remap(self.raw_held);
            stick = self.raw_stick;
            buttons = Self::merge_stick(buttons, &mut stick);
            buttons = self.add_double_taps(buttons, prev_buttons);
            commands = self.build_commands(buttons, prev_buttons);
        }
        if !active {
            buttons = 0;
            stick = [STICK_CENTER; 2];
            commands = 0;
        }
        self.rec_buttons = buttons;
        self.rec_stick = stick;
        self.rec_commands = commands;
        if !time_stopped {
            self.rec_count
                .tick(buttons, buttons & !prev_buttons, prev_buttons & !buttons);
            self.rec_cmd_count.tick(
                commands,
                commands & !prev_commands,
                prev_commands & !commands,
            );
        }
    }

    /// `BtlInput_RingPush` then `BtlInput_RingPop`, gated: `BtlInput_Fetch`. `blocked`: flags 0x11E, 0x11F or 0xB1.
    fn fetch(&mut self, gate: &InputGate, blocked: bool) -> (u32, u32, [u8; 2]) {
        if (self.ring_write + 1) % RING != self.ring_read {
            self.ring[self.ring_write] = (self.rec_buttons, self.rec_commands, self.rec_stick);
            self.ring_write = (self.ring_write + 1) % RING;
        }
        let (mut buttons, mut commands, mut stick) = if self.ring_write == self.ring_read {
            (0, 0, [STICK_CENTER; 2])
        } else {
            let entry = self.ring[self.ring_read];
            self.ring_read = (self.ring_read + 1) % RING;
            entry
        };
        let mut block = false;
        let mut filter = false;
        if gate.filter {
            if gate.stage_4_or_27 {
                block = true;
            } else {
                filter = true;
            }
        }
        if gate.block || blocked {
            block = true;
        }
        if filter {
            buttons &= FILTER_MASK;
            commands &= C_FILTER_MASK;
        }
        if block {
            buttons = 0;
            commands = 0;
            stick = [STICK_CENTER; 2];
        }
        (buttons, commands, stick)
    }

    /// `BtlInput_Update`.
    pub(crate) fn update(&mut self, gate: &InputGate, blocked: bool) {
        let (buttons, commands, stick) = self.fetch(gate, blocked);
        self.stick_prev = self.stick;
        self.stick_raw_prev = self.stick_raw;
        self.pressed = buttons & !self.held;
        self.released = self.held & !buttons;
        self.cmd_pressed = commands & !self.cmd_held;
        self.cmd_released = self.cmd_held & !commands;
        self.prev = self.held;
        self.cmd_prev = self.cmd_held;
        self.held = buttons;
        self.cmd_held = commands;
        self.stick_raw = [byte_to_stick(stick[0]), byte_to_stick(stick[1])];
        if gate.time_stopped {
            self.stick = self.stick_raw;
        } else {
            for i in 0..2 {
                self.stick[i] += (self.stick_raw[i] - self.stick[i]) * 0.5;
            }
        }
        let len = (self.stick[0] * self.stick[0] + self.stick[1] * self.stick[1]).sqrt();
        if len > 1.0 {
            let inv = 1.0 / len;
            self.stick = [self.stick[0] * inv, self.stick[1] * inv];
        }
        let moved = |s: [f32; 2]| s[0].abs() > 0.01 || s[1].abs() > 0.01;
        self.stick_turn = if moved(self.stick_raw) && moved(self.stick_raw_prev) {
            wrap_angle(
                self.stick_raw[0].atan2(self.stick_raw[1])
                    - self.stick_raw_prev[0].atan2(self.stick_raw_prev[1]),
            )
        } else {
            0.0
        };
        if !gate.time_stopped {
            self.count.tick(self.held, self.pressed, self.released);
            self.cmd_count
                .tick(self.cmd_held, self.cmd_pressed, self.cmd_released);
        }
        self.frame_bits = [0; 30];
    }

    pub(crate) fn is_pressed(&self, mask: u32) -> bool {
        self.pressed & mask != 0
    }

    pub(crate) fn is_held(&self, mask: u32) -> bool {
        self.held & mask != 0
    }

    fn is_held_not_prev(&self, mask: u32) -> bool {
        self.prev & mask == 0 && self.held & mask != 0
    }

    pub(crate) fn is_released(&self, mask: u32) -> bool {
        self.released & mask != 0
    }

    fn held_frames(&self, bit: usize) -> i8 {
        self.count.held[bit]
    }

    fn not_held_frames(&self, bit: usize) -> i8 {
        self.count.not_held[bit]
    }

    fn frames_since_press(&self, bit: usize) -> i8 {
        self.count.since_press[bit]
    }

    pub(crate) fn is_cmd_pressed(&self, mask: u32) -> bool {
        self.cmd_pressed & mask != 0
    }

    pub(crate) fn is_cmd_held(&self, mask: u32) -> bool {
        self.cmd_held & mask != 0
    }

    fn cmd_not_held_frames(&self, bit: usize) -> i8 {
        self.cmd_count.not_held[bit]
    }

    fn cmd_prev_frames_since_press(&self, bit: usize) -> i8 {
        self.cmd_count.prev_since_press[bit]
    }

    /// `BtlInput_GetStickX` / `GetStickY`: the smoothed stick.
    pub(crate) fn stick(&self) -> [f32; 2] {
        self.stick
    }

    /// `BtlInput_GetStickAngle`: atan2(x, -y) of the raw stick, 0 when it is neutral.
    pub(crate) fn stick_angle(&self) -> f32 {
        let [x, y] = self.stick_raw;
        if x.abs() < 0.01 && y.abs() < 0.01 {
            0.0
        } else {
            x.atan2(-y)
        }
    }

    /// `BtlInput_SetFrameBit`.
    fn set_frame_bit(&mut self, bit: u32, set: bool) {
        let byte = (bit >> 3) as usize;
        if let Some(b) = self.frame_bits.get_mut(usize::from(set) + byte * 2) {
            *b |= 1 << (bit & 7);
        }
    }

    /// `BtlInput_TestAction`: records the query, tests action input `id`, and gives the result when `want`, its
    /// negation otherwise. `facing_rel_cam` is `BtlAct_GetFacingRelCam` (heading minus the camera yaw).
    pub(crate) fn test_action(
        &mut self,
        flags: &mut Flags,
        facing_rel_cam: f32,
        id: u32,
        want: bool,
    ) -> bool {
        self.set_frame_bit(id, want);
        let turn_from_facing = || wrap_angle(self.stick_angle() - facing_rel_cam);
        let result = match id {
            1 => !self.is_held(R3) && self.is_held(DIR_MASK),
            2 => {
                self.not_held_frames(4) >= 3
                    && self.not_held_frames(5) >= 3
                    && self.not_held_frames(6) >= 3
                    && self.not_held_frames(7) >= 3
            }
            3 => !self.is_held(R3) && self.is_cmd_held(C_DASH_HELD),
            4 => {
                let mut r = false;
                if self.is_held(DIR_MASK) {
                    if turn_from_facing().abs() > 0.785_398_2 {
                        r = self.is_held_not_prev(DIR_MASK);
                    }
                    if self.stick_turn.abs() > 1.470_796_3 {
                        r = true;
                    }
                }
                r
            }
            5 => {
                self.is_held(DIR_MASK)
                    && turn_from_facing().abs() > 0.785_398_2
                    && self.is_cmd_held(C_DASH_HELD)
            }
            6 => {
                let mut r = false;
                if self.cmd_not_held_frames(0) >= 6 {
                    r = !self.is_held(DIR_MASK);
                }
                if self.is_held(R3) {
                    r = true;
                }
                r
            }
            7 => !self.is_held(CHARGE) && self.is_pressed(DASH),
            8 | 9 => self.is_pressed(ASCEND),
            10 => !self.is_held(DESCEND) && self.is_held(ASCEND),
            12 => !self.is_held(ASCEND) && self.is_held(DESCEND),
            14 => !self.is_held(ASCEND) && self.is_pressed(DESCEND),
            11 => !self.is_held(ASCEND) || self.is_held(DESCEND),
            13 => !self.is_held(DESCEND) || self.is_held(ASCEND),
            15 => self.is_pressed(ASCEND_TAP2),
            16 => !self.is_held(ASCEND),
            17 => self.is_pressed(DESCEND_TAP2),
            18 => !self.is_held(DESCEND),
            20 => !self.is_held(DIR_MASK) && self.is_cmd_pressed(C_DASH_CHARGE_P),
            21 => self.is_held(DIR_MASK) && self.is_cmd_pressed(C_DASH_CHARGE_P),
            22 => self.is_cmd_pressed(C_DASH_P),
            23 => !self.is_held(DIR_MASK) && self.is_cmd_held(C_DASH_CHARGE_H),
            24 => self.is_held(DIR_MASK) && self.is_cmd_held(C_DASH_CHARGE_H),
            25 => self.is_cmd_pressed(C_DASH_P2) && self.is_cmd_pressed(C_DIR_DOWN),
            26 => self.is_cmd_pressed(C_DASH_P2) && self.is_cmd_pressed(C_DIR_LEFT),
            27 => self.is_cmd_pressed(C_DASH_P2) && self.is_cmd_pressed(C_DIR_RIGHT),
            28 => self.is_cmd_pressed(C_DASH_P2) && self.is_cmd_pressed(C_DIR_UP),
            30 => self.is_cmd_pressed(C_GUARD_H) && self.is_held(LEFT),
            31 => self.is_cmd_pressed(C_GUARD_H) && self.is_held(RIGHT),
            32 => self.is_cmd_pressed(C_GUARD_H) && self.is_held(UP),
            29 => self.is_cmd_pressed(C_GUARD_H) && self.is_held(DOWN),
            49 => self.is_held(DIR_MASK) && !self.is_held(CHARGE) && self.is_cmd_pressed(C_GUARD_H),
            50 => !self.is_cmd_held(C_GUARD_H),
            51 => self.is_cmd_held(C_DIR_P),
            97 => self.is_held(RS_LEFT),
            98 => self.is_held(RS_RIGHT),
            19 => self.is_held(CHARGE),
            34 => self.is_cmd_held(C_GUARD_H),
            35 => self.is_cmd_held(C_GUARD_H) && !self.is_held(UP) && !self.is_held(DOWN),
            36 => self.is_cmd_held(C_GUARD_H) && self.is_held(UP),
            37 => self.is_cmd_held(C_GUARD_H) && self.is_held(DOWN),
            38 => self.is_pressed(UP) && self.is_pressed(RUSH),
            40 => self.is_held(DOWN),
            41 => self.is_held(UP),
            42 => self.is_cmd_pressed(C_DASH_P3),
            43 => self.is_cmd_pressed(C_GUARD_SIDE),
            44 => self.is_cmd_pressed(C_GUARD_H2),
            45 => self.is_held(DOWN) && self.held_frames(11) >= 6 && self.held_frames(12) >= 6,
            46 => {
                self.is_cmd_pressed(C_LOCKON_P) && self.cmd_prev_frames_since_press(22) < TAP_FRAMES
            }
            47 => self.is_held(DOWN) && self.is_pressed(ANY_FACE),
            48 => self.is_held(UP) && self.is_pressed(ANY_FACE),
            52 => self.is_cmd_held(C_RUSH_TAP),
            53 => self.is_cmd_pressed(C_RUSH_HOLD6) && self.is_cmd_held(C_DIR_UP),
            54 => self.is_cmd_pressed(C_RUSH_HOLD6) && self.is_cmd_held(C_DIR_DOWN),
            55 => self.is_cmd_pressed(C_RUSH_HOLD6) && self.is_cmd_held(C_DIR_LEFT),
            56 => self.is_cmd_pressed(C_RUSH_HOLD6) && self.is_cmd_held(C_DIR_RIGHT),
            57 => self.is_cmd_pressed(C_RUSH_HOLD6) && !self.is_cmd_held(C_DIR_ANY),
            58 => self.is_cmd_held(C_RUSH_H) && self.is_cmd_held(C_DIR_UP),
            59 => self.is_cmd_held(C_RUSH_H) && self.is_cmd_held(C_DIR_DOWN),
            60 => self.is_cmd_held(C_RUSH_H) && self.is_cmd_held(C_DIR_LEFT),
            61 => self.is_cmd_held(C_RUSH_H) && self.is_cmd_held(C_DIR_RIGHT),
            62 => self.is_cmd_held(C_RUSH_H) && !self.is_cmd_held(C_DIR_ANY),
            63 => self.is_cmd_held(C_RUSH_NOT_H),
            64 => !self.is_held(BLAST) && self.is_pressed(RUSH),
            65 => !self.is_held(RUSH),
            66 => !self.is_held(DIR_MASK | RUSH) && self.is_pressed(BLAST),
            77 => !self.is_held(RUSH) && self.is_held(UP) && self.is_pressed(BLAST),
            78 => !self.is_held(RUSH) && self.is_held(DOWN) && self.is_pressed(BLAST),
            79 => self.is_pressed(RUSH),
            80 => self.is_pressed(BLAST),
            82 => self.is_held(UP) && self.is_pressed(RUSH),
            83 => self.is_held(UP) && self.is_pressed(BLAST),
            84 => self.is_held(UP) && self.is_cmd_pressed(C_GUARD_H),
            85 => self.is_held(DOWN) && self.is_pressed(RUSH),
            86 => self.is_held(DOWN) && self.is_pressed(BLAST),
            87 => self.is_held(DOWN) && self.is_cmd_pressed(C_GUARD_H),
            68 => self.is_cmd_held(C_BLAST_P) && self.is_cmd_held(C_DIR_UP),
            69 => self.is_cmd_held(C_BLAST_P) && self.is_cmd_held(C_DIR_DOWN),
            70 => self.is_cmd_held(C_BLAST_P) && self.is_cmd_held(C_DIR_LEFT),
            71 => self.is_cmd_held(C_BLAST_P) && self.is_cmd_held(C_DIR_RIGHT),
            74 => self.is_cmd_held(C_RUSH_P) && self.is_cmd_held(C_DIR_UP),
            75 => self.is_cmd_held(C_RUSH_P) && self.is_cmd_held(C_DIR_DOWN),
            72 => self.is_cmd_held(C_RUSH_P) && self.is_cmd_held(C_DIR_LEFT),
            73 => self.is_cmd_held(C_RUSH_P) && self.is_cmd_held(C_DIR_RIGHT),
            76 => self.is_cmd_held(C_RUSH_P) && !self.is_cmd_held(C_DIR_ANY),
            92 => !self.is_held(CHARGE) && self.is_cmd_pressed(C_DASH_TAP2),
            93 => self.is_cmd_pressed(C_DASH_P4),
            94 => !self.is_held(CHARGE) && self.is_held(UP) && self.is_pressed(BLAST),
            95 => self.is_cmd_held(C_GUARD_H) && self.is_pressed(BLAST2),
            96 => !self.is_held(DIR_MASK) && self.is_cmd_pressed(C_GUARD_H),
            33 | 39 | 81 => self.is_cmd_pressed(C_GUARD_H),
            88 => {
                let mut r = false;
                if !self.is_held(CHARGE | RUSH) {
                    if self.is_released(BLAST) {
                        r = self.frames_since_press(2) < 7;
                    }
                    if self.is_held(BLAST) {
                        r = self.frames_since_press(2) == 6;
                    }
                }
                r
            }
            89 => {
                !self.is_held(CHARGE | RUSH)
                    && self.is_released(BLAST)
                    && self.frames_since_press(2) < 7
            }
            90 => {
                !self.is_held(CHARGE | RUSH)
                    && self.is_held(BLAST)
                    && self.frames_since_press(2) == 6
            }
            67 | 91 => !self.is_held(BLAST),
            99 => {
                self.not_held_frames(15) >= 30
                    && !self.is_held(DIR_MASK)
                    && self.is_released(R3)
                    && self.frames_since_press(13) < 13
            }
            100..=103 => {
                let dir = [LEFT, UP, RIGHT, DOWN][(id - 100) as usize];
                self.not_held_frames(15) >= 30
                    && self.is_held(dir)
                    && self.is_released(R3)
                    && self.frames_since_press(13) < 13
            }
            104..=106 => {
                let dir = [LEFT, UP, RIGHT][(id - 104) as usize];
                self.not_held_frames(15) >= 30
                    && self.is_held(dir)
                    && self.is_held(R3)
                    && self.frames_since_press(13) == 12
            }
            107 => self.is_pressed(L3R3),
            108..=110 | 113 | 114 => {
                let (cmd, forced) = match id {
                    108 => (C_CHARGE_BLAST, 0x122),
                    109 => (C_CHARGE_BLAST_U, 0x123),
                    110 => (C_CHARGE_BLAST_D, 0x124),
                    113 => (C_CHARGE_GUARD, 0x120),
                    _ => (C_CHARGE_GUARD_U, 0x121),
                };
                let mut r = self.is_cmd_held(cmd);
                if flags.test(forced) {
                    flags.clear(forced);
                    flags.clear(0x11E);
                    r = true;
                }
                r
            }
            111 => self.is_cmd_held(C_SWITCH) || flags.test(0x11F),
            112 => self.is_cmd_held(C_UNUSED2000) || flags.test(0x11F),
            _ => false,
        };
        result == want
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(input: &mut Input, held: u32) {
        let pad = PadState {
            held,
            ..PadState::default()
        };
        input.sample(&pad, true, false);
        input.update(&InputGate::default(), false);
    }

    fn input() -> Input {
        let mut input = Input::default();
        input.init(&DEFAULT_KEYS);
        input
    }

    #[test]
    fn default_keys_put_rush_on_square_and_dash_on_cross() {
        let mut input = input();
        frame(&mut input, pad::SQUARE | pad::CROSS);
        assert!(input.is_held(RUSH) && input.is_held(DASH) && !input.is_held(GUARD));
        assert!(input.is_held(ANY_FACE));
    }

    #[test]
    fn a_square_tap_is_rush_input_52_on_its_release() {
        let mut input = input();
        let mut flags = Flags::default();
        for _ in 0..3 {
            frame(&mut input, pad::SQUARE);
            assert!(!input.test_action(&mut flags, 0.0, 52, true));
        }
        frame(&mut input, 0);
        assert!(input.test_action(&mut flags, 0.0, 52, true));
        frame(&mut input, 0);
        assert!(!input.test_action(&mut flags, 0.0, 52, true));
        // Held too long, the release is no tap.
        for _ in 0..10 {
            frame(&mut input, pad::SQUARE);
        }
        frame(&mut input, 0);
        assert!(!input.test_action(&mut flags, 0.0, 52, true));
    }

    #[test]
    fn the_d_pad_moves_the_stick_and_sets_input_1() {
        let mut input = input();
        let mut flags = Flags::default();
        frame(&mut input, pad::UP);
        assert!(input.test_action(&mut flags, 0.0, 1, true));
        // The stick follows the raw value by half each frame.
        assert!((input.stick()[1] + 0.5).abs() < 0.01);
        frame(&mut input, pad::UP);
        assert!((input.stick()[1] + 0.75).abs() < 0.01);
        // Input 2 (no direction for 3 frames) comes on the third neutral frame.
        for n in 1..=3 {
            frame(&mut input, 0);
            assert_eq!(input.test_action(&mut flags, 0.0, 2, true), n == 3);
        }
        // BtlInput_SetFrameBit: the query is recorded in the "wanted true" byte of bit 2.
        assert!(input.frame_bits[1] & 4 != 0 && input.frame_bits[0] & 4 == 0);
    }

    #[test]
    fn analog_stick_sets_the_dominant_direction() {
        let mut input = input();
        let pad = PadState {
            left: stick_from_raw(0x80, 0x00),
            ..PadState::default()
        };
        input.sample(&pad, true, false);
        input.update(&InputGate::default(), false);
        assert!(input.is_held(UP) && !input.is_held(LEFT | RIGHT));
    }

    #[test]
    fn demo_flag_blocks_the_fetched_input() {
        let mut input = input();
        input.sample(
            &PadState {
                held: pad::SQUARE,
                ..PadState::default()
            },
            true,
            false,
        );
        let gate = InputGate {
            block: true,
            ..InputGate::default()
        };
        input.update(&gate, false);
        assert!(!input.is_held(RUSH));
    }

    #[test]
    fn injected_input_bypasses_the_key_config() {
        let mut input = input();
        input.inject_on = true;
        input.inject_buttons = GUARD;
        input.inject_stick = [0.0, -1.0];
        input.sample(&PadState::default(), true, false);
        input.update(&InputGate::default(), false);
        assert!(input.is_held(GUARD | ANY_FACE | UP));
        assert!(input.is_cmd_held(C_GUARD_H));
    }
}
