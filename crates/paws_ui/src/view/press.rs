//! Pointer ownership for a row's held feedback. Moving far enough to scroll
//! cancels the highlight for the remainder of that contact.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct PressState {
    contact: Option<Contact>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Contact {
    id: i32,
    x: f32,
    y: f32,
    cancelled: bool,
}

impl PressState {
    pub(super) fn is_pressed(self) -> bool {
        self.contact.is_some_and(|contact| !contact.cancelled)
    }

    pub(super) fn begin(&mut self, id: i32, x: f32, y: f32) {
        if self.contact.is_none() && x.is_finite() && y.is_finite() {
            self.contact = Some(Contact {
                id,
                x,
                y,
                cancelled: false,
            });
        }
    }

    pub(super) fn move_to(&mut self, id: i32, x: f32, y: f32, slop: f32) {
        if let Some(contact) = self.contact.as_mut().filter(|contact| contact.id == id) {
            if !x.is_finite() || !y.is_finite() || (x - contact.x).hypot(y - contact.y) > slop {
                contact.cancelled = true;
            }
        }
    }

    pub(super) fn end(&mut self, id: i32) {
        if self.contact.is_some_and(|contact| contact.id == id) {
            self.contact = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PressState;

    #[test]
    fn holding_with_small_movements_keeps_feedback_until_release() {
        let mut press = PressState::default();
        press.begin(1, 100.0, 50.0);
        for offset in [0.0, 1.0, 2.0, 0.0] {
            press.move_to(1, 100.0 + offset, 50.0, 8.0);
            assert!(press.is_pressed());
        }
        press.end(1);
        assert!(!press.is_pressed());
    }

    #[test]
    fn scroll_cancellation_does_not_reactivate_when_the_contact_returns() {
        let mut press = PressState::default();
        press.begin(1, 100.0, 50.0);
        press.move_to(1, 100.0, 80.0, 8.0);
        assert!(!press.is_pressed());
        press.move_to(1, 100.0, 50.0, 8.0);
        assert!(!press.is_pressed());
        press.end(1);
        press.begin(1, 100.0, 50.0);
        assert!(press.is_pressed());
    }

    #[test]
    fn other_contacts_cannot_end_or_move_the_held_row() {
        let mut press = PressState::default();
        press.begin(1, 100.0, 50.0);
        press.begin(2, 200.0, 500.0);
        press.move_to(2, 200.0, 500.0, 8.0);
        press.end(2);
        assert!(press.is_pressed());
        press.end(1);
        assert!(!press.is_pressed());
    }
}
