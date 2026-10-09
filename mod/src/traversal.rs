//! Elevate, Double Jump and Rush as a per-frame state machine.
//!
//! Pure logic: the in-game hook feeds it the player's state each frame and
//! applies what it returns to Elden Ring's physics.

use crate::generated::{DOUBLE_JUMP, ELEVATE, RUSH, TRAVERSAL};

/// How long jump must be held in mid-air before Elevate starts.
pub const ELEVATE_HOLD: f32 = 0.3;

#[derive(Debug, Clone, Copy, Default)]
pub struct Input {
    pub dt: f32,
    pub airborne: bool,
    pub riding: bool,
    pub jump_pressed: bool,
    pub jump_held: bool,
    pub jump_held_for: f32,
    pub sprinting: bool,
    pub fp: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Output {
    /// New gravity multiplier to write, if it changes this frame.
    pub gravity: Option<f32>,
    /// New root-motion multiplier to write, if it changes this frame.
    pub motion: Option<f32>,
    /// Restart the fall timer so a slow glide doesn't count as a long fall.
    pub reset_fall: bool,
    /// FP the power uses this frame (whole points).
    pub fp_spent: i32,
    /// Sound cue to play.
    pub cue: Option<usize>,
}

#[derive(Debug, Default)]
pub struct Traversal {
    jumped_in_air: u8,
    jump_left: f32,
    rush_left: f32,
    elevating: bool,
    fp_debt: f32,
    gravity_set: bool,
    motion_set: bool,
    was_airborne: bool,
}

impl Traversal {
    pub fn frame(&mut self, i: Input) -> Output {
        let mut o = Output::default();
        if i.riding {
            return self.stop(o);
        }
        if !i.airborne {
            self.jumped_in_air = 0;
            self.elevating = false;
        }

        // Rush: a jump pressed while sprinting on the ground becomes a long, fast leap.
        let rush = &TRAVERSAL[RUSH];
        if i.jump_pressed && !i.airborne && i.sprinting && i.fp >= rush.fp_cost {
            self.rush_left = rush.duration_sec;
            o.fp_spent += rush.fp_cost;
            o.cue = Some(rush.cue);
        }
        // Double Jump: a fresh press in mid-air.
        let dj = &TRAVERSAL[DOUBLE_JUMP];
        if i.jump_pressed && i.airborne && self.was_airborne && self.jumped_in_air < dj.uses_per_airtime && i.fp >= dj.fp_cost {
            self.jumped_in_air += 1;
            self.jump_left = dj.duration_sec;
            self.elevating = false;
            o.fp_spent += dj.fp_cost;
            o.reset_fall = true;
            o.cue = Some(dj.cue);
        }
        // Elevate: keep holding jump in mid-air.
        let el = &TRAVERSAL[ELEVATE];
        let can_elevate = i.airborne && i.jump_held && i.jump_held_for >= ELEVATE_HOLD && self.jump_left <= 0.0 && i.fp > 0;
        if can_elevate && !self.elevating {
            self.elevating = true;
            o.cue = o.cue.or(Some(el.cue));
        } else if !can_elevate {
            self.elevating = false;
            self.fp_debt = 0.0;
        }
        if self.elevating {
            self.fp_debt += el.fp_per_second * i.dt;
            let whole = self.fp_debt.floor();
            self.fp_debt -= whole;
            o.fp_spent += whole as i32;
            o.reset_fall = true;
        }

        let gravity = if self.jump_left > 0.0 {
            Some(dj.gravity_multiplier)
        } else if self.elevating {
            Some(el.gravity_multiplier)
        } else {
            None
        };
        let motion = if self.rush_left > 0.0 { Some(rush.motion_multiplier) } else { None };
        self.jump_left = (self.jump_left - i.dt).max(0.0);
        self.rush_left = (self.rush_left - i.dt).max(0.0);
        self.was_airborne = i.airborne;

        match gravity {
            Some(g) => {
                o.gravity = Some(g);
                self.gravity_set = true;
            }
            None if self.gravity_set => {
                o.gravity = Some(1.0);
                self.gravity_set = false;
            }
            None => {}
        }
        match motion {
            Some(m) => {
                o.motion = Some(m);
                self.motion_set = true;
            }
            None if self.motion_set => {
                o.motion = Some(1.0);
                self.motion_set = false;
            }
            None => {}
        }
        o
    }

    fn stop(&mut self, mut o: Output) -> Output {
        if self.gravity_set {
            o.gravity = Some(1.0);
        }
        if self.motion_set {
            o.motion = Some(1.0);
        }
        *self = Traversal::default();
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn base() -> Input {
        Input { dt: DT, fp: 100, ..Default::default() }
    }

    #[test]
    fn double_jump_once_per_airtime_then_restores_gravity() {
        let mut t = Traversal::default();
        t.frame(Input { airborne: true, ..base() }); // left the ground
        let o = t.frame(Input { airborne: true, jump_pressed: true, jump_held: true, ..base() });
        assert_eq!(o.cue, Some(TRAVERSAL[DOUBLE_JUMP].cue));
        assert_eq!(o.fp_spent, TRAVERSAL[DOUBLE_JUMP].fp_cost);
        assert_eq!(o.gravity, Some(TRAVERSAL[DOUBLE_JUMP].gravity_multiplier));
        // a second press in the same airtime does nothing
        let o = t.frame(Input { airborne: true, jump_pressed: true, ..base() });
        assert_eq!(o.cue, None);
        let mut restored = false;
        for _ in 0..60 {
            if t.frame(Input { airborne: true, ..base() }).gravity == Some(1.0) {
                restored = true;
            }
        }
        assert!(restored);
        // landing resets it
        t.frame(base());
        t.frame(Input { airborne: true, ..base() });
        assert!(t.frame(Input { airborne: true, jump_pressed: true, ..base() }).cue.is_some());
    }

    #[test]
    fn ground_jump_is_not_a_double_jump() {
        let mut t = Traversal::default();
        let o = t.frame(Input { jump_pressed: true, ..base() });
        assert_eq!(o, Output::default());
    }

    #[test]
    fn elevate_drains_fp_and_resets_fall() {
        let mut t = Traversal::default();
        t.frame(Input { airborne: true, ..base() });
        let mut spent = 0;
        let mut started = false;
        for k in 0..120 {
            let o = t.frame(Input { airborne: true, jump_held: true, jump_held_for: 0.3 + k as f32 * DT, ..base() });
            started |= o.cue == Some(TRAVERSAL[ELEVATE].cue);
            assert!(o.reset_fall);
            assert_eq!(o.gravity, Some(TRAVERSAL[ELEVATE].gravity_multiplier));
            spent += o.fp_spent;
        }
        assert!(started);
        let expect = (TRAVERSAL[ELEVATE].fp_per_second * 2.0) as i32;
        assert!((spent - expect).abs() <= 1, "spent {spent}, expected about {expect}");
        assert_eq!(t.frame(Input { airborne: true, ..base() }).gravity, Some(1.0));
    }

    #[test]
    fn rush_from_sprint_and_not_without_fp() {
        let mut t = Traversal::default();
        let o = t.frame(Input { jump_pressed: true, sprinting: true, ..base() });
        assert_eq!(o.motion, Some(TRAVERSAL[RUSH].motion_multiplier));
        assert_eq!(o.fp_spent, TRAVERSAL[RUSH].fp_cost);
        let mut t = Traversal::default();
        let o = t.frame(Input { jump_pressed: true, sprinting: true, fp: 0, ..base() });
        assert_eq!(o.motion, None);
    }

    #[test]
    fn riding_turns_everything_off() {
        let mut t = Traversal::default();
        t.frame(Input { jump_pressed: true, sprinting: true, ..base() });
        let o = t.frame(Input { riding: true, ..base() });
        assert_eq!(o.motion, Some(1.0));
        assert_eq!(t.frame(Input { riding: true, ..base() }), Output::default());
    }
}
