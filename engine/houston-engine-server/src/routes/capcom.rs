//! `/v1/capcom` — agent-to-agent negotiation (Camino B: direct HTTP).
//!
//! Two engines (A = outbound, B = inbound) talk directly over their public
//! Railway URLs. Every cross-user action raises a human-approval gate on the
//! local engine before the frame is forwarded. The relay (Camino A) can be
//! layered on later without changing these handlers — only the transport.
//!
//! Routes:
//!   POST /v1/capcom/peers          register/list known peer engines
//!   POST /v1/capcom/propose        A → B: send a proposal (raises B's gate)
//!   POST /v1/capcom/inbound        peer → me: receive a frame (internal)
//!   POST /v1/capcom/approve        human verdict on a pending proposal
//!   POST /v1/capcom/pending        list proposals awaiting my human's gate

use crate::routes::error::ApiError;
use crate::state::ServerState;
use axum::{extract::State, routing::post, Json, Router};
use houston_engine_protocol::capcom::{
    AgentCard, ApprovalDecision, ApprovalDirection, ApprovalRequest, CapcomFrame, CapcomProposal,
    ProposalIntent,
};
use houston_ui_events::{EventSink, HoustonEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ── In-memory negotiation state ────────────────────────────────────
//
// For the hackathon demo this lives in a Mutex<HashMap>. Production would
// persist to houston-db, but pending proposals are ephemeral by nature.

#[derive(Default)]
pub struct CapcomState {
    /// proposalId → the proposal awaiting this engine's human gate.
    pub pending: Mutex<HashMap<String, PendingProposal>>,
    /// Known peers we can talk to (peerId → base URL + bearer token).
    pub peers: Mutex<HashMap<String, PeerEndpoint>>,
    /// This engine's own card, advertised in HELLO/ACK.
    pub self_card: Mutex<Option<AgentCard>>,
    /// If `Some`, route frames through the Cloudflare relay instead of
    /// direct-to-peer (Camino A). Populated from env at boot; `None` keeps
    /// the engine in direct mode (Camino B). `Mutex<Option<_>>` defaults to
    /// `None`, so `#[derive(Default)]` still holds.
    pub relay: Mutex<Option<RelayConfig>>,
}

/// Cloudflare relay coordinates. Present only when the engine is in relay
/// mode (the `CAPCOM_RELAY_*` env vars are set — see `state::with_db`).
#[derive(Clone)]
pub struct RelayConfig {
    pub url: String,
    pub token: String,
    pub room: String,
    /// The agent id THIS engine sends as / polls its mailbox under.
    pub self_id: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingProposal {
    pub proposal: CapcomProposal,
    pub peer: AgentCard,
    pub direction: ApprovalDirection,
    /// Where to forward the READY/REJECT once the human decides.
    pub reply_to_peer_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerEndpoint {
    pub peer_id: String,
    /// El id con el que ESTE engine se anuncia ante este peer.
    pub self_id: String,
    pub base_url: String,
    /// Bearer token for the peer engine. Demo-only; production rotates these.
    pub token: String,
    pub card: AgentCard,
}

// ── Request/response DTOs ──────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterPeerRequest {
    pub peer: PeerEndpoint,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposeRequest {
    pub to_peer_id: String,
    pub proposal: CapcomProposal,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboundRequest {
    pub from_peer_id: String,
    pub frame: CapcomFrame,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ack {
    pub ok: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeRequest {
    pub instruction: String,
    /// Who we're addressing (e.g. "agent-b"). Carried for the caller's
    /// follow-up `propose` call; composition itself doesn't use it.
    pub to_peer_id: String,
    pub from_agent: String,
    pub to_agent: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeResponse {
    pub proposal: CapcomProposal,
    /// "claude" | "fallback" — honest provenance for the demo UI.
    pub source: String,
}

pub fn router() -> Router<Arc<ServerState>> {
    Router::new()
        .route("/capcom/peers", post(register_peer))
        .route("/capcom/propose", post(propose))
        .route("/capcom/inbound", post(inbound))
        .route("/capcom/approve", post(approve))
        .route("/capcom/pending", post(pending))
        .route("/capcom/compose", post(compose))
}

// ── Handlers ───────────────────────────────────────────────────────

/// Register a peer engine we can negotiate with.
async fn register_peer(
    State(st): State<Arc<ServerState>>,
    Json(req): Json<RegisterPeerRequest>,
) -> Result<Json<Ack>, ApiError> {
    let mut peers = st.capcom.peers.lock().unwrap();
    peers.insert(req.peer.peer_id.clone(), req.peer);
    Ok(Json(Ack { ok: true }))
}

/// A → B. Outbound side. Raises THIS engine's outbound gate first; only on
/// approval does the proposal actually leave (see `approve`). Here we just
/// stage it and emit the gate event.
async fn propose(
    State(st): State<Arc<ServerState>>,
    Json(req): Json<ProposeRequest>,
) -> Result<Json<Ack>, ApiError> {
    let peer = {
        let peers = st.capcom.peers.lock().unwrap();
        peers
            .get(&req.to_peer_id)
            .cloned()
            .ok_or_else(|| ApiError::bad_request("unknown peer"))?
    };

    // Stage as pending-outbound and raise the local human gate.
    let pending = PendingProposal {
        proposal: req.proposal.clone(),
        peer: peer.card.clone(),
        direction: ApprovalDirection::Outbound,
        reply_to_peer_id: req.to_peer_id.clone(),
    };
    st.capcom
        .pending
        .lock()
        .unwrap()
        .insert(req.proposal.proposal_id.clone(), pending);

    st.events.emit(HoustonEvent::ApprovalRequest(ApprovalRequest {
        proposal: req.proposal,
        peer: peer.card,
        direction: ApprovalDirection::Outbound,
    }));

    Ok(Json(Ack { ok: true }))
}

/// peer → me. Acts on a received frame: a PROPOSAL/COUNTER raises the inbound
/// gate; READY/REJECT resolve a pending negotiation; HELLO/ACK are discovery
/// no-ops.
///
/// Extracted from the `inbound` route so the relay poller
/// (`main::spawn_capcom_poller`) can drive the exact same logic for frames
/// pulled from the mailbox — both transports converge here.
pub async fn handle_inbound(st: &Arc<ServerState>, from_peer_id: &str, frame: CapcomFrame) {
    match frame {
        CapcomFrame::Proposal { proposal } | CapcomFrame::Counter { proposal } => {
            let peer_card = {
                let peers = st.capcom.peers.lock().unwrap();
                peers
                    .get(from_peer_id)
                    .map(|p| p.card.clone())
                    .unwrap_or_else(|| unknown_card(from_peer_id))
            };
            let pending = PendingProposal {
                proposal: proposal.clone(),
                peer: peer_card.clone(),
                direction: ApprovalDirection::Inbound,
                reply_to_peer_id: from_peer_id.to_string(),
            };
            st.capcom
                .pending
                .lock()
                .unwrap()
                .insert(proposal.proposal_id.clone(), pending);

            st.events.emit(HoustonEvent::ApprovalRequest(ApprovalRequest {
                proposal,
                peer: peer_card,
                direction: ApprovalDirection::Inbound,
            }));
        }
        CapcomFrame::Ready { proposal_id } => {
            st.capcom.pending.lock().unwrap().remove(&proposal_id);
            st.events.emit(HoustonEvent::Toast {
                message: format!("Deal {proposal_id} accepted by peer"),
                variant: "success".into(),
            });
        }
        CapcomFrame::Reject { proposal_id, reason } => {
            st.capcom.pending.lock().unwrap().remove(&proposal_id);
            st.events.emit(HoustonEvent::Toast {
                message: format!("Deal {proposal_id} rejected: {reason}"),
                variant: "error".into(),
            });
        }
        CapcomFrame::Hello { .. } | CapcomFrame::Ack { .. } => {
            // Discovery frames — no gate. Could store the card here.
        }
    }
}

/// peer → me, over direct HTTP (Camino B). Thin route wrapper around
/// [`handle_inbound`]; the relay poller calls the same fn for Camino A.
async fn inbound(
    State(st): State<Arc<ServerState>>,
    Json(req): Json<InboundRequest>,
) -> Result<Json<Ack>, ApiError> {
    handle_inbound(&st, &req.from_peer_id, req.frame).await;
    Ok(Json(Ack { ok: true }))
}

/// The human's verdict on a pending proposal. On approval we forward the
/// frame to the peer; on rejection we send a REJECT.
async fn approve(
    State(st): State<Arc<ServerState>>,
    Json(decision): Json<ApprovalDecision>,
) -> Result<Json<Ack>, ApiError> {
    let pending = st
        .capcom
        .pending
        .lock()
        .unwrap()
        .remove(&decision.proposal_id)
        .ok_or_else(|| ApiError::not_found("no such pending proposal"))?;

    let frame = if decision.approved {
        match pending.direction {
            // Outbound approved: the proposal now actually leaves for the peer.
            ApprovalDirection::Outbound => CapcomFrame::Proposal {
                proposal: pending.proposal.clone(),
            },
            // Inbound approved: tell the peer the deal is done.
            ApprovalDirection::Inbound => CapcomFrame::Ready {
                proposal_id: decision.proposal_id.clone(),
            },
        }
    } else {
        CapcomFrame::Reject {
            proposal_id: decision.proposal_id.clone(),
            reason: decision.reason.unwrap_or_else(|| "declined by human".into()),
        }
    };

    // Relay mode (Camino A): address the peer by agent id and drop the frame in
    // the relay mailbox — no PeerEndpoint URL/token needed, so we skip the peer
    // lookup entirely (otherwise "peer gone" would fire spuriously). Direct mode
    // (Camino B): resolve the peer and POST straight to it.
    let relay_opt = st.capcom.relay.lock().unwrap().clone();
    if let Some(relay) = relay_opt {
        forward_via_relay(&relay, &pending.reply_to_peer_id, &frame)
            .await
            .map_err(|e| ApiError::internal(format!("relay forward failed: {e}")))?;
    } else {
        let peer = {
            let peers = st.capcom.peers.lock().unwrap();
            peers
                .get(&pending.reply_to_peer_id)
                .cloned()
                .ok_or_else(|| ApiError::bad_request("peer gone"))?
        };
        forward_to_peer(&peer, &frame)
            .await
            .map_err(|e| ApiError::internal(format!("forward failed: {e}")))?;
    }

    Ok(Json(Ack { ok: true }))
}

/// List proposals awaiting this engine's human gate (drives the dashboard).
async fn pending(
    State(st): State<Arc<ServerState>>,
) -> Result<Json<Vec<PendingProposal>>, ApiError> {
    let pending = st.capcom.pending.lock().unwrap();
    Ok(Json(pending.values().cloned().collect()))
}

// ── Compose: natural language → CapcomProposal ─────────────────────
//
// The engine's own `claude -p` (already installed + authenticated) does the
// real reasoning; a keyword heuristic is the fallback so the demo never breaks
// if Claude is missing or slow (>6s). Self-contained — does NOT touch the
// terminal-manager session/streaming machinery.

/// Turn a free-text instruction into a structured proposal. Always succeeds:
/// Claude when available, heuristic otherwise. `source` reports which ran.
async fn compose(
    State(_st): State<Arc<ServerState>>,
    Json(req): Json<ComposeRequest>,
) -> Result<Json<ComposeResponse>, ApiError> {
    let pid = uuid::Uuid::new_v4().to_string();

    // Try the engine's Claude first (real reasoning).
    if let Some(proposal) = compose_with_claude(&req, &pid).await {
        return Ok(Json(ComposeResponse {
            proposal,
            source: "claude".into(),
        }));
    }
    // Fallback: never fail the demo. Log the degradation so it isn't silent.
    tracing::warn!("[capcom] compose: Claude unavailable or slow — using heuristic fallback");
    let proposal = compose_fallback(&req, &pid);
    Ok(Json(ComposeResponse {
        proposal,
        source: "fallback".into(),
    }))
}

/// Invoke `claude -p` (blocking, 6s hard timeout) and parse its JSON into a
/// proposal. Returns `None` on any failure so the caller falls back.
async fn compose_with_claude(req: &ComposeRequest, pid: &str) -> Option<CapcomProposal> {
    if !houston_terminal_manager::claude_path::is_claude_available() {
        return None;
    }

    let system = format!(
        "You convert a user instruction into a CAPCOM proposal for agent-to-agent \
         negotiation. Output ONLY minified JSON, no prose, matching exactly: \
         {{\"intent\":\"lead_handoff|data_share|meeting|other\",\"subject\":\"short title\",\
         \"terms\":{{...relevant key/values...}},\"message\":\"one sentence to the peer\"}}. \
         from_agent={} to_agent={}.",
        req.from_agent, req.to_agent
    );

    // Feed the resolved shell PATH so the engine-installed `claude` resolves
    // even when the process PATH is minimal — same pattern as the other CLI
    // spawns (claude_runner / provider_oneshot). `is_claude_available()` checks
    // against this same PATH, so the spawn must use it too.
    let out = tokio::process::Command::new("claude")
        .env("PATH", houston_terminal_manager::claude_path::shell_path())
        .arg("-p")
        .arg("--output-format")
        .arg("text")
        .arg("--system-prompt")
        .arg(&system)
        .arg(&req.instruction)
        .output();

    // Hard timeout so a slow CLI never stalls the request (>6s → fallback).
    let out = tokio::time::timeout(std::time::Duration::from_secs(6), out)
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout);

    // Claude may wrap JSON in prose/fences — extract the first {...} block.
    let json_str = extract_json_object(&raw)?;
    #[derive(Deserialize)]
    struct Parsed {
        intent: String,
        subject: String,
        terms: serde_json::Value,
        message: String,
    }
    let p: Parsed = serde_json::from_str(json_str).ok()?;

    Some(CapcomProposal {
        proposal_id: pid.to_string(),
        from_agent: req.from_agent.clone(),
        to_agent: req.to_agent.clone(),
        intent: match p.intent.as_str() {
            "lead_handoff" => ProposalIntent::LeadHandoff,
            "meeting" => ProposalIntent::Meeting,
            "data_share" => ProposalIntent::DataShare,
            _ => ProposalIntent::Other,
        },
        subject: p.subject,
        terms: p.terms,
        message: p.message,
        requires_approval: true,
    })
}

/// Extract the first `{...}` slice from Claude's output (it may wrap the JSON
/// in prose or code fences).
fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end > start {
        Some(&s[start..=end])
    } else {
        None
    }
}

/// Heuristic fallback: keyword-sniff the intent and echo the instruction.
/// Dumb but always works, so the demo never depends on Claude being up.
fn compose_fallback(req: &ComposeRequest, pid: &str) -> CapcomProposal {
    let lower = req.instruction.to_lowercase();
    let intent = if lower.contains("skill") {
        ProposalIntent::Other
    } else if lower.contains("review") || lower.contains("contract") {
        ProposalIntent::DataShare
    } else if lower.contains("meet") || lower.contains("intro") {
        ProposalIntent::Meeting
    } else {
        ProposalIntent::LeadHandoff
    };
    CapcomProposal {
        proposal_id: pid.to_string(),
        from_agent: req.from_agent.clone(),
        to_agent: req.to_agent.clone(),
        intent,
        subject: req.instruction.chars().take(60).collect(),
        terms: serde_json::json!({ "instruction": req.instruction }),
        message: req.instruction.clone(),
        requires_approval: true,
    }
}

// ── Peer HTTP client ───────────────────────────────────────────────

/// POST a frame to a peer engine's `/v1/capcom/inbound`.
async fn forward_to_peer(
    peer: &PeerEndpoint,
    frame: &CapcomFrame,
) -> Result<(), reqwest::Error> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Body<'a> {
        from_peer_id: &'a str,
        frame: &'a CapcomFrame,
    }
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/capcom/inbound", peer.base_url))
        .bearer_auth(&peer.token)
        .json(&Body {
            // Announce ourselves by the id THIS peer knows us as (self_id),
            // NOT peer.peer_id — that's how WE address THEM. Sending the
            // recipient's id makes the receiver log an "unknown peer".
            from_peer_id: &peer.self_id,
            frame,
        })
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// POST a frame to the Cloudflare relay's room mailbox, addressed to a peer
/// agent id (Camino A). Used instead of [`forward_to_peer`] when relay mode is
/// enabled. The relay holds the frame until the recipient's poller pulls it.
async fn forward_via_relay(
    relay: &RelayConfig,
    to_agent: &str,
    frame: &CapcomFrame,
) -> Result<(), reqwest::Error> {
    #[derive(Serialize)]
    struct Body<'a> {
        from: &'a str,
        to: &'a str,
        frame: &'a CapcomFrame,
    }
    reqwest::Client::new()
        .post(format!("{}/room/{}/send", relay.url, relay.room))
        .bearer_auth(&relay.token)
        .json(&Body {
            from: &relay.self_id,
            to: to_agent,
            frame,
        })
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

fn unknown_card(id: &str) -> AgentCard {
    AgentCard {
        id: id.to_string(),
        name: format!("Unknown peer ({id})"),
        role: "unknown".into(),
        skills: vec![],
        integrations: vec![],
    }
}
