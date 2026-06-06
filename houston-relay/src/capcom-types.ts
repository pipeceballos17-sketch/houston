// CAPCOM wire types — mirror of engine/houston-engine-protocol/src/capcom.rs.
// MUST stay in lock-step with the Rust side. Lives in houston-relay/src/.

export interface AgentCard {
  id: string;
  name: string;
  role: string;
  skills: string[];
  integrations: string[];
}

export type ProposalIntent = "lead_handoff" | "meeting" | "data_share" | "other";

export interface CapcomProposal {
  proposalId: string;
  fromAgent: string;
  toAgent: string;
  intent: ProposalIntent;
  subject: string;
  terms: Record<string, unknown>;
  message: string;
  requiresApproval: boolean;
}

// Discriminated union on `frame` — matches serde(tag = "frame").
export type CapcomFrame =
  | { frame: "hello"; card: AgentCard }
  | { frame: "ack"; card: AgentCard }
  | { frame: "proposal"; proposal: CapcomProposal }
  | { frame: "counter"; proposal: CapcomProposal }
  | { frame: "ready"; proposalId: string }
  | { frame: "reject"; proposalId: string; reason: string };

export type ApprovalDirection = "outbound" | "inbound";

export interface ApprovalRequest {
  proposal: CapcomProposal;
  peer: AgentCard;
  direction: ApprovalDirection;
}
