//! Character change requests and the time-stop word (src/battle/btl_change.c): a fighter that transforms (or
//! needs an extra object) pushes a request; the loader serves one at a time, through the states idle -> started
//! (`BtlChange_Update` made it current) -> taken (the loader has its job) -> loaded (the files are in memory) ->
//! ready (the fighter allows the swap) -> done (the fighter is finished; the next update retires it). The
//! time-stop word (`BtlChars_IsTimeStopped`) is set on every frame that ends with a request active.

/// `BTL_CHANGE_RING`: a ring of 8 slots holds 7 requests.
const RING: usize = 8;

/// `BTL_CHANGE_*` states.
pub(crate) const IDLE: u8 = 0;
pub(crate) const STARTED: u8 = 1;
pub(crate) const TAKEN: u8 = 2;
pub(crate) const LOADED: u8 = 3;
pub(crate) const READY: u8 = 4;
pub(crate) const DONE: u8 = 5;

/// `BtlChangeReq`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ChangeReq {
    pub player: usize,
    /// `BTL_CHANGE_KIND_CHARA` (false) or `BTL_CHANGE_KIND_OBJECT` (true).
    pub object: bool,
    /// The character (or the object's id), costume and variant.
    pub chara: i32,
    pub costume: i32,
    pub variant: i32,
    /// The characters whose animation files and voice bank load (all < 0: the model only).
    pub anim_chara: i32,
    pub anim1_chara: i32,
    pub voice_chara: i32,
    pub slot: i32,
}

/// `BtlChangeQueue` (roster + 0x138).
#[derive(Clone, Debug, Default)]
pub(crate) struct ChangeQueue {
    req: [ChangeReq; RING],
    read: usize,
    write: usize,
    cur: Option<ChangeReq>,
    state: u8,
    loaded: bool,
    /// `timeStop` (roster + 0x274).
    pub time_stop: bool,
}

impl ChangeQueue {
    /// `BtlChange_Alloc` + the request: nothing when the ring is full.
    fn push(&mut self, req: ChangeReq) {
        let w = self.write;
        if (w + 1) % RING == self.read {
            return;
        }
        self.write = (w + 1) % RING;
        self.req[w] = req;
    }

    /// `BtlChange_Pop`.
    fn pop(&mut self) -> Option<ChangeReq> {
        if self.write == self.read {
            return None;
        }
        let r = self.read;
        self.read = (r + 1) % RING;
        Some(self.req[r])
    }

    /// `BtlChange_Update`: retires a finished request, starts the next one, and sets the time-stop word while one
    /// is active.
    pub(crate) fn update(&mut self) {
        if self.state == DONE {
            self.state = IDLE;
            self.loaded = false;
            self.cur = None;
        }
        if self.cur.is_none() {
            self.cur = self.pop();
            if self.cur.is_none() {
                self.time_stop = false;
            } else {
                self.state = STARTED;
                self.loaded = false;
                self.time_stop = true;
            }
        } else {
            self.time_stop = true;
        }
    }

    /// `BtlChange_RequestChara`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn request_chara(
        &mut self,
        player: usize,
        chara: i32,
        costume: i32,
        variant: i32,
        anim_chara: i32,
        anim1_chara: i32,
        voice_chara: i32,
    ) {
        self.push(ChangeReq {
            player,
            object: false,
            chara,
            costume,
            variant,
            anim_chara,
            anim1_chara,
            voice_chara,
            slot: 0,
        });
    }

    /// `BtlChange_RequestObject`.
    pub(crate) fn request_object(
        &mut self,
        player: usize,
        id: i32,
        costume: i32,
        variant: i32,
        slot: i32,
    ) {
        self.push(ChangeReq {
            player,
            object: true,
            chara: id,
            costume,
            variant,
            anim_chara: -1,
            anim1_chara: -1,
            voice_chara: -1,
            slot,
        });
    }

    /// `BtlChange_IsPendingType0` / `Type1`: the started request of `player` of that kind.
    pub(crate) fn pending(&self, player: usize, object: bool) -> Option<ChangeReq> {
        self.cur
            .filter(|c| c.object == object && c.player == player && self.state == STARTED)
    }

    /// `BtlChange_SetTaken`.
    pub(crate) fn set_taken(&mut self) {
        if self.cur.is_some() {
            self.state = TAKEN;
        }
    }

    /// `BtlChange_SetLoaded`.
    pub(crate) fn set_loaded(&mut self) {
        if self.cur.is_some() {
            self.state = LOADED;
            self.loaded = true;
        }
    }

    /// `BtlChange_IsLoadedFor`.
    pub(crate) fn is_loaded_for(&self, player: usize) -> bool {
        self.cur.is_some_and(|c| c.player == player) && self.loaded
    }

    /// `BtlChange_IsReady`: state 4, not paused.
    pub(crate) fn is_ready(&self, paused: bool) -> bool {
        self.cur.is_some() && !paused && self.state == READY
    }

    /// `BtlChange_SetReady`.
    pub(crate) fn set_ready(&mut self, player: usize) {
        if self.cur.is_some_and(|c| c.player == player) {
            self.state = READY;
        }
    }

    /// `BtlChange_SetDone`.
    pub(crate) fn set_done(&mut self, player: usize) {
        if self.cur.is_some_and(|c| c.player == player) {
            self.state = DONE;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_runs_through_its_states_and_stops_time() {
        let mut q = ChangeQueue::default();
        q.update();
        assert!(!q.time_stop);
        q.request_chara(1, 5, 0, 0, 5, 5, 5);
        assert!(q.pending(1, false).is_none());
        q.update();
        assert!(q.time_stop);
        assert_eq!(q.pending(1, false).map(|r| r.chara), Some(5));
        assert!(q.pending(0, false).is_none());
        q.set_taken();
        assert!(!q.is_loaded_for(1));
        q.set_loaded();
        assert!(q.is_loaded_for(1) && !q.is_loaded_for(0));
        q.set_ready(0);
        assert!(!q.is_ready(false));
        q.set_ready(1);
        assert!(q.is_ready(false) && !q.is_ready(true));
        q.set_done(1);
        q.update();
        assert!(!q.time_stop);
    }

    #[test]
    fn the_ring_holds_seven_requests() {
        let mut q = ChangeQueue::default();
        for i in 0..9 {
            q.request_object(0, i, 0, 0, 2);
        }
        let mut n = 0;
        while q.pop().is_some() {
            n += 1;
        }
        assert_eq!(n, 7);
    }
}
