# CAPCOM — Agent-to-Agent Negotiation with Human-in-the-Loop

> Built on Houston. Like NASA's CAPCOM, the relay is the single voice that
> mediates between two crews — here, two users' agents that don't know or
> trust each other.

## The feature, in one sentence

Agents belonging to **different users** communicate and negotiate with each
other, and **every action that crosses the boundary between one user and
another passes through human approval** on both sides.

## Canonical use case

User A runs an **outbound** agent (prospecting, sends email). User B runs an
**inbound** agent (manages incoming mail). A has a lead for B. The two agents
negotiate the handoff; each user approves what leaves and what enters their
side.

```
USER A (engine on Railway)      CAPCOM (Cloudflare relay)      USER B (engine on Railway)
 outbound agent                   switchboard / mediator         inbound agent

 1. A ──── HELLO + agent card ────►  route  ────────────────────►  B
 2.                              ◄──── ACK + B's agent card ◄───────
 3. [A's human gate] ◄── approval_request ── "negotiate with B under these terms?"
 4. A approves ──► PROPOSAL {lead, terms, message} ──► route ──►  B
 5.                                              [B's human gate] ◄── "accept this lead?"
 6.                          B approves ──► READY ──► lead enters B's inbound
```

Two human gates, not one. The human-in-the-loop is **bilateral**.

## Three non-negotiable design decisions (and why)

1. **We do not reinvent transport.** We extend Houston's existing
   `EngineEnvelope` (correlated by `id`) and the existing `TunnelRoom`
   Durable Object. The judges are the Houston team — they recognize their
   own protocol used well. This beats anything new and fragile.

2. **CAPCOM is the switchboard.** The relay is the only point that talks to
   both crews. Tenant isolation comes free from the per-tunnel Durable
   Object model — this also satisfies the infra track ("many OpenClaw on one
   host without one accessing another").

3. **HITL rides the existing card.** Every kanban card is already a Claude
   conversation. The approval gate is a new `HoustonEvent` variant — same
   pattern as the existing `AuthRequired` event. No new UI surface invented.

## The negotiation payload — structured with a free-text field

Best of both worlds: validatable + showable in the approval gate, still
expressive. The message field lets Claude reason in natural language inside
a typed envelope.

```jsonc
// CapcomProposal
{
  "proposalId": "uuid",
  "fromAgent": "agent-card-id-A",
  "toAgent": "agent-card-id-B",
  "intent": "lead_handoff",       // enum: lead_handoff | meeting | data_share | ...
  "subject": "Lead: Acme Corp",
  "terms": {                      // structured, intent-specific
    "leadEmail": "buyer@acme.com",
    "value": 5000,
    "deadline": "2026-06-20"
  },
  "message": "Warm lead, replied twice. Handing off — your inbound flow is better fit.",
  "requiresApproval": true
}
```

---

## Where each piece lives in the Houston codebase

| CAPCOM piece | Houston location | What we do |
|---|---|---|
| Handshake frame | `engine/houston-engine-protocol/src/lib.rs` | Add `EnvelopeKind::Handshake`; add `CapcomFrame` enum (HELLO/ACK/PROPOSAL/COUNTER/READY/REJECT) |
| Agent card | `engine/houston-skills/src/lib.rs` (`SkillSummary`) | Reuse: card = `{id, name, skills[], integrations[]}` |
| HITL gate | `engine/houston-ui-events/src/lib.rs` (`HoustonEvent`) | Add `ApprovalRequest { proposal, peer }` variant — mirrors existing `AuthRequired` |
| Peer routing | `houston-relay/src/tunnel-do.ts` (`TunnelRoom`) | Add `role: "peer"`; route frames A↔B, not just desktop→mobile |
| Peer auth | `houston-relay/src/allocate.ts` | Reuse stateless HMAC; peers present tunnel tokens |
| Rust tunnel side | `engine/houston-tunnel/src/frame.rs` | Keep lock-step with `relay/src/types.ts` |
| Engine route | `engine/houston-engine-server/src/routes/` | New `capcom.rs` router: `/v1/capcom/propose`, `/approve`, `/peers` |
| Demo dashboard | new `capcom-dashboard/` (Vercel) | Judge view: live negotiation + both gates |

## Wire protocol additions (lock-step Rust ↔ TS)

```rust
// houston-engine-protocol/src/lib.rs
pub enum EnvelopeKind { Event, Req, Res, Ping, Pong, Handshake }  // + Handshake

pub enum CapcomFrame {
    Hello   { card: AgentCard },
    Ack     { card: AgentCard },
    Proposal{ proposal: CapcomProposal },
    Counter { proposal: CapcomProposal },   // negotiation: B counter-offers
    Ready   { proposal_id: String },        // deal accepted, both sides approved
    Reject  { proposal_id: String, reason: String },
}
```

```rust
// houston-ui-events/src/lib.rs — new HoustonEvent variant (HITL)
ApprovalRequest {
    proposal: CapcomProposal,
    peer: AgentCard,
    direction: ApprovalDirection,  // Outbound (leaving) | Inbound (entering)
}
```

## Infra map (confirmed resources)

- **Railway** — two engine services from `always-on/Dockerfile`:
  `capcom-agent-a` and `capcom-agent-b`. Each gets a public TLS domain,
  isolated. Bearer token per service (`HOUSTON_ENGINE_TOKEN`).
- **Cloudflare** — extend `houston-relay` Worker + `TunnelRoom` DO. Free
  tier is enough. This is the switchboard.
- **Vercel** — `capcom-dashboard` (Next.js): the judge-facing view of the
  live negotiation and the two approval gates.
- **Repo** — fork of `gethouston/houston`. Everything additive → PR-able.

## 8-hour plan (10:00 → 18:00)

| Block | Time | What | Owner |
|---|---|---|---|
| 0. Setup | 10–11 | Fork repo. Deploy 2 engines to Railway from `always-on/Dockerfile`. Smoke `/v1/health` on both. Relay deployed to Cloudflare. | You (infra) |
| 1. Protocol | 11–13 | `EnvelopeKind::Handshake` + `CapcomFrame` + `AgentCard` (Rust). Lock-step TS types in relay. | Claude Code |
| 2. Peer routing | 13–14:30 | `TunnelRoom` `role:"peer"`, route A↔B. HMAC peer auth. | You + Claude Code |
| 3. HITL gate | 14:30–16 | `ApprovalRequest` event + `/v1/capcom/approve`. Pause thread, resume on approve. | Claude Code |
| 4. Demo task | 16–17:30 | Outbound A proposes lead → B counters → both humans approve → lead lands. | Both |
| 5. Polish + pitch | 17:30–18 | Dashboard on Vercel, demo script, fallbacks, README. | Both |

## Definition of done (demo)

1. Two engines live on Railway, isolated, both healthy.
2. A's outbound agent sends a structured PROPOSAL to B through the relay.
3. A's human approves the outbound; B's human approves the inbound. Both
   gates visible in the dashboard.
4. On double-approval, the lead lands in B's inbound agent — visible as a
   new card / conversation.
5. Reject path works: either human rejects → thread closes cleanly.

## Pitch framing (30 sec)

"Houston gives one user agents that do real work. CAPCOM lets *different
users'* agents negotiate with each other — an outbound agent handing a lead
to someone else's inbound agent — with a human approval gate on both sides.
We didn't build a new protocol; we extended Houston's envelope and relay, so
it's a PR, not a side project."
