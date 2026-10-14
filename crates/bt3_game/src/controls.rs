//! The keyboard and mouse layout, after Dragon Ball: Sparking! ZERO's PC defaults: its keys for an action press the
//! PS2 button that does that action in this game. The gamepad keeps the PS2 pad's own layout.
//!
//! In battle the original's default key config (`DEFAULT_KEYS`) makes Square rush, Triangle ki blast, Circle guard,
//! Cross dash, L1 lock-on, L2 ki charge, R1 ascend and R2 descend; R3 transforms (held: fuses) and L3 + R3 switches
//! members. Sparking! ZERO's number keys are single keys for this game's combinations: ki charge with guard (and up)
//! for the skills, with ki blast (and up / down) for the Blast 2s and the Ultimate Blast.

use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::MouseButton;

use crate::btl_input::pad;

/// A key or a mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Key(KeyCode),
    Mouse(MouseButton),
}

use Input::{Key, Mouse};

/// Each battle input's PS2 buttons (`pad::*`); several press a combination at once.
pub const BATTLE: [(Input, u32); 26] = [
    // Movement on WASD, the right stick on the arrows (Sparking! ZERO's camera keys).
    (Key(KeyCode::KeyW), pad::UP),
    (Key(KeyCode::KeyS), pad::DOWN),
    (Key(KeyCode::KeyA), pad::LEFT),
    (Key(KeyCode::KeyD), pad::RIGHT),
    (Key(KeyCode::ArrowUp), pad::RSTICK_UP),
    (Key(KeyCode::ArrowDown), pad::RSTICK_DOWN),
    (Key(KeyCode::ArrowLeft), pad::RSTICK_LEFT),
    (Key(KeyCode::ArrowRight), pad::RSTICK_RIGHT),
    // Melee and ki blast on the mouse.
    (Mouse(MouseButton::Left), pad::SQUARE),
    (Mouse(MouseButton::Right), pad::TRIANGLE),
    (Key(KeyCode::KeyE), pad::CIRCLE),
    (Key(KeyCode::KeyF), pad::CROSS),
    (Key(KeyCode::Space), pad::R1),
    (Key(KeyCode::ControlLeft), pad::R2),
    // Not in Sparking! ZERO's list: ki charge and lock-on.
    (Key(KeyCode::ShiftLeft), pad::L2),
    (Key(KeyCode::KeyQ), pad::L1),
    // Its transformation and switch panels: R3, and L3 + R3.
    (Key(KeyCode::KeyZ), pad::R3),
    (Key(KeyCode::KeyX), pad::L3 | pad::R3),
    // Skill 1 / 2, Blast 1 / 2 (here the Blast 2s of slots 2 / 3) and the Ultimate Blast.
    (Key(KeyCode::Digit1), pad::L2 | pad::CIRCLE),
    (Key(KeyCode::Digit2), pad::L2 | pad::UP | pad::CIRCLE),
    (Key(KeyCode::Digit3), pad::L2 | pad::TRIANGLE),
    (Key(KeyCode::Digit4), pad::L2 | pad::UP | pad::TRIANGLE),
    (Key(KeyCode::Digit5), pad::L2 | pad::DOWN | pad::TRIANGLE),
    (Key(KeyCode::Tab), pad::START),
    (Key(KeyCode::Enter), pad::START),
    (Key(KeyCode::Backspace), pad::SELECT),
];

/// The battle's pause and result menus on the keyboard: WASD or the arrows move, Space confirms (Cross), Escape or
/// Backspace goes back (Triangle), Tab or Enter is START.
pub const BATTLE_MENU: [(Input, u32); 15] = [
    (Key(KeyCode::KeyW), pad::UP),
    (Key(KeyCode::KeyS), pad::DOWN),
    (Key(KeyCode::KeyA), pad::LEFT),
    (Key(KeyCode::KeyD), pad::RIGHT),
    (Key(KeyCode::ArrowUp), pad::UP),
    (Key(KeyCode::ArrowDown), pad::DOWN),
    (Key(KeyCode::ArrowLeft), pad::LEFT),
    (Key(KeyCode::ArrowRight), pad::RIGHT),
    (Key(KeyCode::Space), pad::CROSS),
    (Mouse(MouseButton::Left), pad::CROSS),
    (Key(KeyCode::Escape), pad::TRIANGLE),
    (Key(KeyCode::Backspace), pad::TRIANGLE),
    (Mouse(MouseButton::Right), pad::TRIANGLE),
    (Key(KeyCode::Tab), pad::START),
    (Key(KeyCode::Enter), pad::START),
];

/// The menus' buttons, by name; `main` gives each its `PAD_*` bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuButton {
    Up,
    Down,
    Left,
    Right,
    Cross,
    Circle,
    Triangle,
    Start,
    L1,
    R1,
    L2,
    R2,
}

/// The menus on the keyboard: WASD or the arrows move the cursor, Space confirms (Cross), Escape or Backspace goes
/// back (Triangle), Tab or Enter is START, Q / E page left / right (L1 / R1), 1 / 3 are L2 / R2 (Sparking! ZERO's
/// previous / next), C is Circle.
pub const MENU: [(KeyCode, MenuButton); 18] = [
    (KeyCode::KeyW, MenuButton::Up),
    (KeyCode::KeyS, MenuButton::Down),
    (KeyCode::KeyA, MenuButton::Left),
    (KeyCode::KeyD, MenuButton::Right),
    (KeyCode::ArrowUp, MenuButton::Up),
    (KeyCode::ArrowDown, MenuButton::Down),
    (KeyCode::ArrowLeft, MenuButton::Left),
    (KeyCode::ArrowRight, MenuButton::Right),
    (KeyCode::Space, MenuButton::Cross),
    (KeyCode::Escape, MenuButton::Triangle),
    (KeyCode::Backspace, MenuButton::Triangle),
    (KeyCode::Tab, MenuButton::Start),
    (KeyCode::Enter, MenuButton::Start),
    (KeyCode::KeyC, MenuButton::Circle),
    (KeyCode::KeyQ, MenuButton::L1),
    (KeyCode::KeyE, MenuButton::R1),
    (KeyCode::Digit1, MenuButton::L2),
    (KeyCode::Digit3, MenuButton::R2),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The battle keys follow the original's default key config: the face buttons and shoulders do what
    /// Sparking! ZERO's key for that action does.
    #[test]
    fn battle_keys_press_the_buttons_of_their_actions() {
        use crate::btl_input::{
            ASCEND, BLAST, CHARGE, DASH, DEFAULT_KEYS, DESCEND, GUARD, Input as Btl, LOCKON, RUSH,
        };
        let mut input = Btl::default();
        input.init(&DEFAULT_KEYS);
        let bits = |i: Input| BATTLE.iter().find(|(k, _)| *k == i).unwrap().1;
        for (key, action) in [
            (Mouse(MouseButton::Left), RUSH),
            (Mouse(MouseButton::Right), BLAST),
            (Key(KeyCode::KeyE), GUARD),
            (Key(KeyCode::KeyF), DASH),
            (Key(KeyCode::Space), ASCEND),
            (Key(KeyCode::ControlLeft), DESCEND),
            (Key(KeyCode::ShiftLeft), CHARGE),
            (Key(KeyCode::KeyQ), LOCKON),
        ] {
            // Of the eight actions, only this one (the face buttons also set their own bits).
            let actions = RUSH | BLAST | GUARD | DASH | ASCEND | DESCEND | CHARGE | LOCKON;
            assert_eq!(input.remap_for_test(bits(key)) & actions, action, "{key:?}");
        }
    }

    #[test]
    fn no_key_is_listed_twice() {
        for layout in [&BATTLE[..], &BATTLE_MENU[..]] {
            let mut keys: Vec<_> = layout.iter().map(|(k, _)| format!("{k:?}")).collect();
            keys.sort();
            keys.dedup();
            assert_eq!(keys.len(), layout.len());
        }
        let mut menu: Vec<_> = MENU.iter().map(|(k, _)| format!("{k:?}")).collect();
        menu.sort();
        menu.dedup();
        assert_eq!(menu.len(), MENU.len());
    }
}
