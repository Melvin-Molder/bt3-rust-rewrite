//! The score sheet of Ultimate Battle's result screens (`UbScore`, src/menu/ub_score.c): filled from the battle's
//! result (health left, longest combo, biggest combo, the battle time for the courses, one bonus per result event),
//! priced from the screen's table, ranked, counted into the total line by line, and converted into Z points (the
//! counter runs at a tenth of the total, then the whole total is paid in).

use crate::battle::BattleResult;
use crate::flash::Flash;
use crate::title::TitleSound;

/// `UBSCORE_PAGE`: bonus plates on a page.
pub const PAGE: usize = 3;
const BONUS_MAX: usize = 48;
/// `P_MONEY_MAX`.
pub const MONEY_MAX: i32 = 9_999_999;

/// `gUbRewardItems`: the items the result screens hand out.
const REWARD_ITEMS: [i32; 6] = [0x7E, 0x57, 0x6B, 0x6A, 0x7F, 0x80];

/// `UbScore_GetRewardItem`.
pub fn reward_item(idx: usize) -> i32 {
    REWARD_ITEMS[idx.min(REWARD_ITEMS.len() - 1)]
}

/// One line of the sheet (`UbScoreLine`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Line {
    /// What was measured (a bonus line: the bonus id).
    pub value: i32,
    /// Points not yet counted into the total.
    pub remain: i32,
    /// Points the line is worth.
    pub points: i32,
}

/// An entry of the screen's price table (`UbScorePrice`, 0x18 bytes): what a line value or a bonus is worth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Price {
    pub line: [i32; 4],
    pub bonus: i32,
}

/// Parses the price table (as many entries as the section holds, at most 101).
pub fn parse_prices(data: &[u8]) -> Vec<Price> {
    data.chunks_exact(0x18)
        .take(101)
        .map(|e| {
            let w = |k: usize| i32::from_le_bytes(e[k * 4..k * 4 + 4].try_into().unwrap());
            Price {
                line: [w(0), w(1), w(2), w(3)],
                bonus: w(4),
            }
        })
        .collect()
}

/// `UbScore`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Score {
    pub total: i32,
    /// The total as it was before it was converted.
    pub saved: i32,
    /// Z points made of the total and not yet paid out.
    pub convert: i32,
    /// Health, longest combo (hits), biggest combo (damage), battle time.
    pub line: [Line; 4],
    pub bonus: Vec<Line>,
    /// The bonus indices on the page shown.
    pub shown: [usize; PAGE],
    pub line_count: usize,
    /// The battle clock: hours (at most 99), minutes, seconds, milliseconds.
    pub clock: [i32; 4],
    /// 0 none, 1..4.
    pub rank: i32,
}

/// `gUbScoreTimeSteps[j]`: 999, 1999, ... 98999, 99989.
fn time_step(j: usize) -> i32 {
    if j == 99 {
        99989
    } else {
        j as i32 * 1000 + 999
    }
}

impl Score {
    /// `UbScore_Fill(kind, …)`: the sheet from the battle's result; returns it with the outcome (0 side 0 won, 1
    /// lost, 2 aborted: the sheet left empty) and the pages of bonus plates.
    pub fn fill(kind: i32, result: &BattleResult) -> (Self, i32, usize) {
        let outcome = if result.aborted {
            2
        } else if result.winner & 1 != 0 {
            0
        } else {
            1
        };
        let mut s = Self::default();
        if outcome == 2 {
            return (s, outcome, 0);
        }
        let health = result.health[0];
        s.line[0].value = if 0.0 < health && health < 1.0 {
            1
        } else {
            health as i32
        };
        s.line[1].value = result.max_combo_hits[0].min(100);
        s.line[2].value = result.max_combo_damage[0].min(99990);
        if kind == 2 {
            s.line[3].value = result.frames;
            s.line_count = 4;
        } else {
            s.line_count = 3;
        }
        for i in 0..BONUS_MAX {
            if result.event_summary >> i & 1 != 0 {
                s.bonus.push(Line {
                    value: i as i32,
                    ..Line::default()
                });
            }
        }
        let pages = s.bonus.len().div_ceil(PAGE);
        s.clock = result.clock;
        if s.clock[0] >= 100 {
            s.clock[0] = 99;
        }
        (s, outcome, pages)
    }

    /// `UbScore_CalcPoints`: lines 0, 1 and 3 priced by their value, line 2 by its step, the bonuses by their id.
    pub fn calc_points(&mut self, price: &[Price]) {
        let at = |n: i32| price.get(n.max(0) as usize).copied().unwrap_or_default();
        for i in 0..4 {
            let line = &mut self.line[i];
            if i != 2 {
                line.remain = at(line.value).line[i];
            } else if let Some(j) = (0..100).find(|&j| time_step(j) >= line.value) {
                line.remain = at(j as i32).line[2];
            }
            line.points = line.remain;
        }
        for b in &mut self.bonus {
            b.remain = at(b.value).bonus;
            b.points = b.remain;
        }
    }

    /// `UbScore_CalcRank`: the sheet's total, and its rank (1 the best .. 4; the limits by kind of sheet).
    pub fn calc_rank(&mut self, kind: i32) -> i32 {
        const LIMIT: [[i32; 4]; 2] = [[1599, 2199, 2799, 9999], [2099, 2499, 3199, 9999]];
        let row = usize::from(kind == 2 || kind == 4);
        let total: i32 = self.line[..self.line_count]
            .iter()
            .chain(&self.bonus)
            .map(|l| l.remain)
            .sum();
        self.rank = LIMIT[row]
            .iter()
            .position(|&l| total <= l)
            .map_or(0, |i| i as i32 + 1);
        total
    }

    /// `UbScore_CountLine`: one step of a line (or a bonus) into the total; true when the line is empty. While the
    /// mode 13 menu's cursor (`ubCursor`) is 0 the total stops at 65535.
    pub fn count_line(
        &mut self,
        bonus: bool,
        index: usize,
        step: i32,
        menu_cursor: i32,
        sounds: &mut Vec<TitleSound>,
    ) -> bool {
        let line = if bonus {
            match self.bonus.get_mut(index) {
                Some(l) => l,
                None => return true,
            }
        } else {
            &mut self.line[index.min(3)]
        };
        let done = transfer(&mut line.remain, &mut self.total, step, 1.0, sounds);
        if menu_cursor == 0 && self.total > 0xFFFF {
            self.total = 0xFFFF;
        }
        done
    }

    /// `UbScore_ConvertStep`: up to `step` points of the total into Z points, at a tenth; true when used up.
    pub fn convert_step(&mut self, step: i32, sounds: &mut Vec<TitleSound>) -> bool {
        transfer(&mut self.total, &mut self.convert, step, 0.1, sounds)
    }

    /// `UbScore_SetPage`: the bonus indices of page `page` (1-based).
    pub fn set_page(&mut self, page: usize) {
        if page > 0 {
            let first = page * PAGE - PAGE;
            for i in 0..PAGE {
                if first + i < self.bonus.len() {
                    self.shown[i] = first + i;
                } else {
                    break;
                }
            }
        }
    }
}

/// `UbScore_Transfer`: up to `step` points from `from` to `to` (scaled by `rate`), with the counting sound; true
/// when `from` is empty.
pub fn transfer(
    from: &mut i32,
    to: &mut i32,
    step: i32,
    rate: f32,
    sounds: &mut Vec<TitleSound>,
) -> bool {
    if *from <= 0 {
        return true;
    }
    sounds.push(TitleSound::Effect(2, 5));
    if *from >= step {
        *to += (step as f32 * rate) as i32;
        *from -= step;
        false
    } else {
        *to += (*from as f32 * rate) as i32;
        *from = 0;
        true
    }
}

/// `UbScore_PlateGoto`: lights or dims plate `plate` of the money conversion and its arrow.
pub fn plate_goto(flash: &mut Flash, plate: i32, on: bool) {
    let r = flash.find(None, &format!("mc_pt_convert_plate{plate}"));
    flash.clip_goto_label(r, if on { "fl_on_start" } else { "fl_off_start" });
    let r = flash.find(None, &format!("mc_blue_yajirusi{plate:02}"));
    flash.clip_goto_label(r, if on { "fl_on_loop" } else { "fl_stop" });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sheet_is_priced_and_ranked() {
        let result = BattleResult {
            winner: 1,
            health: [0.5, 0.0],
            max_combo_hits: [150, 0],
            max_combo_damage: [5500, 0],
            event_summary: 0b1011,
            clock: [120, 1, 2, 3],
            ..BattleResult::default()
        };
        let (mut s, outcome, pages) = Score::fill(1, &result);
        assert_eq!((outcome, pages, s.line_count), (0, 1, 3));
        // Below one percent of health counts as one; at most 100 hits.
        assert_eq!(
            [s.line[0].value, s.line[1].value, s.line[2].value],
            [1, 100, 5500]
        );
        assert_eq!(s.clock[0], 99);
        let price: Vec<Price> = (0..101)
            .map(|n| Price {
                line: [n, 2 * n, 3 * n, 4 * n],
                bonus: 10 * n,
            })
            .collect();
        s.calc_points(&price);
        // Damage 5500 is step 5 (first step not below it: 5999).
        assert_eq!(
            [s.line[0].points, s.line[1].points, s.line[2].points],
            [1, 200, 15]
        );
        assert_eq!(
            s.bonus.iter().map(|b| b.points).collect::<Vec<_>>(),
            [0, 10, 30]
        );
        assert_eq!(s.calc_rank(1), 256);
        assert_eq!(s.rank, 1);
        s.set_page(1);
        assert_eq!(s.shown, [0, 1, 2]);
        let mut sounds = Vec::new();
        while !s.count_line(false, 1, 15, 1, &mut sounds) {}
        assert_eq!(s.total, 200);
        assert!(!s.convert_step(0x40, &mut sounds));
        assert_eq!((s.total, s.convert), (200 - 0x40, 6));
    }

    #[test]
    fn an_abort_leaves_the_sheet_empty() {
        let result = BattleResult {
            winner: 8,
            aborted: true,
            ..BattleResult::default()
        };
        let (s, outcome, pages) = Score::fill(1, &result);
        assert_eq!((outcome, pages), (2, 0));
        assert_eq!(s, Score::default());
    }
}
