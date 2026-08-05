//! Presence: online status and last-seen timestamps per peer.
//!
//! Two entry points query the relay for a peer's presence:
//! [`get_presence`](RelayClient::get_presence) (a one-shot request) and
//! [`watch_presence`](RelayClient::watch_presence) (a subscription that pushes
//! on every connect/disconnect). Both feed [`handle_presence`](RelayClient::handle_presence),
//! which caches the snapshot in the presence map, persists an offline
//! `last_seen` on the contact row, and emits the `presence` UI event.
//!
//! This module owns the presence `impl RelayClient` block; the shared
//! [`PresenceInfo`] / [`PresenceEvent`] types stay in the relay core (`super`).

use super::*;

impl RelayClient {
    /// Fetch a peer's current presence (online status + last-seen), waiting up
    /// to [`PRESENCE_FETCH_TIMEOUT`]. The reply is also cached in the presence
    /// map and emitted as a `presence` event by the inbound loop, so a command
    /// caller and every event listener end up with the same snapshot.
    pub async fn get_presence(&self, peer_id: &str) -> Result<PresenceInfo, RelayError> {
        let (tx, rx) = oneshot::channel();
        mutex_guard(&self.inner.pending_presence)?.push_back((peer_id.to_string(), tx));
        if let Err(err) = self.send_json(&ClientMessage::GetPresence {
            peer_id: peer_id.to_string(),
        }) {
            // The request never left, so drop the dangling waiter(s) for this
            // peer to keep the queue aligned with the relay's replies.
            mutex_guard(&self.inner.pending_presence)?.retain(|(peer, _)| peer != peer_id);
            return Err(err);
        }

        match tokio::time::timeout(PRESENCE_FETCH_TIMEOUT, rx).await {
            Ok(Ok(Ok(info))) => Ok(info),
            Ok(Ok(Err(err))) => Err(err),
            Ok(Err(_)) => Err(RelayError::PresenceFetchFailed),
            Err(_) => {
                // The waiter timed out: sweep closed senders so a late reply
                // for this peer cannot keep resolving dead requests (each
                // dropped receiver closes its sender, so this only ever
                // removes stale entries).
                if let Ok(mut pending) = self.inner.pending_presence.lock() {
                    pending.retain(|(_, tx)| !tx.is_closed());
                }
                Err(RelayError::PresenceTimeout)
            }
        }
    }

    /// Subscribe to presence pushes for `peer_id`: the relay sends a
    /// `presence` message whenever the peer comes online or goes offline.
    /// Best-effort — without a connection the subscription is dropped and the
    /// caller is expected to re-watch after (re)connecting.
    pub fn watch_presence(&self, peer_id: &str) -> Result<(), RelayError> {
        self.send_json(&ClientMessage::WatchPresence {
            peer_id: peer_id.to_string(),
        })
    }

    /// Record a peer's presence and notify the UI via a `presence` event.
    ///
    /// Called for both `watch_presence` pushes and `get_presence` replies, so
    /// the cache and the event stream always reflect the same snapshot. An
    /// offline report's `last_seen` is persisted on the contact row so the
    /// timestamp survives restarts.
    pub(crate) fn handle_presence(
        &self,
        peer_id: &str,
        online: bool,
        last_seen: Option<i64>,
    ) -> Result<(), RelayError> {
        write_guard(&self.inner.presence)?
            .insert(peer_id.to_string(), PresenceInfo { online, last_seen });
        if !online {
            if let Some(ts) = last_seen {
                self.ensure_store_open()?;
                let store_guard = self.store_guard()?;
                let store = store_guard.as_ref().ok_or(RelayError::StoreNotOpen)?;
                // Only touch contacts we already know; presence alone must not
                // surface a stranger in the contact list.
                if let Some(mut contact) = store.get_contact(peer_id)? {
                    contact.last_seen = Some(ts);
                    store.upsert_contact(&contact)?;
                }
            }
        }
        let _ = self.inner.app.emit(
            "presence",
            PresenceEvent {
                peer_id: peer_id.to_string(),
                online,
                last_seen,
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_presence_message_parses() {
        let text = r#"{"type":"presence","peer_id":"bob","online":false,"last_seen":1700000000}"#;
        match serde_json::from_str::<ServerMessage>(text).expect("parse") {
            ServerMessage::Presence {
                peer_id,
                online,
                last_seen,
            } => {
                assert_eq!(peer_id, "bob");
                assert!(!online);
                assert_eq!(last_seen, Some(1_700_000_000));
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn server_presence_message_parses_online_with_null_last_seen() {
        let text = r#"{"type":"presence","peer_id":"bob","online":true,"last_seen":null}"#;
        match serde_json::from_str::<ServerMessage>(text).expect("parse") {
            ServerMessage::Presence {
                online, last_seen, ..
            } => {
                assert!(online);
                assert_eq!(last_seen, None);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn presence_client_messages_serialize() {
        let get = serde_json::to_value(ClientMessage::GetPresence {
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(get["type"], "get_presence");
        assert_eq!(get["peer_id"], "bob");

        let watch = serde_json::to_value(ClientMessage::WatchPresence {
            peer_id: "bob".into(),
        })
        .expect("serialize");
        assert_eq!(watch["type"], "watch_presence");
        assert_eq!(watch["peer_id"], "bob");
    }

    #[test]
    fn presence_info_roundtrips_through_json() {
        let online = PresenceInfo {
            online: true,
            last_seen: None,
        };
        let restored: PresenceInfo =
            serde_json::from_str(&serde_json::to_string(&online).expect("serialize"))
                .expect("deserialize");
        assert!(restored.online);
        assert_eq!(restored.last_seen, None);

        let offline = PresenceInfo {
            online: false,
            last_seen: Some(1_700_000_000),
        };
        let json = serde_json::to_value(&offline).expect("serialize");
        assert_eq!(json["online"], false);
        assert_eq!(json["last_seen"], 1_700_000_000);
    }
}
