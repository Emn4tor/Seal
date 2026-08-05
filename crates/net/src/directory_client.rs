use std::time::{Duration, SystemTime, UNIX_EPOCH};

use identity::Identity;
use reqwest::{Client, StatusCode};
use wire_proto::{
    ChannelKind, ChannelRecord, ClaimedOtk, CreateChannelRequest, CreateGroupRequest, GroupRecord,
    OneTimeKeyEntry, PresenceRecord, PresenceUpdateRequest, RegisterUserRequest, RelayInfoResponse,
    RosterUpdateRequest, UploadOtkRequest, UserRecord,
};

use crate::error::NetError;

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

/// Thin client for the directory server's rendezvous API — user/presence/
/// group lookups and the signed writes that go with them. This is the only
/// thing in the app that talks to a server; everything it returns is public
/// key material, addresses, or group membership, never message content.
#[derive(Clone)]
pub struct DirectoryClient {
    base_url: String,
    client: Client,
}

impl DirectoryClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            // A bounded timeout matters more than it looks: without one, a
            // connection left over from before the OS suspended the process
            // (laptop sleep, app backgrounded a while) can sit there looking
            // alive but never actually deliver a response, and this request
            // would otherwise hang forever rather than fail and let a retry
            // succeed on a fresh connection.
            client: Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("failed to build directory HTTP client"),
        }
    }

    async fn send_json<Req: serde::Serialize, Resp: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Req>,
    ) -> Result<Resp, NetError> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.request(method, &url);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(NetError::DirectoryRequestFailed(format!(
                "{status}: {body}"
            )));
        }
        Ok(resp.json().await?)
    }

    async fn send_no_body_response<Req: serde::Serialize>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &Req,
    ) -> Result<(), NetError> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.client.request(method, &url).json(body).send().await?;
        let status = resp.status();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            let body = resp.text().await.unwrap_or_default();
            return Err(NetError::DirectoryRequestFailed(format!(
                "{status}: {body}"
            )));
        }
        Ok(())
    }

    pub async fn register(
        &self,
        identity: &Identity,
        display_name: &str,
    ) -> Result<UserRecord, NetError> {
        let mut req = RegisterUserRequest {
            user_id: identity.user_id(),
            display_name: display_name.to_string(),
            ed25519_key: identity.ed25519_public_base64(),
            curve25519_key: identity.curve25519_public_base64(),
            timestamp: now(),
            nonce: nonce(),
            signature: String::new(),
        };
        req.signature = identity.sign(&req.signing_bytes());
        self.send_json(reqwest::Method::POST, "/v1/users", Some(&req))
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn put_presence(
        &self,
        identity: &Identity,
        peer_id: &str,
        multiaddrs: Vec<String>,
        relay_addrs: Vec<String>,
        ttl_secs: u64,
    ) -> Result<PresenceRecord, NetError> {
        let mut req = PresenceUpdateRequest {
            user_id: identity.user_id(),
            peer_id: peer_id.to_string(),
            multiaddrs,
            relay_addrs,
            ttl_secs,
            timestamp: now(),
            nonce: nonce(),
            signature: String::new(),
        };
        req.signature = identity.sign(&req.signing_bytes());
        self.send_json(
            reqwest::Method::PUT,
            &format!("/v1/presence/{}", identity.user_id()),
            Some(&req),
        )
        .await
    }

    pub async fn get_presence(&self, user_id: &str) -> Result<PresenceRecord, NetError> {
        self.send_json::<(), _>(
            reqwest::Method::GET,
            &format!("/v1/presence/{user_id}"),
            None,
        )
        .await
    }

    /// The directory server's own libp2p relay identity, if it's running
    /// one and its operator has advertised an externally-reachable address
    /// for it (see `directory-server`'s `DIRECTORY_RELAY_EXTERNAL_MULTIADDR`).
    /// Errors (including a 404 for "no relay configured") are all treated
    /// the same by callers: best-effort NAT-traversal fallback, not
    /// required for the app to function on a LAN or when a direct dial
    /// already works.
    pub async fn get_relay_info(&self) -> Result<RelayInfoResponse, NetError> {
        self.send_json::<(), _>(reqwest::Method::GET, "/v1/relay-info", None)
            .await
    }

    pub async fn get_user(&self, user_id: &str) -> Result<UserRecord, NetError> {
        self.send_json::<(), _>(reqwest::Method::GET, &format!("/v1/users/{user_id}"), None)
            .await
    }

    pub async fn upload_one_time_keys(
        &self,
        identity: &Identity,
        keys: Vec<OneTimeKeyEntry>,
        fallback_key: Option<OneTimeKeyEntry>,
    ) -> Result<(), NetError> {
        let mut req = UploadOtkRequest {
            user_id: identity.user_id(),
            keys,
            fallback_key,
            timestamp: now(),
            nonce: nonce(),
            signature: String::new(),
        };
        req.signature = identity.sign(&req.signing_bytes());
        let path = format!("/v1/users/{}/otk", identity.user_id());
        self.send_no_body_response(reqwest::Method::POST, &path, &req)
            .await
    }

    pub async fn claim_one_time_key(&self, user_id: &str) -> Result<ClaimedOtk, NetError> {
        self.send_json::<(), _>(
            reqwest::Method::GET,
            &format!("/v1/users/{user_id}/otk/claim"),
            None,
        )
        .await
    }

    pub async fn create_group(
        &self,
        identity: &Identity,
        group_id: &str,
        name: &str,
    ) -> Result<GroupRecord, NetError> {
        let mut req = CreateGroupRequest {
            group_id: group_id.to_string(),
            name: name.to_string(),
            creator_id: identity.user_id(),
            timestamp: now(),
            nonce: nonce(),
            signature: String::new(),
        };
        req.signature = identity.sign(&req.signing_bytes());
        self.send_json(reqwest::Method::POST, "/v1/groups", Some(&req))
            .await
    }

    /// Group_ids this user belongs to, per the server's roster records —
    /// used to notice group memberships that never arrived locally (the
    /// P2P key-share that normally delivers them was lost) so they can be
    /// recovered instead of staying permanently invisible. See
    /// `AppService::discover_missing_groups`.
    pub async fn list_my_groups(&self, user_id: &str) -> Result<Vec<String>, NetError> {
        self.send_json::<(), _>(
            reqwest::Method::GET,
            &format!("/v1/users/{user_id}/groups"),
            None,
        )
        .await
    }

    pub async fn get_group(&self, group_id: &str) -> Result<GroupRecord, NetError> {
        self.send_json::<(), _>(
            reqwest::Method::GET,
            &format!("/v1/groups/{group_id}"),
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_roster(
        &self,
        identity: &Identity,
        group_id: &str,
        add: Vec<String>,
        remove: Vec<String>,
        expected_version: u64,
    ) -> Result<GroupRecord, NetError> {
        let mut req = RosterUpdateRequest {
            group_id: group_id.to_string(),
            actor_id: identity.user_id(),
            add,
            remove,
            expected_version,
            timestamp: now(),
            nonce: nonce(),
            signature: String::new(),
        };
        req.signature = identity.sign(&req.signing_bytes());
        self.send_json(
            reqwest::Method::PUT,
            &format!("/v1/groups/{group_id}"),
            Some(&req),
        )
        .await
    }

    pub async fn create_channel(
        &self,
        identity: &Identity,
        group_id: &str,
        channel_id: &str,
        name: &str,
        kind: ChannelKind,
    ) -> Result<ChannelRecord, NetError> {
        let mut req = CreateChannelRequest {
            group_id: group_id.to_string(),
            channel_id: channel_id.to_string(),
            name: name.to_string(),
            kind,
            actor_id: identity.user_id(),
            timestamp: now(),
            nonce: nonce(),
            signature: String::new(),
        };
        req.signature = identity.sign(&req.signing_bytes());
        self.send_json(
            reqwest::Method::POST,
            &format!("/v1/groups/{group_id}/channels"),
            Some(&req),
        )
        .await
    }
}
