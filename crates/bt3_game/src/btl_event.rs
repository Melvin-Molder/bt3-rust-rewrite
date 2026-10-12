//! The battle events (`BtlEvent_*`, src/battle/battle_load.c): per side a bit array of the events raised this frame,
//! the frame before, and ever since the battle started. Only the story mode's script reads them: its triggers fire
//! on an event that is new this frame (raised now, not the frame before).
//!
//! The fighters raise their events into their own lists during the frame (`Chr::raise`); the battle takes them in
//! at the end of the frame, which the script sees at the start of the next frame as the original's would.

/// `BattleEventSet`.
#[derive(Clone, Copy, Debug, Default)]
struct EventSet {
    now: [u64; 2],
    prev: [u64; 2],
    held: [u64; 2],
}

/// `BattleEvents` without the script handle.
#[derive(Clone, Debug, Default)]
pub(crate) struct BattleEvents {
    sets: [EventSet; 2],
    /// `interrupt`: a script scene waits for both fighters to be interruptible (event 0x4C).
    pub interrupt: bool,
    /// `waitFlag`: the script waits for a forced action to start (events 0x4D / 0x4E drop it).
    pub wait_flag: bool,
}

/// `BtlEvent_RaiseTimeEvents`' table: the seconds of the sub clock at which time events 0..18 are raised.
const TIME_EVENTS: [i32; 19] = [
    10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 70, 80, 90, 100, 120, 140, 160, 180,
];

impl BattleEvents {
    /// `BtlEvent_Raise`'s bit (the script's reaction to events 0x5A / 0x5B is the caller's).
    pub(crate) fn raise(&mut self, side: usize, ev: u32) {
        let set = &mut self.sets[side & 1];
        let ev = ev & 0x7F;
        set.now[(ev / 64) as usize] |= 1 << (ev % 64);
    }

    /// `BtlEvent_IsNew`: raised this frame and not the frame before.
    pub(crate) fn is_new(&self, side: usize, ev: u32) -> bool {
        let set = &self.sets[side & 1];
        let (w, b) = (((ev & 0x7F) / 64) as usize, ev % 64);
        (set.now[w] & !set.prev[w]) >> b & 1 != 0
    }

    /// `BtlEvent_WasRaised`: raised at any time since the last reset.
    #[cfg(test)]
    pub(crate) fn was_raised(&self, side: usize, ev: u32) -> bool {
        let set = &self.sets[side & 1];
        (set.held[((ev & 0x7F) / 64) as usize] >> (ev % 64)) & 1 != 0
    }

    /// `BtlEventSet_Rotate` of both sides: prev = now, held |= now, now = 0.
    pub(crate) fn rotate(&mut self) {
        for set in &mut self.sets {
            set.prev = set.now;
            for i in 0..2 {
                set.held[i] |= set.now[i];
            }
            set.now = [0; 2];
        }
    }

    /// `BtlEvent_RaiseTimeEvents`: every time event whose second the sub clock has reached, on both sides.
    pub(crate) fn raise_time_events(&mut self, seconds: i32) {
        for (i, &t) in TIME_EVENTS.iter().enumerate() {
            if seconds >= t {
                self.raise(0, i as u32);
                self.raise(1, i as u32);
            }
        }
    }

    /// `BtlEvent_Reset` / `BtlEvent_ClearAll`.
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_is_new_for_one_frame_and_held_after() {
        let mut ev = BattleEvents::default();
        ev.raise(1, 0x48);
        assert!(ev.is_new(1, 0x48) && !ev.is_new(0, 0x48));
        ev.rotate();
        // Raised again the next frame: not new.
        ev.raise(1, 0x48);
        assert!(!ev.is_new(1, 0x48));
        ev.rotate();
        assert!(ev.was_raised(1, 0x48));
        ev.raise_time_events(21);
        assert!(ev.is_new(0, 2) && ev.is_new(1, 0) && !ev.is_new(0, 3));
    }
}
