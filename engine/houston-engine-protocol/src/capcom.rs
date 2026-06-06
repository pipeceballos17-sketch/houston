//! CAPCOM — agent-to-agent negotiation protocol.
//!
//! Additive extension to the Houston engine protocol. Agents belonging to
//! DIFFERENT users negotiate over the relay; every cross-user action passes
//! through a human approval gate on both sides.
//!
//! Lock-step with `houston-relay/src/capcom-types.ts`. Wire shapes here are
//! the source of truth — the relay mirrors them.
//!
//! ## Where the types live
//!
//! The shared negotiation vocabulary — [`AgentCard`], [`ProposalIntent`],
//! [`CapcomProposal`], [`ApprovalDirection`] and [`ApprovalRequest`] — is
//! DEFINED in `houston-ui-events` and re-exported here. Reason: the
//! human-in-the-loop gate is a `HoustonEvent::ApprovalRequest` variant, so its
//! payload type must be visible to `houston-ui-events`. Since
//! `houston-engine-protocol` already depends on `houston-ui-events` (and never
//! the other way round), hosting the payload in the protocol crate would form
//! a dependency cycle (`protocol -> ui-events -> protocol`). We keep the edge
//! one-way — exactly the trick `ui-events` already uses for `ClaudeInstallError`
//! — and re-export so every consumer still imports from
//! `houston_engine_protocol::capcom::*`.
//!
//! `CapcomFrame` (the relay-routed negotiation frames) and `ApprovalDecision`
//! (the REST verdict body) carry no `HoustonEvent` obligation, so they live
//! here in the protocol crate.

use serde::{Deserialize, Serialize};

// Shared vocabulary — owned by `houston-ui-events`, re-exported so the public
// path stays `houston_engine_protocol::capcom::*` (see module docs above).
pub use houston_ui_events::{
    AgentCard, ApprovalDirection, ApprovalRequest, CapcomProposal, ProposalIntent,
};

// ── Handshake / negotiation frames ─────────────────────────────────
//
// Carried in an EngineEnvelope with `kind: Handshake`, routed peer↔peer by
// the relay's TunnelRoom. Correlated by the envelope `id` (the thread) and
// by `proposal_id` (the deal under negotiation).

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "frame", rename_all = "snake_case")]
pub enum CapcomFrame {
    /// Opener: "here's who I am, want to talk?"
    Hello { card: AgentCard },
    /// Reply to HELLO: "here's who I am back."
    Ack { card: AgentCard },
    /// A makes an offer.
    Proposal { proposal: CapcomProposal },
    /// B counter-offers (negotiation can bounce N times).
    Counter { proposal: CapcomProposal },
    /// Deal accepted — BOTH human gates passed.
    Ready { proposal_id: String },
    /// Either side declines. `reason` is shown to the other human.
    Reject { proposal_id: String, reason: String },
}

// ── Human verdict ──────────────────────────────────────────────────

/// The human's verdict, POSTed to `/v1/capcom/approve`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecision {
    pub proposal_id: String,
    pub approved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrips() {
        let f = CapcomFrame::Proposal {
            proposal: CapcomProposal {
                proposal_id: "p1".into(),
                from_agent: "a".into(),
                to_agent: "b".into(),
                intent: ProposalIntent::LeadHandoff,
                subject: "Lead: Acme".into(),
                terms: serde_json::json!({ "leadEmail": "buyer@acme.com", "value": 5000 }),
                message: "Warm lead, handing off.".into(),
                requires_approval: true,
            },
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("\"frame\":\"proposal\""));
        let back: CapcomFrame = serde_json::from_str(&s).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn hello_tag_is_snake_case() {
        let f = CapcomFrame::Hello {
            card: AgentCard {
                id: "a".into(),
                name: "Outbound".into(),
                role: "prospecting".into(),
                skills: vec!["email-outreach".into()],
                integrations: vec!["gmail".into()],
            },
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("\"frame\":\"hello\""));
    }
}
