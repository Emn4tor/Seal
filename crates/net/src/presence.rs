use std::time::{Duration, SystemTime, UNIX_EPOCH};

use identity::Identity;
use wire_proto::PresenceUpdateRequest;

use crate::error::NetError;

/// Same reasoning as `DirectoryClient`'s timeout: without one, a stale
/// connection could hang forever, silently stopping every future
/// re-announcement since the heartbeat loop awaits each push sequentially.
const PRESENCE_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs() as i64
}

fn nonce() -> String {
    format!(
        "{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    )
}

/// Signs and pushes a single presence update to the directory server. This
/// is the only thing the directory server ever learns about a peer's
/// network location — a short-TTL rendezvous record, re-asserted on a
/// heartbeat, never message content.
#[allow(clippy::too_many_arguments)]
pub async fn push_presence(
    directory_base_url: &str,
    identity: &Identity,
    device_id: &str,
    peer_id: &str,
    multiaddrs: Vec<String>,
    relay_addrs: Vec<String>,
    share_online_status: bool,
    ttl_secs: u64,
) -> Result<(), NetError> {
    let user_id = identity.user_id();
    let mut req = PresenceUpdateRequest {
        user_id: user_id.clone(),
        device_id: device_id.to_string(),
        peer_id: peer_id.to_string(),
        multiaddrs,
        relay_addrs,
        ttl_secs,
        share_online_status,
        timestamp: now(),
        nonce: nonce(),
        signature: String::new(),
    };
    req.signature = identity.sign(&req.signing_bytes());

    let url = format!(
        "{}/v1/presence/{}/{}",
        directory_base_url.trim_end_matches('/'),
        user_id,
        device_id
    );
    let resp = reqwest::Client::new()
        .put(&url)
        .json(&req)
        .timeout(PRESENCE_REQUEST_TIMEOUT)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(NetError::DirectoryRequestFailed(format!(
            "{status}: {body}"
        )));
    }
    Ok(())
}

/// Runs `push_presence` on a fixed interval until dropped/aborted.
/// `get_multiaddrs`/`get_relay_addrs` are called fresh each tick so
/// addresses can change over time.
#[allow(clippy::too_many_arguments)]
pub async fn run_presence_heartbeat_loop(
    directory_base_url: String,
    identity: std::sync::Arc<Identity>,
    device_id: String,
    peer_id: String,
    get_multiaddrs: impl Fn() -> Vec<String> + Send + 'static,
    get_relay_addrs: impl Fn() -> Vec<String> + Send + 'static,
    get_share_online_status: impl Fn() -> bool + Send + 'static,
    interval: Duration,
) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        let multiaddrs = get_multiaddrs();
        let relay_addrs = get_relay_addrs();
        let share_online_status = get_share_online_status();
        let ttl_secs = (interval.as_secs() * 2).max(30);
        if let Err(e) = push_presence(
            &directory_base_url,
            &identity,
            &device_id,
            &peer_id,
            multiaddrs,
            relay_addrs,
            share_online_status,
            ttl_secs,
        )
        .await
        {
            tracing::warn!(error = %e, "presence heartbeat failed");
        }
    }
}
