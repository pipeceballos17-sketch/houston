//! `/v1/capcom` — agent-to-agent negotiation (CAPCOM) with human gates.
//!
//! - `POST /v1/capcom/propose`   emit a Proposal to a peer via the relay
//! - `POST /v1/capcom/approve`   record the human verdict (Ready / Reject)
//! - `GET  /v1/capcom/peers`     list known peer agent cards
//!
//! Block 1 is the route skeleton: the surface compiles and is registered, and
//! every handler parses its typed body. The relay transport that actually
//! carries `CapcomFrame`s between peers lands in a later block — until then the
//! mutating routes report `Unavailable` rather than silently succeeding.

use crate::routes::error::ApiError;
use crate::state::ServerState;
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use houston_engine_core::CoreError;
use houston_engine_protocol::capcom::{AgentCard, ApprovalDecision, CapcomProposal};
use std::sync::Arc;

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/capcom/propose", post(propose)) // emit Proposal to peer via relay
        .route("/capcom/approve", post(approve)) // human verdict → Ready/Reject
        .route("/capcom/peers", get(peers)) // list known peer cards
}

/// Emit a CAPCOM proposal to a peer over the relay.
async fn propose(
    State(_st): State<Arc<ServerState>>,
    Json(_proposal): Json<CapcomProposal>,
) -> Result<Json<()>, ApiError> {
    Err(ApiError(CoreError::Unavailable(
        "CAPCOM peer transport isn't wired yet — sending a proposal to a peer lands in a later block.".into(),
    )))
}

/// Record the human's verdict on a pending proposal (Ready / Reject).
async fn approve(
    State(_st): State<Arc<ServerState>>,
    Json(_decision): Json<ApprovalDecision>,
) -> Result<Json<()>, ApiError> {
    Err(ApiError(CoreError::Unavailable(
        "CAPCOM peer transport isn't wired yet — relaying a verdict to the peer lands in a later block.".into(),
    )))
}

/// List the peer agent cards this engine currently knows about.
async fn peers(State(_st): State<Arc<ServerState>>) -> Json<Vec<AgentCard>> {
    // No peer registry yet; the relay-backed roster lands in a later block.
    // An empty list is the truthful current state, not a swallowed error.
    Json(Vec::new())
}
