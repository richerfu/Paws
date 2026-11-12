//! One command session owns import preparation. Cancellation never interrupts
//! a commit that has acquired the intent under this session's lock.
use crate::AppError;
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::watch;

const IMPORT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_SAFE_ID: u64 = 9_007_199_254_740_991;

#[derive(Clone)]
pub(crate) struct ImportSession {
    state: Arc<Mutex<SessionState>>,
    timeout: Duration,
}
#[derive(Default)]
struct SessionState {
    closed: bool,
    latest: u64,
    cancelled_through: u64,
    current: Option<ActiveImport>,
}
struct ActiveImport {
    id: u64,
    committing: bool,
    cancel: watch::Sender<bool>,
}
pub(crate) struct ImportPermit {
    session: ImportSession,
    id: u64,
    cancel: watch::Receiver<bool>,
}
impl Default for ImportSession {
    fn default() -> Self {
        Self {
            state: Arc::default(),
            timeout: IMPORT_TIMEOUT,
        }
    }
}
impl ImportSession {
    fn validate_id(id: u64) -> Result<(), AppError> {
        if id == 0 || id > MAX_SAFE_ID {
            return Err(AppError::InvalidRequest(
                "import intent requires a positive safe integer".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn begin(&self, id: u64) -> Result<ImportPermit, AppError> {
        Self::validate_id(id)?;
        let mut state = self.state.lock().map_err(|_| AppError::Poisoned)?;
        if state.closed || id <= state.cancelled_through {
            return Err(AppError::ImportCancelled);
        }
        if id <= state.latest {
            return Err(AppError::InvalidRequest("import intent is obsolete".into()));
        }
        if let Some(active) = &state.current {
            if !active.committing {
                active.cancel.send_replace(true);
            }
        }
        let (cancel, receiver) = watch::channel(false);
        state.latest = id;
        state.current = Some(ActiveImport {
            id,
            committing: false,
            cancel,
        });
        Ok(ImportPermit {
            session: self.clone(),
            id,
            cancel: receiver,
        })
    }
    pub(crate) fn cancel(&self, id: u64) -> Result<bool, AppError> {
        Self::validate_id(id)?;
        let mut state = self.state.lock().map_err(|_| AppError::Poisoned)?;
        state.cancelled_through = state.cancelled_through.max(id);
        if let Some(active) = &state.current {
            if active.id <= id {
                if active.committing {
                    return Ok(false);
                }
                active.cancel.send_replace(true);
                return Ok(true);
            }
        }
        // Keep a high-water mark rather than expiring tombstones. A very late
        // start must still be rejected after its UI has already cancelled it.
        Ok(id > state.latest)
    }
    pub(crate) fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            if let Some(active) = &state.current {
                if !active.committing {
                    active.cancel.send_replace(true);
                }
            }
        }
    }
}
impl ImportPermit {
    pub(crate) async fn commit<T>(
        &self,
        operation: impl Future<Output = Result<T, paws_model::PawsError>>,
    ) -> Result<T, AppError> {
        self.begin_commit()?;
        operation.await.map_err(AppError::from)
    }
    pub(crate) async fn prepare<T>(
        &mut self,
        operation: impl Future<Output = Result<T, paws_model::PawsError>>,
    ) -> Result<T, AppError> {
        let cancelled = async {
            loop {
                if *self.cancel.borrow_and_update() {
                    return;
                }
                if self.cancel.changed().await.is_err() {
                    return;
                }
            }
        };
        let result = tokio::select! {
            biased;
            _ = cancelled => Err(AppError::ImportCancelled),
            prepared = tokio::time::timeout(self.session.timeout, operation) => {
                match prepared {
                    Ok(result) => result.map_err(AppError::from),
                    Err(_) => Err(AppError::ImportTimedOut),
                }
            }
        };
        if result.is_err() {
            self.session.cancel(self.id)?;
        }
        result
    }
    pub(crate) fn begin_commit(&self) -> Result<(), AppError> {
        let mut state = self.session.state.lock().map_err(|_| AppError::Poisoned)?;
        if state.closed || self.id <= state.cancelled_through {
            return Err(AppError::ImportCancelled);
        }
        let active = state
            .current
            .as_mut()
            .filter(|active| active.id == self.id)
            .ok_or(AppError::ImportCancelled)?;
        active.committing = true;
        Ok(())
    }
}
impl Drop for ImportPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.session.state.lock() {
            if state
                .current
                .as_ref()
                .is_some_and(|active| active.id == self.id)
            {
                state.current = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_before_start_never_expires_and_old_cancel_cannot_cancel_a_new_intent() {
        let session = ImportSession::default();
        assert!(session.cancel(3).unwrap());
        assert!(matches!(session.begin(3), Err(AppError::ImportCancelled)));
        let next = session.begin(4).unwrap();
        assert!(!session.cancel(3).unwrap());
        next.begin_commit().unwrap();
        assert!(!session.cancel(4).unwrap());
    }
    #[tokio::test]
    async fn cancellation_interrupts_preparation_and_the_commit_transition_checks_under_the_same_lock(
    ) {
        let session = ImportSession::default();
        let mut permit = session.begin(1).unwrap();
        session.cancel(1).unwrap();
        let result: Result<(), _> = permit.prepare(std::future::pending()).await;
        assert!(matches!(result, Err(AppError::ImportCancelled)));
        assert!(matches!(
            permit.begin_commit(),
            Err(AppError::ImportCancelled)
        ));
    }
    #[tokio::test]
    async fn timeout_does_not_grant_commit_authority_and_close_does_not_interrupt_an_existing_commit(
    ) {
        let session = ImportSession {
            timeout: Duration::from_millis(10),
            ..Default::default()
        };
        let mut permit = session.begin(1).unwrap();
        let result: Result<(), _> = permit.prepare(std::future::pending()).await;
        assert!(matches!(result, Err(AppError::ImportTimedOut)));
        assert!(matches!(
            permit.begin_commit(),
            Err(AppError::ImportCancelled)
        ));
        drop(permit);
        let committing = session.begin(2).unwrap();
        committing.begin_commit().unwrap();
        session.close();
        assert!(!session.cancel(2).unwrap());
        assert!(matches!(session.begin(3), Err(AppError::ImportCancelled)));
    }

    #[tokio::test]
    async fn close_and_cancel_do_not_drop_an_acquired_commit_or_its_followup() {
        let session = ImportSession::default();
        let permit = session.begin(1).unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let entered_commit = entered.clone();
        let release_commit = release.clone();
        let operation = tokio::spawn(async move {
            permit
                .commit(async move {
                    entered_commit.notify_one();
                    release_commit.notified().await;
                    // The acquired transaction and its followup remain in the
                    // same future; component close must not cancel either stage.
                    Ok(("committed profile", "conditional VPN restart"))
                })
                .await
        });
        entered.notified().await;
        session.close();
        assert!(!session.cancel(1).unwrap());
        release.notify_one();
        assert_eq!(
            operation.await.unwrap().unwrap(),
            ("committed profile", "conditional VPN restart")
        );
    }
}
