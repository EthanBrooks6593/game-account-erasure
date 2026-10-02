use crate::infrai_client::{IdentityControl, InfraiError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerRecord {
    pub user_id: String,
    pub credential_id: String,
    pub assets: Vec<String>,
    pub live_events: Vec<String>,
    pub moderation_queue_entries: Vec<String>,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErasureReceipt {
    pub user_id: String,
    pub sessions_revoked: usize,
    pub assets_removed: usize,
    pub live_events_removed: usize,
    pub moderation_entries_removed: usize,
    pub credential_revoked: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ErasureError {
    AlreadyDeleted,
    Infrai(InfraiError),
}

impl From<InfraiError> for ErasureError {
    fn from(value: InfraiError) -> Self {
        Self::Infrai(value)
    }
}

pub async fn erase_player<C: IdentityControl>(
    control: &C,
    player: &mut PlayerRecord,
) -> Result<ErasureReceipt, ErasureError> {
    if player.deleted {
        return Err(ErasureError::AlreadyDeleted);
    }

    let sessions = control.list_sessions(&player.user_id).await?;
    for session_id in &sessions {
        control.revoke_session(session_id).await?;
    }
    control.revoke_key(&player.credential_id).await?;

    let receipt = ErasureReceipt {
        user_id: player.user_id.clone(),
        sessions_revoked: sessions.len(),
        assets_removed: player.assets.len(),
        live_events_removed: player.live_events.len(),
        moderation_entries_removed: player.moderation_queue_entries.len(),
        credential_revoked: true,
    };
    player.assets.clear();
    player.live_events.clear();
    player.moderation_queue_entries.clear();
    player.deleted = true;
    Ok(receipt)
}
