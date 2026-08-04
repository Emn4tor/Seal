pub mod admin;
pub mod auth;
pub mod db;
pub mod error;
pub mod relay;
pub mod routes;
pub mod state;

use axum::Router;
use axum::routing::{get, post, put};
use tower_http::trace::TraceLayer;

pub use state::AppState;

/// The public rendezvous API: users, presence, and group roster/info only —
/// never message content, and no auth beyond each request's own Ed25519
/// signature (the `user_id` a client claims *is* the fingerprint of the key
/// that must sign it).
pub fn build_public_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/users", post(routes::users::register_user))
        .route("/v1/users/{user_id}", get(routes::users::get_user))
        .route("/v1/users/{user_id}/otk", post(routes::users::upload_otk))
        .route(
            "/v1/users/{user_id}/otk/claim",
            get(routes::users::claim_otk),
        )
        .route(
            "/v1/presence/{user_id}",
            put(routes::presence::put_presence).get(routes::presence::get_presence),
        )
        .route("/v1/relay-info", get(routes::relay::get_relay_info))
        .route("/v1/groups", post(routes::groups::create_group))
        .route(
            "/v1/groups/{group_id}",
            get(routes::groups::get_group).put(routes::groups::update_roster),
        )
        .route(
            "/v1/groups/{group_id}/channels",
            post(routes::groups::create_channel),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
