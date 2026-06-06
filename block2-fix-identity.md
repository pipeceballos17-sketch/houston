# Block 2 fix — peer identity bug ("Unknown peer" + "peer gone")

## The bug

When A forwards an approved proposal to B, `forward_to_peer` identifies the
sender using the *recipient's* id (`peer.peer_id`), so A tells B "I am
agent-b" instead of "I am agent-a". B then can't resolve the sender (shows
"Unknown peer (agent-b)") and when B tries to reply, the lookup fails with
"peer gone".

The sender must announce *its own* id, not the recipient's.

## Fix — 2 edits in `engine/houston-engine-server/src/routes/capcom.rs`

### Edit 1 — add `self_id` to PeerEndpoint

Find the `PeerEndpoint` struct and add a field:

```rust
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerEndpoint {
    pub peer_id: String,
    /// The id THIS engine announces to this peer (i.e. "who am I to them").
    pub self_id: String,
    pub base_url: String,
    pub token: String,
    pub card: AgentCard,
}
```

### Edit 2 — use `self_id` when forwarding

In `forward_to_peer`, change the `from_peer_id` line. The function signature
already receives `peer: &PeerEndpoint`, so:

```rust
        .json(&Body {
            from_peer_id: &peer.self_id,   // was: &peer.peer_id
            frame,
        })
```

That's it — two lines. Rebuild, push, Railway redeploys.

```bash
cargo build -p houston-engine-server   # (will skip locally — Railway verifies)
git add -A
git commit -m "CAPCOM block 2 fix: announce sender self_id when forwarding"
git push
```
