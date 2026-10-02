use game_account_erasure::erasure::{erase_player, PlayerRecord};
use game_account_erasure::infrai_client::{IdentityControl, InfraiError};
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

#[derive(Default)]
struct RecordingControl {
    calls: RefCell<Vec<String>>,
}

impl IdentityControl for RecordingControl {
    async fn list_sessions(&self, user_id: &str) -> Result<Vec<String>, InfraiError> {
        self.calls.borrow_mut().push(format!("list:{user_id}"));
        Ok(vec!["session-a".into(), "session-b".into()])
    }
    async fn revoke_session(&self, session_id: &str) -> Result<(), InfraiError> {
        self.calls
            .borrow_mut()
            .push(format!("session:{session_id}"));
        Ok(())
    }
    async fn revoke_key(&self, key_id: &str) -> Result<(), InfraiError> {
        self.calls.borrow_mut().push(format!("key:{key_id}"));
        Ok(())
    }
}

#[test]
fn revokes_access_before_erasing_game_data() {
    let control = RecordingControl::default();
    let mut player = PlayerRecord {
        user_id: "player-42".into(),
        credential_id: "key-9".into(),
        assets: vec!["map".into(), "skin".into()],
        live_events: vec!["raid".into()],
        moderation_queue_entries: vec!["report-7".into()],
        deleted: false,
    };
    let receipt = block_on(erase_player(&control, &mut player)).unwrap();
    assert_eq!(
        control.calls.into_inner(),
        vec![
            "list:player-42",
            "session:session-a",
            "session:session-b",
            "key:key-9"
        ]
    );
    assert_eq!(
        (
            receipt.sessions_revoked,
            receipt.assets_removed,
            receipt.live_events_removed,
            receipt.moderation_entries_removed
        ),
        (2, 2, 1, 1)
    );
    assert!(receipt.credential_revoked && player.deleted);
    assert!(
        player.assets.is_empty()
            && player.live_events.is_empty()
            && player.moderation_queue_entries.is_empty()
    );
}

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}
fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        if let Poll::Ready(value) = Pin::new(&mut future).poll(&mut context) {
            return value;
        }
    }
}
