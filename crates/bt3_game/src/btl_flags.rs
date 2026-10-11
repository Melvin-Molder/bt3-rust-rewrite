//! Fighter flags (src/battle/btl_char_flag.c): 0x13D numbered flags per fighter, each either "held" (up until it
//! is cleared) or "plain" (up for one frame), with the value they had one frame ago.
//!
//! The frame is cut into stages (`BtlChar_SetStage`): 1 reset, 2 sample, 3 input, 4 / 5 the action dispatcher,
//! 6..10 the main passes, 11 events, 12 end. Every write stamps the flag with the current stage; when that stage
//! comes round again the flag is "promoted": previous := current and a plain flag drops. A plain flag set in stage
//! S is therefore seen by everything from stage S of this frame to stage S - 1 of the next.

/// `BTL_FLAG_COUNT`.
pub(crate) const FLAG_COUNT: usize = 0x13D;
const FLAG_BYTES: usize = 0x28;

/// The fighter's flag block (fighter + 0x1084..0x1262).
#[derive(Clone, Debug)]
pub(crate) struct Flags {
    stage: u8,
    held: [u8; FLAG_BYTES],
    pulse: [u8; FLAG_BYTES],
    prev_held: [u8; FLAG_BYTES],
    prev_pulse: [u8; FLAG_BYTES],
    stamp: [u8; FLAG_COUNT],
}

impl Default for Flags {
    fn default() -> Self {
        Self {
            stage: 0,
            held: [0; FLAG_BYTES],
            pulse: [0; FLAG_BYTES],
            prev_held: [0; FLAG_BYTES],
            prev_pulse: [0; FLAG_BYTES],
            stamp: [0; FLAG_COUNT],
        }
    }
}

fn bit(n: u32) -> (usize, u8) {
    ((n >> 3) as usize, 1 << (n & 7))
}

impl Flags {
    /// `BtlChar_PromoteFlags`: every flag last written in `stage` takes its current value as the previous one,
    /// and a plain flag drops.
    fn promote(&mut self, stage: u8) {
        for n in 0..FLAG_COUNT {
            if self.stamp[n] == stage {
                let (i, m) = bit(n as u32);
                self.prev_held[i] = (self.prev_held[i] & !m) | (self.held[i] & m);
                self.prev_pulse[i] = (self.prev_pulse[i] & !m) | (self.pulse[i] & m);
                self.pulse[i] &= !m;
            }
        }
    }

    /// `BtlChar_SetStage`: a lower stage than the current one starts a new frame, which promotes stage 1 too.
    pub(crate) fn set_stage(&mut self, stage: u8) {
        if stage < self.stage {
            self.promote(1);
        }
        self.promote(stage);
        self.stage = stage;
    }

    /// `BtlChar_GetStage`.
    pub(crate) fn stage(&self) -> u8 {
        self.stage
    }

    /// The previous value is kept when a flag is written again before its stage came round (the frame wrapped).
    fn snapshot_held(&mut self, n: u32) {
        let (i, m) = bit(n);
        if self.stage < self.stamp[n as usize] {
            self.prev_held[i] = (self.prev_held[i] & !m) | (self.held[i] & m);
        }
    }

    /// `BtlChar_SetHeldFlag`.
    pub(crate) fn set_held(&mut self, n: u32) {
        self.snapshot_held(n);
        let (i, m) = bit(n);
        self.held[i] |= m;
        self.stamp[n as usize] = self.stage;
    }

    /// `BtlChar_ToggleHeldFlag`.
    pub(crate) fn toggle_held(&mut self, n: u32) {
        self.snapshot_held(n);
        let (i, m) = bit(n);
        self.held[i] ^= m;
        self.stamp[n as usize] = self.stage;
    }

    /// `BtlChar_ClearFlag`: drops a flag of either kind.
    pub(crate) fn clear(&mut self, n: u32) {
        self.snapshot_held(n);
        let (i, m) = bit(n);
        self.held[i] &= !m;
        self.pulse[i] &= !m;
        self.stamp[n as usize] = self.stage;
    }

    /// `BtlChar_ClearFlagRange`: `first..=last`, in either order.
    pub(crate) fn clear_range(&mut self, first: u32, last: u32) {
        for n in first.min(last)..=first.max(last) {
            self.clear(n);
        }
    }

    /// `BtlChar_SetFlag`: a plain flag.
    pub(crate) fn set(&mut self, n: u32) {
        let (i, m) = bit(n);
        if self.stage < self.stamp[n as usize] {
            self.prev_pulse[i] = (self.prev_pulse[i] & !m) | (self.pulse[i] & m);
            self.pulse[i] &= !m;
        }
        self.pulse[i] |= m;
        self.stamp[n as usize] = self.stage;
    }

    /// `BtlChar_TestFlag`: held or plain.
    pub(crate) fn test(&self, n: u32) -> bool {
        let (i, m) = bit(n);
        (self.held[i] | self.pulse[i]) & m != 0
    }

    /// `BtlChar_TestPrevFlag`.
    pub(crate) fn test_prev(&self, n: u32) -> bool {
        let (i, m) = bit(n);
        (self.prev_held[i] | self.prev_pulse[i]) & m != 0
    }

    /// `BtlChar_IsFlagRaised`: up now, not a frame ago.
    pub(crate) fn raised(&self, n: u32) -> bool {
        self.test(n) && !self.test_prev(n)
    }

    /// `BtlChar_IsFlagDropped`.
    pub(crate) fn dropped(&self, n: u32) -> bool {
        !self.test(n) && self.test_prev(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_flag_lasts_one_frame_from_its_stage() {
        let mut flags = Flags::default();
        flags.set_stage(3);
        flags.set(0x40);
        // Seen by the later stages of this frame and the earlier stages of the next one.
        for stage in [6, 11, 12, 2] {
            flags.set_stage(stage);
            assert!(flags.test(0x40), "stage {stage}");
        }
        flags.set_stage(3);
        assert!(!flags.test(0x40));
        assert!(flags.test_prev(0x40));
        assert!(flags.dropped(0x40));
    }

    #[test]
    fn held_flag_stays_until_cleared_and_reports_its_rise() {
        let mut flags = Flags::default();
        flags.set_stage(4);
        flags.set_held(5);
        assert!(flags.raised(5));
        flags.set_stage(12);
        flags.set_stage(4);
        assert!(flags.test(5) && !flags.raised(5));
        flags.clear(5);
        assert!(!flags.test(5));
    }

    #[test]
    fn rewrite_after_the_frame_wrapped_keeps_the_previous_value() {
        let mut flags = Flags::default();
        flags.set_stage(9);
        flags.set(0x5E);
        // Next frame, stage 7 writes it again before stage 9 came round: the old value becomes "previous".
        flags.set_stage(2);
        flags.set_stage(7);
        flags.set(0x5E);
        assert!(flags.test(0x5E) && flags.test_prev(0x5E));
    }

    #[test]
    fn stage_one_flags_promote_on_every_new_frame() {
        let mut flags = Flags::default();
        flags.set_stage(1);
        flags.set(0xCD);
        flags.set_stage(12);
        assert!(flags.test(0xCD));
        flags.set_stage(2);
        assert!(!flags.test(0xCD));
    }
}
