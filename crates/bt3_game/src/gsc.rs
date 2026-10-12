//! The GSC script engine (`Gsc_*`, src/sys/gsc.c): a cooperative machine that runs the actions of loaded script
//! files as tasks, one step a frame. The engine knows no command: each command word goes to the handler table
//! (`GscCommands`), which reads its operands and options through the engine and answers per phase whether it is
//! done. A command that is not done stops its task for the frame and adds 10 to the task's timer.
//!
//! The running list is kept in the original's order: a new task goes to the front, so `update` (which walks the
//! list from the front, the next task taken before each step) does not step a task started during the walk,
//! except that starting an action from a script steps it once at once.

use bt3_data::gsc::{
    GscScript, WORD_ARG, WORD_CALL, WORD_CALL_WAIT, WORD_COMMAND, WORD_END, WORD_OPERAND,
    WORD_OPTION,
};

/// First frame; done at once when true.
pub(crate) const PHASE_BEGIN: u32 = 0;
/// The command is over.
pub(crate) const PHASE_END: u32 = 1;
/// Every frame, the first included; true = done.
pub(crate) const PHASE_UPDATE: u32 = 2;
/// Replaces `PHASE_BEGIN` once the task has been told to abort.
pub(crate) const PHASE_BEGIN_ABORT: u32 = 4;

/// `GSC_TIME_STEP`.
const TIME_STEP: i32 = 10;

/// The command handlers (`gBtlScriptCommands`): `command(gsc, id, phase)` with the task being stepped current;
/// true for a non-zero return. A table without the id is the implementor's to answer.
pub(crate) trait GscCommands {
    fn command(&mut self, gsc: &mut Gsc, id: u32, phase: u32) -> bool;
}

/// `GscFile`.
#[derive(Clone, Debug)]
struct File {
    handle: i32,
    task_serial: i32,
    script: GscScript,
}

/// `GscTask`.
#[derive(Clone, Debug)]
struct Task {
    id: i32,
    action: i32,
    /// Index of the action in its file (`None`: the file has no such action, so it ends at once).
    act: Option<usize>,
    pc: usize,
    operand: usize,
    flags: i32,
    time: i32,
    child: i32,
    abort: bool,
    /// `work[0x20]`: the handlers' own words.
    work: [i32; 8],
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Gsc {
    files: Vec<File>,
    file_serial: i32,
    /// The running tasks, front first.
    tasks: Vec<Task>,
    /// `gGscCurTask`: the id of the task being stepped.
    cur: Option<i32>,
    /// The work handed out without a current task.
    scratch: [i32; 8],
}

/// `Gsc_GetFileHandle`.
fn file_handle(id: i32) -> i32 {
    id & 0xFFFF_0000u32 as i32
}

impl Gsc {
    /// `Gsc_LoadFile`: registers a script and returns its handle (`serial << 16`).
    pub(crate) fn load_file(&mut self, script: GscScript) -> i32 {
        let handle = self.file_serial << 16;
        self.file_serial += 1;
        self.files.push(File {
            handle,
            task_serial: 0,
            script,
        });
        handle
    }

    /// `Gsc_UnloadFile`: kills the file's tasks and releases it.
    #[cfg(test)]
    pub(crate) fn unload_file(&mut self, handle: i32) {
        if let Some(i) = self.files.iter().position(|f| f.handle == handle) {
            self.tasks.retain(|t| file_handle(t.id) != handle);
            self.files.remove(i);
        }
    }

    fn file(&self, handle: i32) -> Option<&File> {
        self.files.iter().find(|f| f.handle == handle)
    }

    fn task_index(&self, id: i32) -> Option<usize> {
        self.tasks.iter().position(|t| t.id == id)
    }

    /// The running tasks' ids and actions, front first.
    pub(crate) fn tasks(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.tasks.iter().map(|t| (t.id, t.action))
    }

    /// `Gsc_IsTaskAlive`.
    pub(crate) fn is_task_alive(&self, id: i32) -> bool {
        self.task_index(id).is_some()
    }

    /// `Gsc_GetTaskCount`.
    #[cfg(test)]
    pub(crate) fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// `Gsc_GetTaskTime`: 10 a frame the task's command has waited, 0 without the task.
    pub(crate) fn task_time(&self, id: i32) -> i32 {
        self.task_index(id).map_or(0, |i| self.tasks[i].time)
    }

    /// `Gsc_HasAction`.
    #[cfg(test)]
    pub(crate) fn has_action(&self, handle: i32, action: i32) -> bool {
        self.file(handle)
            .is_some_and(|f| f.script.action(action).is_some())
    }

    /// `Gsc_StartAction`: a new task at the front of the list; its id is the file's handle and task serial.
    pub(crate) fn start_action(&mut self, handle: i32, action: i32) -> i32 {
        let Some(file) = self.files.iter_mut().find(|f| f.handle == handle) else {
            return 0;
        };
        let id = handle | (file.task_serial & 0xFFFF);
        file.task_serial += 1;
        let act = file.script.action(action);
        self.tasks.insert(
            0,
            Task {
                id,
                action,
                act,
                pc: 0,
                operand: 0,
                flags: 0,
                time: 0,
                child: 0,
                abort: false,
                work: [0; 8],
            },
        );
        id
    }

    /// `Gsc_StartMain`.
    pub(crate) fn start_main(&mut self, handle: i32) -> i32 {
        self.start_action(handle, -1)
    }

    /// `Gsc_RunAction`: starts an action and steps it until it has ended.
    pub(crate) fn run_action(&mut self, handle: i32, action: i32, h: &mut dyn GscCommands) {
        let id = self.start_action(handle, action);
        while self.is_task_alive(id) {
            self.step_task(id, h);
        }
    }

    /// `Gsc_KillTask`.
    pub(crate) fn kill_task(&mut self, id: i32) {
        if let Some(i) = self.task_index(id) {
            self.tasks.remove(i);
        }
    }

    /// `Gsc_AbortTask`: its commands begin with `PHASE_BEGIN_ABORT` from now on.
    pub(crate) fn abort_task(&mut self, id: i32) {
        if let Some(i) = self.task_index(id) {
            self.tasks[i].abort = true;
        }
    }

    /// `Gsc_Update`: every running task stepped once.
    pub(crate) fn update(&mut self, h: &mut dyn GscCommands) {
        let mut next = self.tasks.first().map(|t| t.id);
        while let Some(id) = next {
            let Some(i) = self.task_index(id) else {
                break;
            };
            next = self.tasks.get(i + 1).map(|t| t.id);
            self.step_task(id, h);
        }
    }

    /// Word `at` of task `i`'s action.
    fn word(&self, i: usize, at: usize) -> u32 {
        let t = &self.tasks[i];
        match (t.act, self.file(file_handle(t.id))) {
            (Some(act), Some(f)) => f.script.word(act, at),
            _ => WORD_END,
        }
    }

    /// `Gsc_NextCommand` for task `i`.
    fn next_command(&self, i: usize) -> usize {
        let t = &self.tasks[i];
        match (t.act, self.file(file_handle(t.id))) {
            (Some(act), Some(f)) => f.script.next_command(act, t.pc),
            _ => t.pc + 1,
        }
    }

    /// Moves task `id` on to its next word, the command state cleared.
    fn advance(&mut self, id: i32, to: Option<usize>) {
        if let Some(i) = self.task_index(id) {
            let t = &mut self.tasks[i];
            t.pc = to.unwrap_or(t.pc + 1);
            t.flags = 0;
            t.time = 0;
        }
    }

    /// `Gsc_StepTask`: runs a task until a command has to wait or its action ends.
    pub(crate) fn step_task(&mut self, id: i32, h: &mut dyn GscCommands) {
        self.cur = Some(id);
        loop {
            let Some(i) = self.task_index(id) else {
                break;
            };
            let word = self.word(i, self.tasks[i].pc);
            match word & 0xF {
                WORD_END => {
                    self.tasks.remove(i);
                    break;
                }
                WORD_CALL => {
                    self.call_action((word >> 16) as u16 as i16 as i32, h);
                    self.advance(id, None);
                }
                WORD_CALL_WAIT => {
                    let action = (word >> 16) as u16 as i16 as i32;
                    if self.tasks[i].flags & 1 == 0 {
                        let child = self.call_action(action, h);
                        let Some(i) = self.task_index(id) else {
                            break;
                        };
                        self.tasks[i].child = child;
                        self.tasks[i].flags |= 1;
                    }
                    let Some(i) = self.task_index(id) else {
                        break;
                    };
                    let child = self.tasks[i].child;
                    if self.tasks[i].abort {
                        self.abort_task(child);
                    }
                    if self.is_task_alive(child) {
                        break;
                    }
                    self.advance(id, None);
                }
                WORD_COMMAND => {
                    let cmd = word >> 16;
                    let mut done = false;
                    let t = &self.tasks[i];
                    let begin = if t.abort {
                        (t.flags & 2 == 0).then_some((PHASE_BEGIN_ABORT, 2))
                    } else {
                        (t.flags & 1 == 0).then_some((PHASE_BEGIN, 1))
                    };
                    if let Some((phase, flag)) = begin {
                        self.reset_operands();
                        done = h.command(self, cmd, phase);
                        if let Some(i) = self.task_index(id) {
                            self.tasks[i].flags |= flag;
                        }
                    }
                    if !self.is_task_alive(id) {
                        break;
                    }
                    if !done {
                        self.reset_operands();
                        done = h.command(self, cmd, PHASE_UPDATE);
                    }
                    let Some(i) = self.task_index(id) else {
                        break;
                    };
                    if done {
                        self.reset_operands();
                        h.command(self, cmd, PHASE_END);
                        let Some(i) = self.task_index(id) else {
                            break;
                        };
                        let next = self.next_command(i);
                        self.advance(id, Some(next));
                    } else {
                        self.tasks[i].time += TIME_STEP;
                        break;
                    }
                }
                // No other word starts a command in the files; the original would spin on it.
                _ => self.advance(id, None),
            }
        }
        self.cur = None;
    }

    /// `Gsc_CallAction`: from inside a task, another action of its file started and stepped once.
    pub(crate) fn call_action(&mut self, action: i32, h: &mut dyn GscCommands) -> i32 {
        let cur = self.cur;
        let handle = file_handle(cur.unwrap_or(0));
        let id = self.start_action(handle, action);
        self.step_task(id, h);
        self.cur = cur;
        id
    }

    fn cur_index(&self) -> Option<usize> {
        self.cur.and_then(|id| self.task_index(id))
    }

    /// `Gsc_GetCurrentTime`.
    pub(crate) fn current_time(&self) -> i32 {
        self.cur.map_or(0, |id| self.task_time(id))
    }

    /// `Gsc_IsCurrentBegun`.
    pub(crate) fn is_current_begun(&self) -> bool {
        self.cur_index()
            .is_some_and(|i| self.tasks[i].flags & 1 != 0)
    }

    /// The current task's work words.
    pub(crate) fn work(&mut self) -> &mut [i32; 8] {
        match self.cur_index() {
            Some(i) => &mut self.tasks[i].work,
            None => &mut self.scratch,
        }
    }

    /// `Gsc_ResetOperands`: the cursor on the word after the command.
    fn reset_operands(&mut self) {
        if let Some(i) = self.cur_index() {
            self.tasks[i].operand = self.tasks[i].pc + 1;
        }
    }

    #[cfg(test)]
    fn cur_word(&self, at: usize) -> u32 {
        self.cur_index().map_or(WORD_END, |i| self.word(i, at))
    }

    /// `GscTask_NextOperand`: the operand word at the cursor (moved past it), if it is one.
    fn next_operand(&mut self) -> Option<u32> {
        let i = self.cur_index()?;
        let word = self.word(i, self.tasks[i].operand);
        (word & 0xF == WORD_OPERAND).then(|| {
            self.tasks[i].operand += 1;
            word
        })
    }

    fn value(&self, word: u32) -> [u8; 4] {
        self.cur
            .and_then(|id| self.file(file_handle(id)))
            .and_then(|f| f.script.value(word))
            .unwrap_or([0; 4])
    }

    /// `Gsc_GetIntOr`.
    pub(crate) fn int_or(&mut self, def: i32) -> i32 {
        self.next_operand()
            .map_or(def, |w| i32::from_le_bytes(self.value(w)))
    }

    /// `Gsc_GetInt` (0 past the operands, where the original reads address 0).
    pub(crate) fn int(&mut self) -> i32 {
        self.int_or(0)
    }

    /// `Gsc_GetFloatOr`.
    pub(crate) fn float_or(&mut self, def: f32) -> f32 {
        self.next_operand()
            .map_or(def, |w| f32::from_le_bytes(self.value(w)))
    }

    /// `Gsc_GetFloat`.
    pub(crate) fn float(&mut self) -> f32 {
        self.float_or(0.0)
    }

    /// `Gsc_GetStringOr`: the string's bytes up to its NUL.
    #[cfg(test)]
    pub(crate) fn string_or(&mut self, def: &[u8]) -> Vec<u8> {
        match self.next_operand() {
            Some(w) => self
                .cur
                .and_then(|id| self.file(file_handle(id)))
                .map_or(Vec::new(), |f| f.script.string(w).to_vec()),
            None => def.to_vec(),
        }
    }

    /// `Gsc_GetString`.
    #[cfg(test)]
    pub(crate) fn string(&mut self) -> Vec<u8> {
        self.string_or(&[])
    }

    /// The option words of the current command: (word index, letter, operand count).
    fn options(&self) -> Vec<(usize, u8, usize)> {
        let Some(i) = self.cur_index() else {
            return Vec::new();
        };
        let pc = self.tasks[i].pc;
        let mut w = pc + ((self.word(i, pc) >> 8) & 0xFF) as usize + 1;
        let mut out = Vec::new();
        loop {
            let word = self.word(i, w);
            if word & WORD_ARG == 0 {
                break;
            }
            if word & 0xF == WORD_OPTION {
                let n = ((word >> 16) & 0xFF) as usize;
                out.push((w, (word >> 8) as u8, n));
                w += n + 1;
            } else {
                w += 1;
            }
        }
        out
    }

    /// `Gsc_FindOptionN`: the cursor on the operands of the n-th option with this letter.
    pub(crate) fn find_option_n(&mut self, letter: u8, n: usize) -> bool {
        let found = self.options().into_iter().filter(|o| o.1 == letter).nth(n);
        match (found, self.cur_index()) {
            (Some((w, ..)), Some(i)) => {
                self.tasks[i].operand = w + 1;
                true
            }
            _ => false,
        }
    }

    /// `Gsc_FindOption`.
    pub(crate) fn find_option(&mut self, letter: u8) -> bool {
        self.find_option_n(letter, 0)
    }

    /// `Gsc_CountOptions`.
    #[cfg(test)]
    pub(crate) fn count_options(&self) -> usize {
        self.options().len()
    }

    /// `Gsc_CountOption`.
    #[cfg(test)]
    pub(crate) fn count_option(&self, letter: u8) -> usize {
        self.options().iter().filter(|o| o.1 == letter).count()
    }

    /// The command word the current task is on (for tests).
    #[cfg(test)]
    fn current_word(&self) -> u32 {
        self.cur_index()
            .map_or(WORD_END, |i| self.cur_word(self.tasks[i].pc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bt3_data::gsc::GscAction;

    const fn cmd(id: u32, operands: u32) -> u32 {
        (id << 16) | (operands << 8) | WORD_COMMAND
    }
    const fn operand(index: u32) -> u32 {
        (index << 8) | WORD_OPERAND
    }
    const fn option(letter: u8, operands: u32) -> u32 {
        (operands << 16) | ((letter as u32) << 8) | WORD_OPTION
    }

    /// Command 1 "wait n frames" (done once its time reaches n * 10), command 2 records its operands and
    /// options, command 3 records each phase.
    #[derive(Default)]
    struct Log {
        seen: Vec<String>,
    }

    impl GscCommands for Log {
        fn command(&mut self, gsc: &mut Gsc, id: u32, phase: u32) -> bool {
            match (id, phase) {
                (1, PHASE_BEGIN) => false,
                (1, PHASE_UPDATE) => {
                    let n = gsc.int();
                    gsc.current_time() >= n * 10
                }
                (2, PHASE_BEGIN) => {
                    let a = gsc.int();
                    let b = gsc.float();
                    let l = gsc.find_option(b'l').then(|| gsc.int());
                    let w = gsc.find_option(b'w');
                    let s = String::from_utf8(gsc.string()).unwrap_or_default();
                    self.seen.push(format!(
                        "2: {a} {b} {l:?} {w} {} {s:?} {}",
                        gsc.count_options(),
                        gsc.current_word() >> 16
                    ));
                    true
                }
                (3, p) => {
                    self.seen.push(format!("3: phase {p}"));
                    p == PHASE_UPDATE
                }
                _ => true,
            }
        }
    }

    fn script() -> GscScript {
        let mut data = Vec::new();
        data.extend(7i32.to_le_bytes());
        data.extend(2.5f32.to_le_bytes());
        data.extend(3i32.to_le_bytes());
        data.extend(1i32.to_le_bytes());
        GscScript {
            actions: vec![
                GscAction {
                    id: -1,
                    words: vec![
                        cmd(2, 2),
                        operand(0),
                        operand(1),
                        option(b'l', 1),
                        operand(2),
                        option(b'w', 0),
                        (5 << 16) | WORD_CALL_WAIT,
                        cmd(3, 0),
                        WORD_END,
                    ],
                },
                GscAction {
                    id: 5,
                    words: vec![cmd(1, 1), operand(3), WORD_END],
                },
            ],
            data,
        }
    }

    #[test]
    fn tasks_run_commands_wait_and_call_actions() {
        let mut gsc = Gsc::default();
        let mut log = Log::default();
        let handle = gsc.load_file(script());
        let main = gsc.start_main(handle);
        assert_eq!(main, handle);
        // Frame 1: command 2 reads its operands and options and is done; the call starts action 5 (stepped
        // once: its wait has begun) and waits for it.
        gsc.update(&mut log);
        assert_eq!(log.seen, ["2: 7 2.5 Some(3) true 2 \"\" 2"]);
        assert_eq!(gsc.task_count(), 2);
        let child = main | 1;
        assert_eq!(gsc.task_time(child), 10);
        // The child is at the front: a frame later its wait is done, and the main task goes on to command 3.
        gsc.update(&mut log);
        assert!(!gsc.is_task_alive(child));
        assert_eq!(&log.seen[1..], ["3: phase 0", "3: phase 2", "3: phase 1"]);
        assert!(!gsc.is_task_alive(main));
        // An abort begins a command with phase 4; running an action to its end steps it until it is gone.
        let id = gsc.start_main(handle);
        gsc.abort_task(id);
        log.seen.clear();
        gsc.step_task(id, &mut log);
        assert!(gsc.is_task_alive(id));
        assert!(gsc.is_task_alive(id | 1) || gsc.task_count() == 2);
        gsc.kill_task(id);
        gsc.run_action(handle, 5, &mut log);
        // Only the aborted task's child is left.
        assert_eq!(gsc.task_count(), 1);
        assert!(gsc.has_action(handle, 5) && !gsc.has_action(handle, 6));
        gsc.unload_file(handle);
        assert!(!gsc.has_action(handle, 5));
    }
}
