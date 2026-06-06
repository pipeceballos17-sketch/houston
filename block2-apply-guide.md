# Block 2 — Apply guide (paste into Claude Code)

Camino B: direct A↔B HTTP handshake with bilateral human gates. Replaces the
block-1 route skeleton with real handlers. Additive to ServerState.

## Edit 1 — replace the route skeleton

Overwrite `engine/houston-engine-server/src/routes/capcom.rs` with the
contents of `capcom-routes-block2.rs`.

## Edit 2 — add CapcomState to ServerState

File: `engine/houston-engine-server/src/state.rs`

- Add the field to `struct ServerState`:
  ```rust
  /// CAPCOM agent-to-agent negotiation state (pending proposals, peers).
  pub capcom: Arc<crate::routes::capcom::CapcomState>,
  ```
- In `with_db(...)` where the struct is built, initialise it:
  ```rust
  capcom: Arc::new(crate::routes::capcom::CapcomState::default()),
  ```

## Edit 3 — confirm HoustonEvent::Toast exists

The handlers emit `HoustonEvent::Toast { message }`. Block 1 confirmed Toast
is already a variant (it is). If the field name differs (e.g. `text`), adjust
the two `.emit(HoustonEvent::Toast { ... })` calls to match.

## Edit 4 — dependencies

`engine/houston-engine-server/Cargo.toml` needs `reqwest` with json + a TLS
backend. Check if it's already a dep (it likely is, transitively). If not:
```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
```

## Edit 5 — make ApprovalRequest carry through HoustonEvent

Block 1 added `HoustonEvent::ApprovalRequest(ApprovalRequest)`. Confirm the
import path in capcom.rs matches where the struct actually lives (ui-events
vs protocol re-export). Fix the `use` line if Claude Code put it elsewhere.

## Verify

```bash
cargo build -p houston-engine-server
cargo test -p houston-engine-protocol capcom
```

Green → commit + push (Railway auto-redeploys both engines):
```bash
git add -A
git commit -m "CAPCOM block 2: direct A-B handshake handlers + bilateral HITL gate"
git push
```

Railway redeploys A and B from the capcom branch automatically. Wait for both
to go Online, then run the end-to-end test below.
