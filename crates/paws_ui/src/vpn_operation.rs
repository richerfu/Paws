#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VpnOperationReceipt {
    pub request_id: String,
    pub operation_id: Option<String>,
    pub label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VpnUnconfirmedOperation {
    pub message: String,
    pub receipt: VpnOperationReceipt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VpnCommandAction {
    Start,
    Stop,
    Restart,
    OwnedStop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VpnOperationPhase {
    AwaitingBridge,
    Confirming,
    Unconfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnconfirmedVpnBlocker {
    id: u64,
    action: VpnCommandAction,
    owner: Option<String>,
    operation: VpnUnconfirmedOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveVpnOperation {
    pub id: u64,
    pub action: VpnCommandAction,
    pub owner: Option<String>,
    pub phase: VpnOperationPhase,
    pub unconfirmed: Option<VpnUnconfirmedOperation>,
    recovery_from: Vec<UnconfirmedVpnBlocker>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VpnOperationFailureDisposition {
    Stale,
    Cleared,
    RecoveryStillBlocked,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct VpnOperationState {
    pub active: Option<ActiveVpnOperation>,
}

impl VpnOperationState {
    pub(crate) fn begin(&mut self, id: u64, action: VpnCommandAction) -> bool {
        self.begin_owned(id, action, None)
    }

    pub(crate) fn begin_owned(
        &mut self,
        id: u64,
        action: VpnCommandAction,
        owner: Option<String>,
    ) -> bool {
        if id == 0 || self.active.is_some() {
            return false;
        }
        self.active = Some(ActiveVpnOperation {
            id,
            action,
            owner,
            phase: VpnOperationPhase::AwaitingBridge,
            unconfirmed: None,
            recovery_from: Vec::new(),
        });
        true
    }

    pub(crate) fn mark_unconfirmed(
        &mut self,
        id: u64,
        unconfirmed: VpnUnconfirmedOperation,
    ) -> bool {
        let Some(active) = self.active.as_mut().filter(|active| active.id == id) else {
            return false;
        };
        active.phase = VpnOperationPhase::Unconfirmed;
        active.unconfirmed = Some(unconfirmed);
        true
    }

    pub(crate) fn begin_confirmation(&mut self, id: u64) -> Option<VpnUnconfirmedOperation> {
        let active = self
            .active
            .as_mut()
            .filter(|active| active.id == id && active.phase == VpnOperationPhase::Unconfirmed)?;
        let operation = active.unconfirmed.clone()?;
        active.phase = VpnOperationPhase::Confirming;
        Some(operation)
    }

    /// Replace an unknown command with an explicit stop fence. If that fence
    /// fails, the previous unknown command is restored instead of unlocking
    /// Start. A successful stop fence is authoritative for the whole chain.
    pub(crate) fn begin_recovery(&mut self, expected_id: u64, recovery_id: u64) -> bool {
        if recovery_id == 0 {
            return false;
        }
        let Some(mut previous) = self.active.take() else {
            return false;
        };
        if previous.id != expected_id
            || !matches!(
                previous.phase,
                VpnOperationPhase::Unconfirmed | VpnOperationPhase::Confirming
            )
        {
            self.active = Some(previous);
            return false;
        }
        let Some(operation) = previous.unconfirmed.take() else {
            self.active = Some(previous);
            return false;
        };
        previous.recovery_from.push(UnconfirmedVpnBlocker {
            id: previous.id,
            action: previous.action,
            owner: previous.owner,
            operation,
        });
        self.active = Some(ActiveVpnOperation {
            id: recovery_id,
            action: VpnCommandAction::Stop,
            owner: None,
            phase: VpnOperationPhase::AwaitingBridge,
            unconfirmed: None,
            recovery_from: previous.recovery_from,
        });
        true
    }

    pub(crate) fn finish_success(&mut self, id: u64) -> bool {
        if self.active.as_ref().is_some_and(|active| active.id == id) {
            self.active = None;
            true
        } else {
            false
        }
    }

    pub(crate) fn finish_failure(&mut self, id: u64) -> VpnOperationFailureDisposition {
        let Some(mut failed) = self.active.take() else {
            return VpnOperationFailureDisposition::Stale;
        };
        if failed.id != id {
            self.active = Some(failed);
            return VpnOperationFailureDisposition::Stale;
        }
        let Some(blocker) = failed.recovery_from.pop() else {
            return VpnOperationFailureDisposition::Cleared;
        };
        self.active = Some(ActiveVpnOperation {
            id: blocker.id,
            action: blocker.action,
            owner: blocker.owner,
            phase: VpnOperationPhase::Unconfirmed,
            unconfirmed: Some(blocker.operation),
            recovery_from: failed.recovery_from,
        });
        VpnOperationFailureDisposition::RecoveryStillBlocked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unconfirmed(request_id: &str) -> VpnUnconfirmedOperation {
        VpnUnconfirmedOperation {
            message: "still unknown".to_owned(),
            receipt: VpnOperationReceipt {
                request_id: request_id.to_owned(),
                operation_id: Some(format!("ability:{request_id}")),
                label: "VPN test",
            },
        }
    }

    #[test]
    fn stop_pending_is_independent_of_a_false_runtime_projection() {
        let mut state = VpnOperationState::default();
        assert!(state.begin(1, VpnCommandAction::Stop));

        // A RuntimeStatusProjection may already say `vpn_running = false`
        // while the platform stop receipt is still pending. It is
        // intentionally not an input to the operation state machine.
        let projected_vpn_running = false;
        assert!(!projected_vpn_running);
        assert_eq!(state.active.as_ref().map(|active| active.id), Some(1));
    }

    #[test]
    fn a_second_toggle_cannot_replace_an_active_operation() {
        let mut state = VpnOperationState::default();
        assert!(state.begin(1, VpnCommandAction::Stop));
        assert!(!state.begin(2, VpnCommandAction::Start));
        assert_eq!(state.active.as_ref().map(|active| active.id), Some(1));
    }

    #[test]
    fn a_stale_completion_cannot_finish_a_new_operation() {
        let mut state = VpnOperationState::default();
        assert!(state.begin(2, VpnCommandAction::Start));
        assert!(!state.finish_success(1));
        assert_eq!(state.active.as_ref().map(|active| active.id), Some(2));
    }

    #[test]
    fn unconfirmed_receipt_can_be_rechecked_and_only_its_terminal_result_clears() {
        let mut state = VpnOperationState::default();
        assert!(state.begin(1, VpnCommandAction::Stop));
        assert!(state.mark_unconfirmed(1, unconfirmed("one")));
        assert_eq!(
            state.active.as_ref().map(|active| active.phase),
            Some(VpnOperationPhase::Unconfirmed)
        );
        assert_eq!(
            state
                .begin_confirmation(1)
                .map(|operation| operation.receipt.request_id),
            Some("one".to_owned())
        );
        assert_eq!(
            state.active.as_ref().map(|active| active.phase),
            Some(VpnOperationPhase::Confirming)
        );
        assert!(state.finish_success(1));
        assert!(state.active.is_none());
    }

    #[test]
    fn an_exact_terminal_failure_releases_a_non_recovery_operation() {
        let mut state = VpnOperationState::default();
        assert!(state.begin(1, VpnCommandAction::Start));
        assert!(state.mark_unconfirmed(1, unconfirmed("one")));
        assert!(state.begin_confirmation(1).is_some());
        assert_eq!(
            state.finish_failure(1),
            VpnOperationFailureDisposition::Cleared
        );
        assert!(state.active.is_none());
    }

    #[test]
    fn a_failed_recovery_stop_restores_the_original_unknown_operation() {
        let mut state = VpnOperationState::default();
        assert!(state.begin_owned(1, VpnCommandAction::Restart, Some("session-a".to_owned())));
        assert!(state.mark_unconfirmed(1, unconfirmed("original")));
        assert!(state.begin_recovery(1, 2));
        assert_eq!(state.active.as_ref().map(|active| active.id), Some(2));

        assert_eq!(
            state.finish_failure(2),
            VpnOperationFailureDisposition::RecoveryStillBlocked
        );
        let active = state.active.as_ref().expect("original blocker restored");
        assert_eq!(active.id, 1);
        assert_eq!(active.owner.as_deref(), Some("session-a"));
        assert_eq!(active.phase, VpnOperationPhase::Unconfirmed);
        assert_eq!(
            active
                .unconfirmed
                .as_ref()
                .map(|operation| operation.receipt.request_id.as_str()),
            Some("original")
        );
    }

    #[test]
    fn stale_original_completion_does_not_clear_recovery_and_stop_success_fences_it() {
        let mut state = VpnOperationState::default();
        assert!(state.begin(1, VpnCommandAction::Start));
        assert!(state.mark_unconfirmed(1, unconfirmed("original")));
        assert!(state.begin_recovery(1, 2));

        assert!(!state.finish_success(1));
        assert_eq!(state.active.as_ref().map(|active| active.id), Some(2));
        assert!(state.finish_success(2));
        assert!(state.active.is_none());
    }
}
