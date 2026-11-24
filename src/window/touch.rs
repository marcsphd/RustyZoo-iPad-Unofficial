//! Preserve the original mouse UI and reserve two fingers for the camera.

use rustyzoo::app::{Command, Mouse};
use sdl3::event::Event;

type Point = (i32, i32);

const DRAG_DISTANCE: i32 = 8;
const PAN_START_DISTANCE: f32 = 12.0;
const PINCH_MIN_DISTANCE: f32 = 24.0;
const PINCH_RATIO: f32 = 1.22;
const PINCH_DOMINANCE: f32 = 1.3;
const HOLD_NS: u64 = 550_000_000;
const DOUBLE_TAP_NS: u64 = 350_000_000;

#[derive(Debug, Clone, Copy)]
pub enum Phase {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Debug, Clone, Copy)]
pub struct Contact {
    pub id: (u64, u64),
    pub phase: Phase,
    pub position: (f32, f32),
    pub time: u64,
}

impl Contact {
    pub fn event(event: &Event) -> Option<Self> {
        let phase = match event {
            Event::FingerDown { .. } => Phase::Down,
            Event::FingerMotion { .. } => Phase::Move,
            Event::FingerUp { .. } => Phase::Up,
            Event::FingerCanceled { .. } => Phase::Cancel,
            _ => return None,
        };
        let (id, position, time) = match *event {
            Event::FingerDown {
                touch_id,
                finger_id,
                x,
                y,
                timestamp,
                ..
            }
            | Event::FingerMotion {
                touch_id,
                finger_id,
                x,
                y,
                timestamp,
                ..
            }
            | Event::FingerUp {
                touch_id,
                finger_id,
                x,
                y,
                timestamp,
                ..
            }
            | Event::FingerCanceled {
                touch_id,
                finger_id,
                x,
                y,
                timestamp,
                ..
            } => ((touch_id, finger_id), (x, y), timestamp),
            _ => unreachable!(),
        };
        Some(Self {
            id,
            phase,
            position,
            time,
        })
    }
}

#[derive(Debug, Default)]
enum Gesture {
    #[default]
    Idle,
    Tap {
        at: Point,
        time: u64,
    },
    Drag,
    Camera(CameraGesture),
    UntilRelease,
}

#[derive(Debug)]
enum CameraGesture {
    Undecided { origin: Point, span: f32 },
    Pan,
    Pinch { span: f32 },
}

impl CameraGesture {
    fn start(first: Point, second: Point) -> Self {
        Self::Undecided {
            origin: midpoint(first, second),
            span: spread(first, second),
        }
    }

    fn moved(&mut self, before: Point, at: Point, other: Point, output: &mut Vec<Command>) {
        let center = midpoint(at, other);
        match *self {
            Self::Undecided { origin, span } => {
                let current_span = spread(at, other);
                let spacing_change = (current_span - span).abs();
                let translation = spread(origin, center);
                if pinch_step(span, current_span) && spacing_change >= translation * PINCH_DOMINANCE
                {
                    output.push(Command::Zoom(current_span > span));
                    *self = Self::Pinch { span: current_span };
                } else if translation >= PAN_START_DISTANCE
                    && spacing_change < translation * PINCH_DOMINANCE
                {
                    output.push(Command::Pan(origin.0 - center.0, origin.1 - center.1));
                    *self = Self::Pan;
                }
            }
            Self::Pan => {
                // SDL delivers each finger's motion separately.
                let previous = midpoint(before, other);
                if previous != center {
                    output.push(Command::Pan(previous.0 - center.0, previous.1 - center.1));
                }
            }
            Self::Pinch { ref mut span } => {
                let current_span = spread(at, other);
                if pinch_step(*span, current_span) {
                    output.push(Command::Zoom(current_span > *span));
                    *span = current_span;
                }
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct Touch {
    fingers: Vec<((u64, u64), Point)>,
    gesture: Gesture,
    last_tap: Option<(Point, u64)>,
}

fn spread(a: Point, b: Point) -> f32 {
    ((a.0 - b.0) as f32).hypot((a.1 - b.1) as f32)
}

fn midpoint(a: Point, b: Point) -> Point {
    ((a.0 + b.0) / 2, (a.1 + b.1) / 2)
}

fn pinch_step(from: f32, to: f32) -> bool {
    (to - from).abs() >= PINCH_MIN_DISTANCE
        && (to >= from * PINCH_RATIO || from >= to * PINCH_RATIO)
}

fn motion(a: Point, b: Point) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

fn move_to((x, y): Point, output: &mut Vec<Command>) {
    output.push(Command::Mouse(Mouse::Move(x, y)));
}

impl Touch {
    pub fn cancel(&mut self, output: &mut Vec<Command>) {
        if matches!(self.gesture, Gesture::Drag) {
            output.extend([Command::Mouse(Mouse::Right), Command::Mouse(Mouse::Up)]);
        }
        *self = Self::default();
    }

    pub fn handle(&mut self, contact: Contact, at: Point, output: &mut Vec<Command>) {
        let index = self.fingers.iter().position(|(id, _)| *id == contact.id);
        match (contact.phase, index) {
            (Phase::Down, None) => self.down(contact, at),
            (Phase::Move, Some(index)) => self.moved(index, at, output),
            (Phase::Up | Phase::Cancel, Some(index)) => self.up(index, contact, at, output),
            _ => {}
        }
    }

    fn down(&mut self, contact: Contact, at: Point) {
        self.fingers.push((contact.id, at));
        self.gesture = match (self.fingers.as_slice(), &self.gesture) {
            ([_], _) => Gesture::Tap {
                at,
                time: contact.time,
            },
            ([(_, first), (_, second)], Gesture::Tap { .. }) => {
                self.last_tap = None;
                Gesture::Camera(CameraGesture::start(*first, *second))
            }
            _ => return,
        };
    }

    fn moved(&mut self, index: usize, at: Point, output: &mut Vec<Command>) {
        let before = std::mem::replace(&mut self.fingers[index].1, at);
        match &mut self.gesture {
            Gesture::Tap { at: start, .. } if motion(*start, at) >= DRAG_DISTANCE => {
                move_to(*start, output);
                output.push(Command::Mouse(Mouse::Down));
                move_to(at, output);
                self.gesture = Gesture::Drag;
                self.last_tap = None;
            }
            Gesture::Drag if index == 0 => move_to(at, output),
            Gesture::Camera(camera) if index < 2 => {
                camera.moved(before, at, self.fingers[1 - index].1, output);
            }
            _ => {}
        }
    }

    fn up(&mut self, index: usize, contact: Contact, at: Point, output: &mut Vec<Command>) {
        self.fingers.remove(index);
        match self.gesture {
            Gesture::Tap { at: start, time } if matches!(contact.phase, Phase::Up) => {
                if motion(start, at) < DRAG_DISTANCE {
                    move_to(at, output);
                    if contact.time.saturating_sub(time) >= HOLD_NS {
                        output.push(Command::Mouse(Mouse::Right));
                        self.last_tap = None;
                    } else {
                        let double = self.last_tap.take().is_some_and(|(previous, time)| {
                            contact.time.saturating_sub(time) <= DOUBLE_TAP_NS
                                && motion(previous, at) < DRAG_DISTANCE
                        });
                        output.push(Command::Mouse(if double {
                            Mouse::Double
                        } else {
                            Mouse::Down
                        }));
                        output.push(Command::Mouse(Mouse::Up));
                        if !double {
                            self.last_tap = Some((at, contact.time));
                        }
                    }
                }
            }
            Gesture::Drag if index == 0 => {
                move_to(at, output);
                if matches!(contact.phase, Phase::Cancel) {
                    output.push(Command::Mouse(Mouse::Right));
                }
                output.push(Command::Mouse(Mouse::Up));
            }
            Gesture::Drag => return,
            Gesture::Camera(_) if index >= 2 => return,
            _ => {}
        }
        self.gesture = if self.fingers.is_empty() {
            Gesture::Idle
        } else {
            Gesture::UntilRelease
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(touch: &mut Touch, output: &mut Vec<Command>, id: u64, phase: Phase, at: Point) {
        touch.handle(
            Contact {
                id: (0, id),
                phase,
                position: (0.0, 0.0),
                time: 0,
            },
            at,
            output,
        );
    }

    #[test]
    fn a_camera_gesture_does_not_click_on_either_finger_release() {
        let mut touch = Touch::default();
        let mut output = Vec::new();
        for (id, phase, at) in [
            (0, Phase::Down, (0, 0)),
            (1, Phase::Down, (100, 0)),
            (1, Phase::Move, (130, 0)),
            (0, Phase::Up, (0, 0)),
            (1, Phase::Up, (130, 0)),
        ] {
            send(&mut touch, &mut output, id, phase, at);
        }
        assert_eq!(output, [Command::Zoom(true)]);
    }

    #[test]
    fn finger_spacing_changes_do_not_zoom_during_a_pan() {
        let mut touch = Touch::default();
        let mut output = Vec::new();
        for (id, phase, at) in [
            (0, Phase::Down, (0, 0)),
            (1, Phase::Down, (60, 0)),
            (1, Phase::Move, (47, 0)), // A small squeeze on contact.
            (0, Phase::Move, (0, 26)),
            (1, Phase::Move, (47, 26)),
            (0, Phase::Move, (20, 30)), // Larger squeeze after panning starts.
            (1, Phase::Move, (41, 30)),
            (0, Phase::Up, (20, 30)),
            (1, Phase::Up, (41, 30)),
        ] {
            send(&mut touch, &mut output, id, phase, at);
        }
        assert!(!output.is_empty());
        assert!(
            output
                .iter()
                .all(|command| matches!(command, Command::Pan(..)))
        );
    }

    #[test]
    fn an_intentional_pinch_zooms_in_steps_without_panning() {
        let mut touch = Touch::default();
        let mut output = Vec::new();
        for (id, phase, at) in [
            (0, Phase::Down, (10, 0)),
            (1, Phase::Down, (110, 0)),
            (0, Phase::Move, (18, 0)),
            (1, Phase::Move, (94, 0)),
            (0, Phase::Move, (42, 0)),
            (1, Phase::Move, (118, 0)),
        ] {
            send(&mut touch, &mut output, id, phase, at);
        }
        assert_eq!(
            output,
            [
                Command::Zoom(false),
                Command::Zoom(false),
                Command::Zoom(true)
            ]
        );
    }

    #[test]
    fn one_finger_can_click_or_drag() {
        let mut touch = Touch::default();
        let mut output = Vec::new();
        send(&mut touch, &mut output, 0, Phase::Down, (10, 10));
        send(&mut touch, &mut output, 0, Phase::Up, (10, 10));
        assert_eq!(
            output,
            [
                Command::Mouse(Mouse::Move(10, 10)),
                Command::Mouse(Mouse::Down),
                Command::Mouse(Mouse::Up),
            ]
        );

        output.clear();
        send(&mut touch, &mut output, 0, Phase::Down, (10, 10));
        send(&mut touch, &mut output, 0, Phase::Move, (25, 25));
        send(&mut touch, &mut output, 0, Phase::Up, (30, 30));
        assert_eq!(
            output,
            [
                Command::Mouse(Mouse::Move(10, 10)),
                Command::Mouse(Mouse::Down),
                Command::Mouse(Mouse::Move(25, 25)),
                Command::Mouse(Mouse::Move(30, 30)),
                Command::Mouse(Mouse::Up),
            ]
        );
    }
}
