//! Presence: online status, last-seen and visibility privacy.
//!
//! `watch_presence` subscribes a socket to pushes for one peer, `get_presence`
//! answers a one-shot status query and `broadcast_presence` pushes online/
//! offline changes to every watcher. `set_privacy` lets a peer hide its online
//! status and last-seen from everyone else (reports become `online: false`
//! with a `null` last-seen). Presence traffic is rate limited per source IP
//! under the `presence:<ip>` bucket.

use super::*;

impl Relay {
    /// Register `watcher`'s socket as a presence subscriber of `watched`.
    ///
    /// Re-watching the same peer replaces the watcher's previous registration,
    /// so a peer can never hold two live channels in one watched list (and
    /// reconnecting cannot duplicate pushes). Watching a peer you are already
    /// watching is a no-op apart from the replacement.
    ///
    /// Presence traffic (both this and `get_presence`) is rate limited per
    /// source IP under the `presence:<ip>` bucket.
    pub(crate) async fn watch_presence(
        &self,
        watcher: &str,
        ip: &str,
        watched: &str,
        tx: Outbound,
    ) {
        if !self.inner.limiter.try_take(&format!("presence:{ip}")) {
            tracing::warn!(ip = %ip, "presence rate limit exceeded");
            let _ = self
                .send(
                    watcher,
                    ServerMessage::Error {
                        code: "rate_limited".into(),
                    },
                )
                .await;
            return;
        }

        let mut watchers = self.inner.presence_watchers.write().await;
        let list = watchers.entry(watched.to_string()).or_default();
        list.retain(|w| w.peer_id != watcher);
        list.push(PresenceWatcher {
            peer_id: watcher.to_string(),
            tx,
        });
    }

    /// Persist the caller's presence-visibility preference and confirm with a
    /// `privacy_updated` reply.
    ///
    /// When a peer hides its presence, every `get_presence` reply and every
    /// `broadcast_presence` push for that peer reports `online: false` with
    /// `last_seen: null`, so other peers cannot tell when the peer is online
    /// or when it was last seen. The preference is rate limited under the
    /// `presence:<ip>` bucket like the other presence operations.
    pub(crate) async fn set_privacy(&self, peer_id: &str, ip: &str, presence_visible: bool) {
        if !self.inner.limiter.try_take(&format!("presence:{ip}")) {
            tracing::warn!(ip = %ip, "presence rate limit exceeded");
            let _ = self
                .send(
                    peer_id,
                    ServerMessage::Error {
                        code: "rate_limited".into(),
                    },
                )
                .await;
            return;
        }

        match self
            .inner
            .store
            .set_presence_visible(peer_id, presence_visible)
        {
            Ok(()) => {
                let _ = self.send(peer_id, ServerMessage::PrivacyUpdated).await;
            }
            Err(err) => {
                tracing::error!(peer = %peer_id, "failed to persist privacy setting: {err}");
            }
        }
    }

    /// Answer a one-shot presence query for `target`: whether the peer is
    /// online right now, plus its stored last-seen timestamp when offline.
    /// Unknown peers report `online: false` with `last_seen: null`. A peer
    /// that hides its presence is always reported as offline with no
    /// last-seen, even while it is connected.
    pub(crate) async fn get_presence(&self, requester: &str, ip: &str, target: &str) {
        if !self.inner.limiter.try_take(&format!("presence:{ip}")) {
            tracing::warn!(ip = %ip, "presence rate limit exceeded");
            let _ = self
                .send(
                    requester,
                    ServerMessage::Error {
                        code: "rate_limited".into(),
                    },
                )
                .await;
            return;
        }

        let visible = self.inner.store.get_presence_visible(target);
        let (online, last_seen) = if !visible {
            (false, None)
        } else {
            let online = self.inner.online.read().await.contains_key(target);
            let last_seen = if online {
                None
            } else {
                self.inner.store.get_last_seen(target)
            };
            (online, last_seen)
        };
        let _ = self
            .send(
                requester,
                ServerMessage::Presence {
                    peer_id: target.to_string(),
                    online,
                    last_seen,
                },
            )
            .await;
    }

    /// Push a presence change for `peer_id` to every registered watcher.
    ///
    /// Watchers whose channel is gone (closed socket, or the peer itself
    /// disconnected) are dropped in the same pass, so dead subscriptions
    /// cannot accumulate. The `presence_watchers` lock is held while sending;
    /// sends into unbounded channels never block, so this is safe.
    ///
    /// A peer that hides its presence is pushed as `online: false` with
    /// `last_seen: null` even while it is connected.
    pub(crate) async fn broadcast_presence(&self, peer_id: &str, online: bool) {
        let visible = self.inner.store.get_presence_visible(peer_id);
        let (online, last_seen) = if !visible {
            (false, None)
        } else if online {
            (true, None)
        } else {
            (false, self.inner.store.get_last_seen(peer_id))
        };
        let text = serde_json::to_string(&ServerMessage::Presence {
            peer_id: peer_id.to_string(),
            online,
            last_seen,
        })
        .ok();

        let mut watchers = self.inner.presence_watchers.write().await;
        if let Some(list) = watchers.get_mut(peer_id) {
            match text {
                Some(text) => {
                    list.retain(|w| w.tx.send(WsMessage::Text(text.clone().into())).is_ok())
                }
                None => list.clear(),
            }
            if list.is_empty() {
                watchers.remove(peer_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::test_utils::read_reply;

    #[tokio::test]
    async fn watcher_receives_online_and_offline_presence_pushes() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let watched = "bob".to_string();
        let (watch_tx, mut watch_rx) = mpsc::unbounded_channel::<WsMessage>();

        relay
            .watch_presence("alice", "127.0.0.1", &watched, watch_tx)
            .await;

        // Bob comes online: the watcher must get an `online: true` push.
        let (bob_tx, _bob_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(watched.clone(), bob_tx);
        relay.broadcast_presence(&watched, true).await;
        let reply = read_reply(&mut watch_rx);
        assert_eq!(reply["type"].as_str(), Some("presence"));
        assert_eq!(reply["peer_id"].as_str(), Some("bob"));
        assert_eq!(reply["online"].as_bool(), Some(true));
        assert!(
            reply["last_seen"].is_null(),
            "online pushes carry no last_seen"
        );

        // Bob goes offline: last_seen must be included in the push.
        relay.inner.online.write().await.remove(&watched);
        relay
            .inner
            .store
            .set_last_seen(&watched, 1_700_000_000)
            .unwrap();
        relay.broadcast_presence(&watched, false).await;
        let reply = read_reply(&mut watch_rx);
        assert_eq!(reply["type"].as_str(), Some("presence"));
        assert_eq!(reply["online"].as_bool(), Some(false));
        assert_eq!(reply["last_seen"].as_i64(), Some(1_700_000_000));
    }

    #[tokio::test]
    async fn watch_presence_replaces_previous_channel_for_same_peer() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let watched = "bob".to_string();
        let (tx1, mut rx1) = mpsc::unbounded_channel::<WsMessage>();
        let (tx2, mut rx2) = mpsc::unbounded_channel::<WsMessage>();

        // Alice watches bob, then re-watches bob on a fresh socket: the old
        // registration must be replaced, not appended.
        relay
            .watch_presence("alice", "127.0.0.1", &watched, tx1)
            .await;
        relay
            .watch_presence("alice", "127.0.0.1", &watched, tx2)
            .await;

        relay.broadcast_presence(&watched, true).await;
        let reply = read_reply(&mut rx2);
        assert_eq!(reply["type"].as_str(), Some("presence"));
        assert_eq!(reply["online"].as_bool(), Some(true));
        assert!(
            rx1.try_recv().is_err(),
            "the replaced channel must not receive pushes"
        );
    }

    #[tokio::test]
    async fn get_presence_reports_online_status_and_last_seen() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert("requester".into(), out_tx);

        // Unknown peer: offline, no last_seen.
        relay.get_presence("requester", "127.0.0.1", "ghost").await;
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["type"].as_str(), Some("presence"));
        assert_eq!(reply["peer_id"].as_str(), Some("ghost"));
        assert_eq!(reply["online"].as_bool(), Some(false));
        assert!(reply["last_seen"].is_null());

        // Offline peer with a stored last_seen.
        relay
            .inner
            .store
            .set_last_seen("bob", 1_700_000_000)
            .unwrap();
        relay.get_presence("requester", "127.0.0.1", "bob").await;
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["online"].as_bool(), Some(false));
        assert_eq!(reply["last_seen"].as_i64(), Some(1_700_000_000));

        // Online peer reports online:true regardless of the stored value.
        let (bob_tx, _bob_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert("bob".into(), bob_tx);
        relay.get_presence("requester", "127.0.0.1", "bob").await;
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["online"].as_bool(), Some(true));
        assert!(reply["last_seen"].is_null());
    }

    #[tokio::test]
    async fn presence_is_rate_limited_per_ip() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 1.0, 0.0);
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert("requester".into(), out_tx);

        relay.get_presence("requester", "10.0.0.1", "bob").await;
        let first = read_reply(&mut out_rx);
        assert_eq!(first["type"].as_str(), Some("presence"));

        relay.get_presence("requester", "10.0.0.1", "bob").await;
        let second = read_reply(&mut out_rx);
        assert_eq!(second["type"].as_str(), Some("error"));
        assert_eq!(second["code"].as_str(), Some("rate_limited"));

        // A different IP has its own bucket and is not blocked.
        relay.get_presence("requester", "10.0.0.2", "bob").await;
        let third = read_reply(&mut out_rx);
        assert_eq!(third["type"].as_str(), Some("presence"));
    }

    #[tokio::test]
    async fn disconnect_records_last_seen_and_pushes_offline() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let watched = "bob".to_string();
        let (watch_tx, mut watch_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .watch_presence("alice", "127.0.0.1", &watched, watch_tx)
            .await;

        // Simulate bob's online -> disconnect sequence as handle_socket does:
        // unregister, persist last_seen, broadcast offline.
        let (bob_tx, _bob_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(watched.clone(), bob_tx);
        relay.inner.online.write().await.remove(&watched);
        let _ = relay.inner.store.set_last_seen(&watched, unix_now());
        relay.broadcast_presence(&watched, false).await;

        let reply = read_reply(&mut watch_rx);
        assert_eq!(reply["online"].as_bool(), Some(false));
        let last_seen = reply["last_seen"].as_i64().expect("last_seen must be set");
        assert!(
            last_seen <= unix_now() && last_seen > unix_now() - 60,
            "last_seen must be near now"
        );
    }

    #[tokio::test]
    async fn set_privacy_persists_preference_and_replies() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let peer_id = "peer-privacy".to_string();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(peer_id.clone(), out_tx);

        relay.set_privacy(&peer_id, "127.0.0.1", false).await;
        assert!(!relay.inner.store.get_presence_visible(&peer_id));
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["type"].as_str(), Some("privacy_updated"));
    }

    #[tokio::test]
    async fn set_privacy_is_rate_limited_per_ip() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_limiter(store, 1.0, 0.0);
        let peer_id = "peer-privacy".to_string();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert(peer_id.clone(), out_tx);

        relay.set_privacy(&peer_id, "10.0.0.1", false).await;
        assert_eq!(
            read_reply(&mut out_rx)["type"].as_str(),
            Some("privacy_updated")
        );

        relay.set_privacy(&peer_id, "10.0.0.1", true).await;
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["type"].as_str(), Some("error"));
        assert_eq!(reply["code"].as_str(), Some("rate_limited"));
        // The rejected flip must not overwrite the accepted one.
        assert!(!relay.inner.store.get_presence_visible(&peer_id));
    }

    #[tokio::test]
    async fn get_presence_hides_status_for_peer_that_hides_presence() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert("requester".into(), out_tx);

        // Bob is ONLINE but hides his presence: the report must say offline.
        relay
            .inner
            .store
            .set_presence_visible("bob", false)
            .unwrap();
        let (bob_tx, _bob_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .inner
            .online
            .write()
            .await
            .insert("bob".into(), bob_tx);
        relay
            .inner
            .store
            .set_last_seen("bob", 1_700_000_000)
            .unwrap();

        relay.get_presence("requester", "127.0.0.1", "bob").await;
        let reply = read_reply(&mut out_rx);
        assert_eq!(reply["type"].as_str(), Some("presence"));
        assert_eq!(reply["online"].as_bool(), Some(false));
        assert!(
            reply["last_seen"].is_null(),
            "a hidden peer must never leak its last_seen"
        );
    }

    #[tokio::test]
    async fn broadcast_presence_hides_status_for_peer_that_hides_presence() {
        let store = Store::open_in_memory().unwrap();
        let relay = Relay::with_store(store);
        let watched = "bob".to_string();
        let (watch_tx, mut watch_rx) = mpsc::unbounded_channel::<WsMessage>();
        relay
            .watch_presence("alice", "127.0.0.1", &watched, watch_tx)
            .await;

        relay
            .inner
            .store
            .set_presence_visible(&watched, false)
            .unwrap();
        relay
            .inner
            .store
            .set_last_seen(&watched, 1_700_000_000)
            .unwrap();

        // Online push: hidden peer is reported offline with no last_seen.
        relay.broadcast_presence(&watched, true).await;
        let reply = read_reply(&mut watch_rx);
        assert_eq!(reply["online"].as_bool(), Some(false));
        assert!(reply["last_seen"].is_null());

        // Offline push: last_seen stays hidden too.
        relay.broadcast_presence(&watched, false).await;
        let reply = read_reply(&mut watch_rx);
        assert_eq!(reply["online"].as_bool(), Some(false));
        assert!(reply["last_seen"].is_null());
    }
}
