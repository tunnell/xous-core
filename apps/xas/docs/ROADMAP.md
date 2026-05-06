# Xous Signal Client — MVP Roadmap

A staged plan for building a Signal client on Xous from a fresh repo, following the design in [`REPORT.md`](./REPORT.md). Each stage is sized to be handed to a single Claude Code session, with clear inputs, outputs, and verification.

**MVP definition.** Three hardware-confirmed flows: (1) link as secondary device, (2) receive one message, (3) send one message. Everything in service of those three.

**Stages 0–7 are hosted-mode-only** (Linux). Stages 8–12 require Precursor hardware (or Renode).

**Recommended order: 0 → 1 → 2 → 3 → 6 → 7 → 4 → 5 → 8 → 6.5 → 9a → 9b → 10 → 11 → 12.** This is **not** the numerical sequence. Stages 6 (libsignal-service-rs transport fork) and 7 (presage tokio removal) come before Stages 4 and 5 (storage trait impls) because rv32 cross-compile of anything that pulls `presage` is gated on the tokio + reqwest removal — the transitive `mio` dep doesn't compile on rv32-xous. Doing 6+7 first keeps rv32 verification unbroken throughout the project. Stage 4 partial work (Cargo dep wiring + the curve25519-dalek vendoring) can happen before 6+7 to validate that the storage crate's deps resolve cleanly; full Stage 4 implementation (`StateStore` impl + tests) waits until after 6+7 so it gets rv32 verification on its first commit. Stage 6.5 (rv32 verification of libsignal core crates) is best done after the storage trait surface lands so a real rv32 build of the workspace exists to verify against; it surfaces any C/`*-sys`/`tokio` leaks the cryptography.rs adoption matrix didn't predict before they spread further into the integration work.

The numerical numbering is preserved for stage-section references but doesn't dictate temporal order. Each stage's `Prerequisites` line is the authoritative dependency.

---

## How each stage works

Each stage below is a self-contained agent prompt. Hand the whole stage section to Claude Code with:

> Read `ROADMAP.md` Stage N. Do everything it says. Cite source for non-trivial claims. When done, run the verification step and report results.

The agent should always have read access to:

- `REPORT.md` — the design rationale + storage layout + tokio-removal patch table
- `CALL_GRAPH.md` — the per-command call graph for what each flow needs
- The cloned upstream repos at `~/precursor-signal/repos/{presage, libsignal-service-rs, libsignal, async-executor, async-task, async-lock, async-channel, event-listener, futures-lite, futures-timer, signal-tungstenite, tungstenite-rs, gurk-rs, purple-presage}` — for reading.

Stop conditions for every stage:
- Verification fails: don't move on; either fix or surface.
- The stage scope grows beyond ~1.5× the original estimate: stop, surface, re-scope.
- Verification reveals an assumption in `REPORT.md` is wrong: stop, document the discrepancy, ask before changing.

---

## Stage 0 — Repository scaffolding and workspace

**Goal.** Stand up an empty workspace that compiles to nothing useful but uses every `[patch.crates-io]` entry from xous-core, has the right Rust toolchain pinned, and has a coherent crate layout for the work to come.

**Prerequisites.**
- `~/precursor-signal/REPORT.md` and `CALL_GRAPH.md` checked in.
- xous-core cloned at `~/precursor-signal/repos/xous-core` for reference.

**Deliverables.**

```
xous-app-signal/
├── Cargo.toml                      # workspace
├── rust-toolchain.toml             # 1.85 stable (or matching nightly if Xous fork requires)
├── README.md
├── AGENTS.md                       # project conventions
├── docs/
│   ├── REPORT.md                   # symlink or copy
│   ├── CALL_GRAPH.md
│   └── ROADMAP.md
├── crates/
│   ├── presage-store-pddb/         # the 9 storage trait impls (skeleton)
│   ├── xous-net-bridge/            # sync TLS+WS pump (skeleton)
│   ├── xous-signal-bridge/         # Manager-on-worker + IPC forwarder (skeleton)
│   └── xous-app-signal/            # binary entry point; on-device binary name `xas`
└── stage/                          # per-stage execution reports (REPORT-N.md)
```

**Steps.**

1. Create workspace `Cargo.toml` with members for the four crates above. Set `[workspace.package]` with edition `"2024"`, `rust-version = "1.85"`.
2. Mirror xous-core's `[profile.release]` from [`xous-core/Cargo.toml:154-161`](https://github.com/betrusted-io/xous-core/blob/main/Cargo.toml#L154-L161): `codegen-units = 1`, `lto = "fat"`, `opt-level = "s"`, `incremental = true`, `debug = true`, `strip = false`. Plus a `[profile.release-small]` variant with `debug = false`, `strip = true` for size measurement (per `REPORT.md` § Binary size strategy).
3. Add `[patch.crates-io]` entries mirroring xous-core's **git-based** patches at [`Cargo.toml:165-196`](https://github.com/betrusted-io/xous-core/blob/main/Cargo.toml#L164-L196): `sha2` (`betrusted-io/hashes`, branch `sha2-v0.10.8-xous`) and `ring` (`betrusted-io/ring-xous` at the pinned rev). The path-based patches in xous-core (`aes` → `services/aes`, `getrandom` → `imports/getrandom`) cannot be mirrored 1:1 in a standalone workspace — those paths only exist inside xous-core's tree. Defer them to Stage 9 (rv32 hardware integration), at which point we either merge into xous-core's tree or vendor the forks. Add a TODO comment in the workspace `Cargo.toml` noting this. **Do not** copy xous-core's `curve25519-dalek` patch — see `REPORT.md` Risk #3.
4. For each of the four crates, generate a stub `Cargo.toml` and `src/lib.rs` (or `main.rs` for the app). Each `lib.rs` should be empty except for a `//! Crate description` doc comment.
5. Write `AGENTS.md` with project conventions:
   - Cite source for non-trivial claims (`repo/path:line`).
   - No emojis in code.
   - Run `cargo fmt + cargo clippy` before committing.
   - Don't add deps without checking `[patch.crates-io]` in xous-core first.
   - Reference `REPORT.md` for design decisions; if the design seems wrong, surface to user before changing.
6. Write `README.md`: one-paragraph project description, link to `docs/REPORT.md`, build instructions (`cargo build --workspace`), test instructions.
7. Add `rust-toolchain.toml` with `channel = "1.95.0"` (or the version the user's Xous fork actually has installed; if unknown, default to `stable` and let it resolve).

**Verification.**

```
cargo build --workspace                   # compiles cleanly to no-op binary
cargo build --workspace --release         # release profile works
cargo build --workspace --profile=release-small   # size-measurement profile works
cargo tree --workspace -d                 # no duplicate dep versions
cargo run --bin xas                       # binary name is correct, runs and exits cleanly
cargo fmt --all -- --check                # formatting is clean
cargo clippy --workspace --all-targets -- -D warnings   # no clippy warnings
```

The workspace must compile cleanly with a no-op binary. `cargo tree -d` must show zero duplicates (we'll watch this every stage). The fmt + clippy checks are required-before-commit per `AGENTS.md`; making them part of stage verification keeps the next agent honest.

**Out of scope.** Anything that uses smol, presage, libsignal — those come later. This stage is just the empty harness.

---

## Stage 1 — Vendor smol primitives, write a `LocalExecutor` smoke test

**Goal.** Add the seven smol-rs crates as git deps with pinned revs, write a tiny binary that uses `LocalExecutor + futures-timer + async-channel` to prove the runtime works.

**Prerequisites.** Stage 0 complete.

**Deliverables.**

- Workspace `Cargo.toml` updated with `[workspace.dependencies]` for the seven smol crates, pinned to specific git revs (the HEADs of the cloned reference copies in `~/precursor-signal/repos/`).
- `crates/xous-app-signal/src/main.rs` does the smoke test:
  - Spawn task A on `LocalExecutor` that sleeps 100 ms and sends a message.
  - Task B receives the message, prints it.
  - Drive both via `LocalExecutor::run(future)`.
- Hosted-mode `cargo run -p xous-app-signal --bin xas` prints expected output and exits cleanly.

**Steps.**

1. Add `[workspace.dependencies]` entries:
   - `async-task` (git, pin to current main HEAD)
   - `async-executor` (git, pin)
   - `async-channel` (git, pin)
   - `async-lock` (git, pin)
   - `event-listener` (git, pin)
   - `futures-lite` (git, pin)
   - `futures-timer` (git, pin)
   
   Use the rev hashes from each cloned repo's `git log --oneline -1`.
2. In `crates/xous-app-signal/Cargo.toml`, add the deps it needs (`async-executor`, `async-channel`, `futures-timer`, `futures-lite`).
3. In `crates/xous-app-signal/src/main.rs`, write a `fn main()` that:
   - Constructs a `LocalExecutor::new()`.
   - Creates an `async-channel::bounded::<&'static str>(1)`.
   - Spawns task A: `async { futures_timer::Delay::new(Duration::from_millis(100)).await; tx.send("hello").await.unwrap(); }`.
   - Drives the executor with `block_on(executor.run(async { let msg = rx.recv().await.unwrap(); println!("got: {msg}"); }))`.
4. Verify hosted-mode output matches expectation.

**Verification.**

```
cargo run -p xous-app-signal --bin xas
# Expected output:
# got: hello

cargo check --target=riscv32imac-unknown-xous-elf -p xous-app-signal
# Cross-compile sanity for Xous rv32; should compile cleanly.
# This catches dep-tree problems on rv32 (assembly cfgs, std assumptions, etc.)
# without needing a Renode boot test. Free correctness verification per stage.

cargo tree --workspace -d
# Zero duplicate dep versions
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

**Renode test status.** Renode boot tests start at Stage 9 (the first stage with a Xous app entry point that produces UART output). For Stages 1–8, the rv32 verification is `cargo check --target=riscv32imac-unknown-xous-elf` only — that catches dep-tree problems early without needing a full image build.

**Stop conditions.**
- If `futures-timer` doesn't compile cleanly on hosted Linux: it should; if not, file a bug.
- If LocalExecutor's `run` API differs from what's documented in `REPORT.md`: reread the actual source at `~/precursor-signal/repos/async-executor/src/lib.rs:441-650` and reconcile.
- If the rv32 cross-compile fails: this is real signal, not noise. Stop and diagnose. Likely cause is a smol primitive pulling in a transitive dep with an x86-asm cfg or a std assumption that doesn't hold on Xous.

---

## Stage 2 — TLS smoke test using Xous-style stack

**Goal.** Demonstrate sync HTTPS over rustls 0.22.2 against a real public endpoint, using only deps that work on rv32. Hosted-mode for now; rv32 build verified later.

**Prerequisites.** Stage 1 complete.

**Deliverables.**

- New crate `crates/xous-net-bridge/` populated with:
  - `src/tls.rs`: a `tls_connect(host: &str, port: u16, alpn: &[&[u8]]) -> std::io::Result<rustls::StreamOwned<ClientConnection, TcpStream>>` function.
  - `src/lib.rs`: re-exports.
- A new `examples/https_get.rs` in `xous-net-bridge` that:
  - Uses `tls_connect` to connect to `https://example.com:443`.
  - Sends `GET / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n`.
  - Reads response until EOF.
  - Prints status line.

**Steps.**

1. Add `rustls = "0.22.2"` (the version xous-core uses; see [`libs/tls/Cargo.toml:31`](https://github.com/betrusted-io/xous-core/blob/main/libs/tls/Cargo.toml#L31)) and `webpki-roots` to `xous-net-bridge/Cargo.toml`.
2. Implement `tls_connect`. On hosted-mode it uses `std::net::TcpStream`; later on Xous it'll use the equivalent. Keep the API the same.
3. Use `rustls::ClientConfig::builder().with_root_certificates(...).with_no_client_auth()` with `webpki-roots` as the trust anchors.
4. Write the example. Verify it prints `HTTP/1.1 200 OK`.

**Verification.**

```
cargo run --example https_get -p xous-net-bridge
# Expected:
# HTTP/1.1 200 OK

cargo check --target=riscv32imac-unknown-xous-elf -p xous-net-bridge
# rv32 cross-compile sanity. rustls + ring on rv32 is the first real test
# of xous-core's [patch.crates-io].ring patch — if this fails, the patch
# isn't being inherited correctly.

cargo tree --workspace -d
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

**Out of scope.** Anything async. Anything Signal-specific. Just rustls + sync TCP.

---

## Stage 3 — WebSocket smoke test against Signal's keepalive endpoint

**Goal.** Establish a WSS connection to `wss://chat.signal.org/v1/keepalive`, read one frame, close cleanly. Validates that sync `tungstenite` + rustls + DNS can speak Signal's WebSocket layer.

**Prerequisites.** Stage 2 complete.

**Deliverables.**

- `xous-net-bridge/src/ws.rs` exposing `pub fn ws_connect(host: &str, path: &str) -> Result<WebSocket<RustlsStream>, Error>`.
- An example `examples/signal_ws_keepalive.rs` that connects, reads one frame (or times out after 60s), prints, closes.

**Steps.**

1. Add `tungstenite = "0.29.0"` (upstream `snapview/tungstenite-rs`, sync) to `xous-net-bridge/Cargo.toml`. Per `REPORT.md` §Decision 3, **don't** use `signalapp/tungstenite-rs` (that fork only adds permessage-deflate which Signal's WS doesn't require).
2. Implement `ws_connect` building on Stage 2's `tls_connect`: open TCP, wrap in TLS, do `tungstenite::client(...)` handshake.
3. The Signal WS expects an authenticated connection for most paths. `/v1/keepalive` (without `/provisioning/`) requires auth and will reject. **For the smoke test, use `/v1/keepalive/provisioning`** which is the unauth provisioning channel — it's the same endpoint `link_device` uses (`libsignal-service-rs/src/provisioning/mod.rs:163-170`). Connect, wait for a single frame or 60s timeout, close.
4. Verify: hosted-mode example connects without TLS errors and the connection stays open.

**Verification.**

```
cargo run --example signal_ws_keepalive -p xous-net-bridge
# Expected: WS handshake succeeds, connection idles for 60s, closes cleanly

cargo check --target=riscv32imac-unknown-xous-elf -p xous-net-bridge
# Confirms tungstenite + utf-8 + sha1 + http crates all build for rv32.

cargo tree --workspace -d
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

**Out of scope.** Async, channel bridge to executor (that's Stage 7). Signal protocol parsing. Auth.

---

## Stage 4 — `presage-store-pddb`: skeleton + `StateStore` + `ContentsStore::profile`

**Goal.** Stand up the storage crate with one trait fully implemented end-to-end, plus a tiny ContentsStore method, so we have a verified end-to-end pattern before scaling up.

**Prerequisites.** Stage 3 + Stage 7 complete (Stage 7 removes tokio from presage; without it, rv32 cross-compile of `presage-store-pddb` is blocked by the `mio` transitive dep). The Cargo dep wiring + curve25519-dalek vendoring (Step 1 below) can land before Stage 7; the actual `StateStore` impl + tests should wait until after Stage 7 so that the per-stage rv32 verification passes on the first commit.

Familiarity with `REPORT.md` §Decision 1 (dictionary layout), §Decision 6 (curve25519-dalek strategy), and `CALL_GRAPH.md` §1 (storage trait reference).

**Deliverables.**

- `crates/presage-store-pddb/src/`:
  - `lib.rs` — `pub struct PddbStore { ... }` with `Clone + Send + Sync` (per presage's `Store` trait bounds; see [`presage/src/store.rs:333-339`](https://github.com/whisperfish/presage/blob/main/presage/src/store.rs#L333-L339)).
  - `state.rs` — full `StateStore` impl (9 methods).
  - `error.rs` — error type implementing `presage::store::StoreError`.
- A **hosted-mode mock backend** (`backend_mock.rs`) — an in-memory `HashMap<(dict, key), Vec<u8>>` behind the same internal API the real PDDB backend will eventually expose. This lets us test on Linux without Xous.
- Unit tests in `state.rs` — round-trip RegistrationData, identity keypairs, sender certificate.

**Steps.**

1. **(Lands early — can be done before Stage 6+7.)** Cargo dep wiring + curve25519-dalek vendoring:
   - Add `presage = { git = "...", rev = "..." }` to `presage-store-pddb/Cargo.toml`.
   - Vendor `betrusted-io/curve25519-dalek` (HW-accelerated for Precursor's curve25519 IP core; driver at `curve25519-dalek/src/backend/serial/u32e/`) into `vendor/curve25519-dalek/`.
   - Bump `version = "4.1.2"` → `version = "4.1.3"` in `vendor/curve25519-dalek/curve25519-dalek/Cargo.toml` so the `[patch.crates-io]` redirect matches what libsignal's zkgroup declares.
   - Port the `src/lizard/` module from `signalapp/curve25519-dalek` (`signal-curve25519-4.1.3` tag) into the vendored copy — wire it via `pub mod lizard;` in `lib.rs`. Adds 4 methods to `RistrettoPoint`: `lizard_encode<H>`, `lizard_decode<H>`, `from_uniform_bytes_single_elligator`, `decode_253_bits`. All used by zkgroup. Patches are additive — no API conflicts.
   - Add to workspace `Cargo.toml`:
     ```toml
     [patch.crates-io.curve25519-dalek]
     path = "vendor/curve25519-dalek/curve25519-dalek"

     [patch.crates-io.curve25519-dalek-derive]
     path = "vendor/curve25519-dalek/curve25519-dalek-derive"

     [patch."https://github.com/signalapp/curve25519-dalek"]
     curve25519-dalek = { path = "vendor/curve25519-dalek/curve25519-dalek" }
     curve25519-dalek-derive = { path = "vendor/curve25519-dalek/curve25519-dalek-derive" }
     ```
     The git-URL patch is required because libsignal's `Cargo.toml:90` aliases `curve25519-dalek-signal = { git = "...signalapp/...", package = "curve25519-dalek" }`; `[patch.crates-io]` only patches crates.io, not git URLs.
   - Add `.cargo/config.toml` to auto-activate the u32e backend for rv32-xous:
     ```toml
     [target.riscv32imac-unknown-xous-elf]
     rustflags = ["--cfg", "curve25519_dalek_backend=\"u32e_backend\""]
     ```
     Hosted Linux falls back to the portable Rust backend.
   - **Precursor-only HW acceleration.** The IP core is Precursor-only (Bao1x has a different PKE engine; deferred). Per `REPORT.md` §Decision 6.
   - Verify hosted-mode `cargo build -p presage-store-pddb` succeeds (full Whisperfish stack compiles).
2. Define `pub struct PddbStore { backend: Arc<dyn KvBackend>, ... }` where `KvBackend` is an internal trait with `get(dict, key) -> Result<Option<Vec<u8>>>`, `put(dict, key, &[u8])`, `delete(dict, key)`, `delete_dict(dict)`, `list_keys(dict) -> Vec<String>`. The mock backend implements it via `HashMap`.
3. Implement [`StateStore`](https://github.com/whisperfish/presage/blob/main/presage/src/store.rs#L36-L83): all 9 methods. Use one PDDB key per field in a single `signal.state` dictionary, per `REPORT.md` §Decision 1.
4. Implement just `ContentsStore::profile` and `ContentsStore::save_profile` for the round-trip test. Don't try to implement the rest of `ContentsStore` yet — that's Stage 4c.
5. Write unit tests:
   - Empty store: `is_registered() == false`, `load_registration_data() == None`.
   - After `save_registration_data + set_aci_identity_key_pair`: round-trip equals input.
   - After `clear_registration`: state is reset.
   - Profile round-trip.

**Verification.**

```
cargo test -p presage-store-pddb
# All tests pass

cargo check --target=riscv32imac-unknown-xous-elf -p presage-store-pddb
# rv32 cross-compile. Gated on Stage 7 (presage tokio removal) because mio
# (transitively pulled by tokio via reqwest via libsignal-service-rs) doesn't
# compile on rv32-xous. After Stage 7, this passes.

cargo tree --workspace -d
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

**Note.** This stage uses a mock backend. The real PDDB backend is wired up at the integration step (Stage 8). The mock backend stays around forever as the test harness.

---

## Stage 5 — Implement remaining storage traits over PDDB

**Goal.** Complete all 9 traits + presage's `ContentsStore`. After this, the store is feature-complete against the mock backend.

**Prerequisites.** Stages 4 + 7 complete. Same rv32 gating as Stage 4: rv32 cross-compile of the storage crate requires Stage 7's tokio-removal first.

Read `CALL_GRAPH.md` §10 (storage trait → command index) for which methods are hot vs cold.

**This stage can be parallelized** into three sub-stages (5a, 5b, 5c) if convenient.

### Stage 5a — Six libsignal protocol storage traits

**Deliverables.** New module per trait in `presage-store-pddb/src/protocol/`:
- `identity_key_store.rs` — `IdentityKeyStore` (5 methods).
- `pre_key_store.rs` — `PreKeyStore` (3 methods). **Use the packed-key strategy** from `REPORT.md` §Decision 1: a single PDDB key holding `Vec<(PreKeyId, PreKeyRecord)>` rather than per-id keys.
- `signed_pre_key_store.rs` — `SignedPreKeyStore` (2 methods).
- `kyber_pre_key_store.rs` — `KyberPreKeyStore` (3 methods, including `mark_kyber_pre_key_used` last-resort dedup; see [`libsignal/rust/protocol/src/storage/traits.rs:117-140`](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/protocol/src/storage/traits.rs#L117-L140)).
- `session_store.rs` — `SessionStore` (2 methods). **In-memory dirty-set + flush** per `REPORT.md` §Decision 5: `store_session` writes to a `HashMap<ProtocolAddress, SessionRecord>`, flush is a separate method called on quiescence (`Received::QueueEmpty`).
- `sender_key_store.rs` — `SenderKeyStore` (2 methods).

Plus `protocol_store.rs` — `ProtocolStore` blanket impl. ACI vs PNI variants per [`presage/presage/src/store.rs:342-355`](https://github.com/whisperfish/presage/blob/main/presage/src/store.rs#L342-L355).

Per-trait unit tests: round-trip empty → save → load → check.

### Stage 5b — libsignal-service-rs extension traits

**Deliverables.**
- `pre_keys_store.rs` — `PreKeysStore` (8 methods of ID counters and counts; see [`libsignal-service-rs/src/pre_keys.rs:57-90`](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/pre_keys.rs#L57-L90)).
- `kyber_pre_key_store_ext.rs` — `KyberPreKeyStoreExt` (5 methods; see [`pre_keys.rs:23-51`](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/pre_keys.rs#L23-L51)). Includes the staleness-marking methods using `chrono::DateTime<Utc>`.
- `session_store_ext.rs` — `SessionStoreExt` (5 methods; see [`session_store.rs:13-86`](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/session_store.rs#L13-L86)).

Tests: each method's contract.

### Stage 5c — `ContentsStore` (the largest)

**Deliverables.** Complete impl of presage's `ContentsStore` — ~30 methods covering messages-by-thread, contacts, groups, profiles, sticker packs. See [`presage/presage/src/store.rs:86-330`](https://github.com/whisperfish/presage/blob/main/presage/src/store.rs#L86-L330).

Key implementation points:
- Messages are dictionary-per-thread: dict name = `signal.threads.<hex(thread_id)>`. Per `REPORT.md` §Decision 1 dictionary layout.
- `clear_messages(thread)` is implemented as `backend.delete_dict()` (atomic).
- `messages(thread, range: impl RangeBounds<u64>)` lists keys, parses timestamps, filters in-process. Document the cost; cache the index.
- `contacts()`, `groups()` return iterators backed by `list_keys` + per-key reads.

**Verification (whole Stage 5).**

```
cargo test -p presage-store-pddb
# All tests pass for all 9 traits.

cargo check --target=riscv32imac-unknown-xous-elf -p presage-store-pddb
# Full rv32 build of all trait impls. SPQR comes in transitively here via
# libsignal-protocol. If sha2/asm fails, xous-core's [patch.crates-io].sha2
# isn't being inherited correctly.

cargo tree --workspace -d
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

---

## Stage 6 — Fork `libsignal-service-rs`: swap reqwest+reqwest-websocket for sync transport

**Goal.** Get `libsignal-service-rs` to compile and pass its own tests against our Stage 3 sync TLS+WS pump rather than reqwest.

**Prerequisites.** Stage 3 complete (need the sync TLS+WS pump). Stage 4 step 1 (Cargo dep wiring + curve25519-dalek vendoring) should have landed too — without that, the libsignal-service-rs build won't resolve. Stages 4 (full) and 5 are explicitly NOT prerequisites and should come *after* this stage so they can ship with rv32 verification on first commit.

Read `REPORT.md` §Decision 3 (transport replacement) and `CALL_GRAPH.md` §0 (which libsignal-service-rs components are involved).

**Deliverables.**

- New folder `vendor/libsignal-service-rs/` containing the upstream HEAD as a git submodule or path-vendored copy.
- A patch series (or feature branch) on top of upstream that replaces:
  - [`reqwest::Client`](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/push_service/mod.rs#L80) → a small `HttpClient` trait with one impl using `ureq` (sync HTTP/1.1 + rustls) and one in-memory mock for tests.
  - [`reqwest_websocket::WebSocket`](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/websocket/mod.rs#L14) → a `Stream<Item = Result<Frame>>` + `Sink<Frame>` adapter backed by an `async-channel`. The actual sync `tungstenite` pump runs in a separate Xous thread (Stage 7); `libsignal-service-rs` only sees the channel ends.
- The [`tokio::task::spawn(task)`](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/push_service/mod.rs#L184) at `push_service/mod.rs:184` is replaced: `SignalWebSocket::new()` returns the task to the caller; the caller spawns it on `LocalExecutor`. This is one of the two API changes the patch makes.
- Replace `tokio::time::Instant` → `std::time::Instant` and `tokio::time::interval_at` → `futures-timer::Delay`-driven loop in [`websocket/mod.rs:223`](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/websocket/mod.rs#L223).
- `Cargo.toml` adjustments: drop `reqwest`, `reqwest-websocket`, `tokio` (production); add `ureq`, `tungstenite`, `async-channel`, `futures-timer`, `async-lock`.
- `default = []` instead of `default = ["cdsi"]` per `REPORT.md` §Decision 5.
- Build with `--no-default-features` to skip CDSI.

**Steps.**

1. Add `vendor/libsignal-service-rs/` as path dep at `[workspace.dependencies] libsignal-service = { path = "vendor/libsignal-service-rs" }`.
2. Apply the transport patch. Aim for a single feature commit on top of upstream HEAD; total diff ≤ 2 kLoC.
3. Rebuild upstream's smoke tests (those that don't require network) — they should pass.
4. Smoke test: a hosted-mode integration test that mocks the `HttpClient`, mocks the WS as a pre-canned frame stream, runs `link_device` to the point where it would emit a provisioning URL — does it succeed on the mock?

**Verification.**

```
cargo test -p libsignal-service --features=phonenumber --no-default-features
# Existing upstream tests pass
cargo build -p libsignal-service --no-default-features --features=phonenumber
# Builds cleanly without tokio/reqwest in deps
cargo tree -p libsignal-service | grep -E "tokio|reqwest"
# Should be empty (or only dev-dep references)

cargo check --target=riscv32imac-unknown-xous-elf -p libsignal-service --no-default-features --features=phonenumber
# rv32 cross-compile of the forked libsignal-service-rs.

cargo tree --workspace -d
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

**Stop conditions.** If the diff exceeds 3 kLoC, the abstraction boundary is wrong — surface and discuss before continuing.

---

## Stage 6.5 — Verify libsignal core builds clean for rv32-xous

**Goal.** Confirm that the libsignal crates we *transitively* pull through `libsignal-service-rs` and `presage` (`libsignal-protocol`, `signal-crypto`, `libsignal-account-keys`, `poksho`, `zkgroup`, `zkcredential`, `usernames`, `spqr`) build green for `riscv32imac-unknown-xous-elf`, and apply the **minimum** patch series required.

**Why this is its own stage.** A user-supplied analysis (the cryptography.rs adoption memo, see `RESUME.md`) confirms that libsignal v0.90+ migrated from `pqcrypto-kyber` (C-FFI) to `libcrux-ml-kem` (pure Rust). Every other crypto dep on the protocol path is also pure Rust. The transitive C-surface that *would* block rv32 (`boring`, `boring-sys`, `tokio-boring-signal`) lives only in `rust/net*`, `rust/attest`, `rust/keytrans`, `rust/svrb` — none of which we reach. So in principle the libsignal stack should build for rv32 with at most a handful of cfg-guard tweaks. This stage exists to **verify** that, **before** we sink the storage trait surface (Stage 5) or the integration work (Stages 8–9) into a stack that turns out to need rv32-incompatible patches.

In practice we currently sit at the end of Stage 9a with `cargo check --target=riscv32imac-unknown-xous-elf -p xous-app-signal` passing — which means the libsignal core *does* build for rv32 in our specific setup. This stage is now a **structured re-verification** plus an explicit list of the patches that would be needed if upstream regressed.

**Prerequisites.** Stage 6 complete (libsignal-service-rs forked + transport replaced). The vendored `libsignal-service-rs` pulls libsignal-protocol/zkgroup transitively at the v0.91.0 git tag.

**Deliverables.**

- A `cargo tree --target=riscv32imac-unknown-xous-elf -p libsignal-protocol -e features` snapshot, captured in `stage/REPORT-6.5.md`. Grep should return zero matches for `^(boring|boring-sys|openssl|openssl-sys|aws-lc|aws-lc-sys|bindgen|cc|cmake|.*-sys$|tokio|mio|reqwest|hyper|h2)$`. Any match is either a real bug or an incorrectly-activated optional feature.
- A `cargo tree -i sha2 -p libsignal-protocol --target=riscv32imac-unknown-xous-elf` snapshot showing every consumer resolves to `betrusted-io/hashes` (xous-core's `[patch.crates-io].sha2`).
- Same for `getrandom` (single resolved version, 0.3 with `getrandom_backend="custom"`; if 0.2 also appears, document why).
- Same for `curve25519-dalek` (must point at our vendored `apps/xas/vendor/curve25519-dalek/curve25519-dalek/`, version 4.1.3, with the lizard module visible).
- For each cryptographic primitive Signal uses, a row in the **primitive map** (`stage/REPORT-6.5.md` §Primitive map) listing: primitive name, libsignal file:line where it's called, the upstream crate + version, the cryptography.rs-listed equivalent, and rv32 viability. Primitives to enumerate: AES-256, AES-256-CTR, AES-256-CBC, AES-256-GCM-SIV, HMAC-SHA-256, HKDF-SHA-256, SHA-256, SHA-512, X25519, Ed25519, Curve25519 base point ops, Ristretto255, ML-KEM-1024, Argon2id, constant-time eq, zeroize, RNG.
- A "rv32 patch series" against the vendored copies of libsignal that, if needed, applies cfg guards to keep aarch64/x86 SIMD code paths from firing on rv32. Likely empty in our current setup (Stage 9a confirms `cargo check --target=...-xous-elf -p xous-app-signal` passes); record any patches as small diffs in `vendor/libsignal-service-rs/patches/` or whatever location our fork uses.

**Steps.**

1. From the standalone workspace, run:
   ```sh
   cargo tree --target=riscv32imac-unknown-xous-elf -e features -p libsignal-service \
     | tee stage/cargo-tree-libsignal-rv32.txt
   ```
2. Search for transitive C-surface markers:
   ```sh
   grep -E '^(│|├|└|\s)*(boring|boring-sys|openssl|openssl-sys|aws-lc|aws-lc-sys|bindgen|cc|cmake|.*-sys|tokio|mio|reqwest|hyper|h2)\b' \
     stage/cargo-tree-libsignal-rv32.txt
   ```
   Empty output is the goal. Any match: classify as feature-gated (then disable the feature) or genuinely required (then surface to the user).
3. For each primitive listed in the deliverables, find the libsignal file:line of its first call site and record in the primitive map. Sources: `~/precursor-signal/repos/libsignal/rust/{protocol,crypto,account-keys,zkgroup,poksho}/src/`.
4. Verify the cryptography.rs catalog (`https://cryptography.rs/`, fetched 2026-05-06) lists each upstream crate we use as a recommendation, or that the deviation is documented (notably libsignal's `libcrux-ml-kem` over cryptography.rs's `ml-kem` — both pure-Rust; libsignal's choice is formally verified in F\* via the hax toolchain).
5. If `cargo check --target=riscv32imac-unknown-xous-elf -p libsignal-service --no-default-features --features=phonenumber` currently passes, the patch series is empty and the stage is a verification-only commit. If it fails, the patch series is the diff to make it pass.

**Verification.**

```sh
cargo check --target=riscv32imac-unknown-xous-elf -p libsignal-service --no-default-features --features=phonenumber
# Returns 0.

cargo tree --target=riscv32imac-unknown-xous-elf -p libsignal-service \
  | grep -E '(boring|openssl|aws-lc|bindgen|tokio|mio|reqwest)' \
  | wc -l
# Returns 0.

cargo tree --target=riscv32imac-unknown-xous-elf -p libsignal-protocol \
  | grep -E 'getrandom v[0-9]+' | sort -u
# Single line per major version; if both 0.2 and 0.3 appear, document why
# in the report (xous-core's `imports/getrandom` is 0.2; our cfg-custom
# extern handles 0.3 transitively).

# Standard checks.
cargo test -p presage-store-pddb        # 22 passed
cargo test -p xous-signal-bridge        # 3 passed
cargo run -p xous-app-signal --bin xas  # Stage 8 output unchanged
cargo clippy --workspace --all-targets -- -D warnings   # clean
cargo fmt --all -- --check              # clean
```

**Stop conditions.**
- Any C-surface dep appears in the rv32-target cargo tree. Surface; do not silently work around.
- `libcrux-ml-kem` fails to build for rv32 (the memo flagged this as the highest-risk unverified item). Fallback: replace `libcrux-ml-kem = ...` in `~/precursor-signal/repos/libsignal/rust/protocol/Cargo.toml`'s deps with `ml-kem = "0.2"` (RustCrypto, listed by cryptography.rs) and adjust `kem.rs` call sites. This is a libsignal patch and adds ~1 week; only do it if the build fails and surface first.
- The patch series exceeds 6 patches or any single patch touches > 50 lines of libsignal source. If so, escalate; the design assumption was "libsignal builds for rv32 with near-zero patches" and we're outside that band.

---

## Stage 7 — Fork `presage`: remove tokio dependence

**Goal.** Get `presage` to compile against the Stage 6 forked `libsignal-service-rs`, with the ~30-line tokio-removal patch from `REPORT.md` §Decision 2 applied.

**Prerequisites.** Stage 6 complete. Read `REPORT.md` §Decision 2 (the patch table) and the load-bearing comment at [`presage/src/manager/registered.rs:597-599`](https://github.com/whisperfish/presage/blob/main/presage/src/manager/registered.rs#L597-L599).

**Deliverables.**

- `vendor/presage/` containing upstream HEAD as path dep or submodule.
- Patch applying every line in the Decision 2 table:
  - `tokio::sync::Mutex` → `async_lock::Mutex` (1 line)
  - `tokio::task::spawn_local(...)` × 3 → `executor.spawn(...).detach()` (3 sites at registered.rs:696, 727, 746)
  - `tokio::spawn(...)` × 2 → `executor.spawn(...).detach()` (registered.rs:816, 1707)
  - `tokio::task::spawn_blocking(...)` → inline call (registered.rs:1255; single-threaded fine)
  - `tokio::time::error::Elapsed` → custom unit error in `errors.rs:65`
- Cargo.toml: remove `tokio`, add `async-executor`, `async-lock`, `futures-timer`.
- `Manager::link_secondary_device` and `Manager::register` and `Manager::load_registered` gain a new `executor: &'static LocalExecutor<'static>` parameter (so the spawn sites have something to spawn against). Document this API change in patch commit message.
- presage's existing tests pass against the patch.

**Verification.**

```
cargo test -p presage
# upstream tests pass with the patch applied
cargo tree -p presage | grep tokio
# Empty (no tokio in production deps)

cargo check --target=riscv32imac-unknown-xous-elf -p presage
# rv32 cross-compile of forked presage on top of forked libsignal-service-rs.

cargo tree --workspace -d
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

**Stop conditions.** If any spawn site can't be replaced with `LocalExecutor::spawn` (e.g. if a future genuinely requires Send because of some downstream type bound), surface — that's a structural issue not a search-and-replace.

---

## Stage 8 — Wire up `Manager` on a worker thread + IPC scaffolding

**Goal.** First end-to-end integration. A binary that, in hosted mode, instantiates `Manager::load_registered` against the Stage 5 PDDB-mock store, prints "no registration found" or whatever it returns, and exits cleanly. This proves the entire stack glues.

**Prerequisites.** Stages 5, 6, 7 complete.

**Deliverables.**

- `crates/xous-signal-bridge/src/`:
  - `lib.rs` — `pub fn run_signal_worker(store: PddbStore, cmd_rx: Receiver<Cmd>, event_tx: Sender<Event>) -> std::thread::JoinHandle<()>` spawns a thread, builds a `LocalExecutor`, runs `executor.run(async move { ... })` which loops on `cmd_rx`.
  - `cmd.rs` — initial `enum Cmd { Hello, GetWhoami }` and `enum Event { Pong, Whoami(...) }`.
- `crates/xous-app-signal/src/main.rs` is updated:
  - Construct `PddbStore` with the mock backend.
  - Spawn the worker thread via `run_signal_worker`.
  - In main thread: send `Cmd::Hello` over the channel; wait for `Event::Pong`; print; exit.

The worker handles `Hello` by replying immediately. It handles `GetWhoami` by trying to call `Manager::load_registered(...)` — if no registration data, returns an error; that's expected at this stage.

**Steps.**

1. In the worker:
   ```rust
   pub fn run_signal_worker(store: PddbStore, cmd_rx: Receiver<Cmd>, event_tx: Sender<Event>) -> JoinHandle<()> {
       std::thread::Builder::new()
           .name("signal-worker".into())
           .stack_size(4 * 1024 * 1024)  // start at 4 MB; raise if zkgroup demands
           .spawn(move || {
               let executor: &'static LocalExecutor<'static> = Box::leak(Box::new(LocalExecutor::new()));
               futures_lite::future::block_on(executor.run(async move {
                   while let Ok(cmd) = cmd_rx.recv().await {
                       match cmd {
                           Cmd::Hello => { let _ = event_tx.send(Event::Pong).await; }
                           Cmd::GetWhoami => { /* try Manager::load_registered, send back */ }
                       }
                   }
               }));
           }).unwrap()
   }
   ```
2. Drive from main: send Hello, await Pong, log success, exit.
3. Smoke test sending GetWhoami before any registration exists; should get an error event back. The error is expected; what we're testing is the channel round-trip.

**Verification.**

```
cargo run -p xous-app-signal --bin xas
# Expected output:
# worker started
# pong
# whoami: error (no registration data) -- expected
# exiting

cargo check --target=riscv32imac-unknown-xous-elf -p xous-app-signal
# Full rv32 cross-compile of the entire stack glue.
# This is the strongest pre-Stage-9 sanity check we have.

cargo tree --workspace -d
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

---

## Stage 9 — Hardware/Renode bring-up: Stage 8 binary on Precursor / Renode

**Goal.** First emulator/hardware test. The Stage 8 binary, with the real PDDB backend wired up, built for `riscv32imac-unknown-xous-elf`, bundled into a Xous image, and booted in Renode. Confirms toolchain, deps, build profile, image-bundling, and runtime environment all hold together.

**Integration choice.** Stage 9b's first attempt tried to merge the workspace into a xous-core fork as `apps/xas/`. That hit a hard architectural blocker (documented in `apps/xas/docs/INTEGRATION_STATUS.md` on `tunnell/xous-core/xas` branch, PR #24): xous-core's workspace-level `[patch.crates-io].aes` redirects the `aes` crate to `services/aes` (a Xous-IPC shim) which doesn't expose `Aes256Enc` that libsignal's `zkgroup` requires. Cargo doesn't support per-subtree patches. Two related conflicts (curve25519-dalek 4.1.2 vs 4.1.3, getrandom 0.2/0.3 + rkyv-uuid resolver cycle) stack on top.

**Revised integration choice.** The standalone workspace at `~/precursor-signal/xous-app-signal/` stays as the build root. A new `xtask` crate inside it cross-builds the rv32 binary and bundles it into a Xous image via xous-core's existing image-building tooling, *without* merging the workspace. The xous-core fork's `xas` branch keeps `apps/xas/` as a documentation subtree (the PR shows what we want to ship; the diff is human-readable).

This is option I from `apps/xas/docs/INTEGRATION_STATUS.md`. The user-supplied cryptography.rs analysis confirms it's the right choice: libsignal's protocol crates build pure-Rust on rv32 by themselves (no `aes` patch needed for our subgraph), so the merge was solving a problem we created.

**Prerequisites.** Stages 0–8 complete on hosted mode. Renode 1.16+ installed (`renode --version` — verified 1.16.1.4499 at this point). xous-core cloned (`~/precursor-signal/repos/xous-core/`). Optionally, Precursor hardware available.

### Stage 9a — Workspace-internal preparation

**Doable without the xous-core fork existing.** The `xous-app-signal` workspace stays where it is; we add the pieces that make it ready to drop into a xous-core-style tree.

**Deliverables.**

- `crates/xous-app-signal/src/getrandom_xous.rs` (or similar) — `__getrandom_v03_custom` extern matching the `--cfg getrandom_backend="custom"` setting from `.cargo/config.toml`. On Xous, calls `xous_api_trng::Trng::new(...).fill_buf(...)` (per `xous-core/api/xous-api-trng`); on hosted, defers to `getrandom_v02`. Without this the rv32 release build won't link (Stage 6.1 phase 3f follow-up).
- `crates/presage-store-pddb/src/backend_pddb.rs` (feature-gated by `pddb-backend`) — real `KvBackend` implementation against [`xous-core/services/pddb`](https://github.com/betrusted-io/xous-core/tree/main/services/pddb)'s native API. The crate's `Cargo.toml` adds `[features] pddb-backend = ["dep:pddb"]` with `pddb` as an optional dep targeted at `cfg(target_os = "xous")`. Hosted/test builds default to `mock-backend`.
- `xous-app-signal/docs/INTEGRATION.md` — concrete recipe for cloning the fork into `~/precursor-signal/repos/xous-core-for-xas/` and dropping our workspace in as `apps/xas/`. Spelled out so Stage 9b is mechanical.
- ROADMAP refinement: this split (9a/9b) instead of one monolithic Stage 9.

**Verification.**

```
# Hosted-mode still works (no behavioural change)
cargo run -p xous-app-signal --bin xas

# rv32 release build links (the strict Stage 9a target — without
# the getrandom 0.3 backend, the linker fails on
# `__getrandom_v03_custom undefined symbol`; with it, the binary
# links).
cargo build --target=riscv32imac-unknown-xous-elf --release -p xous-app-signal

# rv32 check still passes.
cargo check --target=riscv32imac-unknown-xous-elf -p xous-app-signal

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

### Stage 9b — xtask bundling + Renode boot

**Prerequisites.** Stage 9a complete. `tunnell/xous-core` cloned at `~/precursor-signal/repos/xous-core/` with `dev` / `dev-for-xas` / `xas` branches per `docs/SYNC.md`. Renode 1.16+ on PATH.

**Deliverables.**

- New `xtask/` crate at the standalone workspace root. Provides:
  - `cargo xtask build-rv32` — cross-builds the `xas` binary for `riscv32imac-unknown-xous-elf` with the `pddb-backend` feature.
  - `cargo xtask renode-image` — builds the rv32 binary, then drives `cargo xtask renode-image` (or equivalent) inside `~/precursor-signal/repos/xous-core/` with our binary injected. Two implementation paths to choose at execution time:
    * **path A — app-loader:** drop our binary into `~/precursor-signal/repos/xous-core/apps/<some-app>/` as a precompiled artifact, register a thin `apps/xas-loader/Cargo.toml` (matching `apps/app-loader`'s shape) that references the artifact, then run xous-core's xtask. Keeps xous-core's `[patch.crates-io]` from touching our crate graph.
    * **path B — Renode-direct load:** skip xous-core's workspace entirely. Build a Xous image with the standard set of services + kernel via xous-core's xtask, then in the Renode boot script (`xas-smoke.resc`) load our `xas` ELF directly into a known address and `start` from it. Cleaner separation; relies on Renode's `sysbus.LoadELF` semantics.
  - `cargo xtask renode-test xas-smoke.robot` — runs `renode-test` against our smoke script.
- `crates/presage-store-pddb/src/backend_pddb.rs` real impl behind `pddb-backend` feature flag. Forwards `KvBackend::{get,put,delete,delete_dict,list_keys}` to `pddb::Pddb` (path-dep on `~/precursor-signal/repos/xous-core/services/pddb/`). Single `Arc<Mutex<Pddb>>` per `PddbStore`; basis = `"signal"`.
- Real `__getrandom_v03_custom` body in `crates/xous-app-signal/src/main.rs`, replacing the Stage 9a `panic!`. Calls `trng::Trng::get_u64`/`get_u32` (path-deps on `~/precursor-signal/repos/xous-core/services/trng/` and `~/precursor-signal/repos/xous-core/api/xous-names/`).
- Logging shim: replace each `println!` in `xous-app-signal/src/main.rs` and the bridge with `log::info!`, with one-time logger init via `log_server::init_wait()`. UART output in Renode flows through xous-api-log.
- u32e backend re-enabled: `apps/xas/.cargo/config.toml`'s `--cfg curve25519_dalek_backend="u32e_backend"` line uncommented. The `utralib` Precursor SOC features wire up correctly when our binary is built against the path-dep'd xous-core tree.
- `tests/renode/xas-smoke.resc` — Renode boot script based on `~/precursor-signal/repos/xous-core/emulation/betrusted.resc`. Path-A flavor loads xous-core's full image; path-B flavor loads just the kernel+core services and our binary as an extra ELF.
- `tests/renode/xas-smoke.robot` — Robot Framework test asserting on the Stage 8 UART output:
  ```robot
  *** Test Cases ***
  Should boot and print pong
      Create Xas Machine
      Start Emulation
      Wait For Line On Uart    main loop                  testerId=1   timeout=30
      Wait For Line On Uart    xas: pong                  testerId=0   timeout=10
      Wait For Line On Uart    xas: exiting               testerId=0   timeout=15
  ```
- `tests/renode/run-renode-tests.sh` — wrapper. `cargo xtask renode-image && renode-test tests/renode/xas-smoke.robot`.

**Steps.**

1. Decide path A vs path B. Path B is simpler for the smoke test (less xous-core integration); path A is the canonical way to ship an app on Xous (eventually we want this for hardware deploy). Recommend B for Stage 9b's smoke test, plan A for Stage 13 (hardware-deploy stage, not currently in roadmap).
2. Implement `xtask/` with `build-rv32` first. Verify `cargo xtask build-rv32` produces a non-empty `xas` ELF at a known location.
3. Wire `pddb` / `trng` / `xous_names` path-deps. Implement `backend_pddb.rs` and the real `__getrandom_v03_custom` body.
4. Replace `println!` with `log::info!` + logger init.
5. Uncomment u32e cfg in `.cargo/config.toml`. Verify `cargo xtask build-rv32` still passes.
6. Write `xas-smoke.resc` modeled on betrusted.resc. Test that `renode -e "include @xas-smoke.resc; start"` boots and prints something.
7. Write `xas-smoke.robot` and `run-renode-tests.sh`. Run end-to-end.
8. On hardware (optional): `cargo xtask hardware-deploy` — if Precursor available, push the ELF over USB.
9. Once all of the above is green, `apps/xas/` on the `tunnell/xous-core/xas` branch is updated to mirror the standalone workspace's state — purely for the PR/diff (the build still happens out-of-tree).

**Verification.**

```sh
cd ~/precursor-signal/xous-app-signal

# Hosted-mode (carryover from Stage 8)
cargo run -p xous-app-signal --bin xas

# rv32 release build via the new xtask
cargo xtask build-rv32

# Binary size sanity
cargo bloat --target=riscv32imac-unknown-xous-elf --release --crates -p xous-app-signal --features=pddb-backend | head -20

# Image build + Renode boot test
cargo xtask renode-image
./tests/renode/run-renode-tests.sh xas-smoke.robot
# Expected: 1 test passed.

# Standard checks unchanged.
cargo test -p presage-store-pddb        # 22 passed
cargo test -p xous-signal-bridge        # 3 passed
cargo clippy --workspace --all-targets -- -D warnings   # clean
cargo fmt --all -- --check              # clean
```

**Stop conditions.**

- The `pddb` path-dep doesn't resolve. xous-core's `services/pddb` may have changed shape; fall back to opening the file and reading the public API.
- `__getrandom_v03_custom` panic still hit at runtime (means rv32 binary calls into getrandom 0.3 from a path that isn't routed). Trace via Renode's UART log; the call site is the actual bug.
- xous-core's xtask refuses to bundle our binary (path A) because our crate wasn't a real workspace member. Switch to path B.
- Renode boot hangs during PDDB-mount phase: the PDDB needs a one-time `pddb format` step on first boot. Check the Robot script's setup phase or document the prerequisite.
- If `cargo bloat` shows a single dep > 30% of the binary (excluding `xous-signal-bridge` and `presage`), it's a size-regression bug — surface.
- If the `xas` branch on `tunnell/xous-core` drifts from the standalone source-of-truth, the PR diff will become noisy. Update the branch as part of the Stage 9b commit, not on a delayed schedule.

---

## Stage 10 — MVP flow #1: Link as secondary device on Precursor

**Goal.** First user-facing flow. Run the app on Precursor, see it print a `tsurl://...` link, scan it with the Signal app on a phone, see "linking succeeded," persist the registration data, reboot, confirm the data persists. This is the LinkDevice flow from `CALL_GRAPH.md` §3.2.

**Prerequisites.** Stage 9 complete.

**Deliverables.**

- New IPC opcode `Cmd::LinkDevice { device_name: String }` and `Event::QrUrl(Url)`, `Event::LinkComplete(...)`, `Event::LinkError(...)`.
- Worker handler that calls [`Manager::link_secondary_device`](https://github.com/whisperfish/presage/blob/main/presage/src/manager/linking.rs#L63) with our store + executor.
- App displays the QR-encoded URL on the Precursor screen (use [`xous-core/libs/blitstr2`](https://github.com/betrusted-io/xous-core/tree/main/libs/blitstr2) or a similar text renderer).
- After the user scans, the app prints "linked" and persists registration data.
- After power-cycle, app launches in "registered" state (call `Manager::load_registered` first; if it returns successfully, skip the link flow).

**Steps.**

1. Add the IPC opcodes.
2. Worker handler (pseudocode):
   ```rust
   Cmd::LinkDevice { device_name } => {
       let (qr_tx, qr_rx) = futures::channel::oneshot::channel();
       executor.spawn(async move {
           let manager = Manager::link_secondary_device(
               store.clone(), SignalServers::Production, device_name, qr_tx, executor
           ).await;
           match manager {
               Ok(_) => event_tx.send(Event::LinkComplete).await,
               Err(e) => event_tx.send(Event::LinkError(e.to_string())).await,
           }
       }).detach();
       if let Ok(url) = qr_rx.await {
           event_tx.send(Event::QrUrl(url)).await;
       }
   }
   ```
3. Render QR. The simplest way: use the `qrcode` crate to produce a `Vec<Vec<bool>>` matrix, render as ASCII or via blitstr2 to the Xous framebuffer. Aim for a 33×33 module size (largest readable).
4. Verify the linking flow end-to-end on hardware.
5. Power-cycle; verify registration data persisted (the next launch finds an existing registration via `Manager::load_registered`).

**Verification.**

- App boots on Precursor (or Renode), displays QR on the framebuffer.
- Scanning with a Signal phone app linked the device successfully (visible in Signal phone's "linked devices" list).
- Power-cycle → app loads existing registration silently.
- PDDB inspection shows `signal.state.registration_data` key exists.

**Renode test.** Add `tests/renode/xas-link-mock.robot`:

```robot
*** Test Cases ***
Should display tsurl QR-encoding sentinel
    Create Xas Machine With Mocked Signal Server
    Start Emulation
    # Mocked Signal server returns a canned ProvisioningStep::Url(tsurl://test)
    Wait For Line On Uart    xas: link-device starting     testerId=0    timeout=20
    Wait For Line On Uart    xas: tsurl://test             testerId=0    timeout=30
    Wait For Line On Uart    xas: link complete            testerId=0    timeout=60
```

The mocked Signal server is a small Python or Rust HTTPS server that runs on the host alongside Renode and answers Signal's provisioning protocol with canned responses. This lets the link flow exercise real cipher and store paths without requiring a phone-in-the-loop. Keeps the Renode test deterministic.

**Stop conditions.** If linking fails with an HTTP error: most common cause is TLS root cert missing. Verify `webpki-roots` is in deps and the CA pinning matches what `libsignal-service-rs` expects. If linking succeeds but persistence fails: check that PDDB writes are syncing; try `Pddb::sync()` after each StateStore write. If the Renode mocked-server test fails but real-hardware works: the mock is wrong; trust hardware as ground truth.

---

## Stage 11 — MVP flow #2: Receive a single message

**Goal.** App displays one decrypted message from the Signal network on Precursor.

**Prerequisites.** Stage 10 complete (device is linked).

**Deliverables.**

- `Cmd::StartReceive` / `Event::Message(Content)` / `Event::ReceiveError(...)`.
- Worker handler that calls [`Manager::receive_messages`](https://github.com/whisperfish/presage/blob/main/presage/src/manager/registered.rs#L565) and forwards each `Received::Content` as an `Event::Message`.
- App displays the received message text on the Precursor screen.
- Per `REPORT.md` §Decision 5, after each batch of received messages (or on `Received::QueueEmpty`), the worker calls `store.flush_dirty_sessions()` to write accumulated SessionRecord changes to PDDB in batched chunks.

**Steps.**

1. Add `Cmd::StartReceive` and dispatch in the worker; spawn the receive task on the executor.
2. Per-message:
   - Pin and poll the stream inline (gurk-rs pattern, [`gurk-rs/src/main.rs:176-215`](https://github.com/boxdot/gurk-rs/blob/master/src/main.rs#L176-L215)).
   - On each `Received::Content(content)`, send `Event::Message(content)` to the IPC server.
   - On each `Received::QueueEmpty`, call `store.flush_dirty_sessions()`.
3. App UI: when an `Event::Message` arrives, render the body text on screen (just the latest message; full thread is later).
4. Test: send a Signal message from a phone to the linked Precursor. Verify it appears.

**Verification.**

- Hardware: send a 1:1 text from a paired phone. Within a few seconds, message appears on Precursor.
- Reboot the Precursor; verify the message persists in `signal.threads.<thread_id>` dictionary.
- Reconnect; verify the receive WS reconnects automatically.

**Renode test.** Extend the Stage 10 mocked Signal server with a "deliver one canned ciphertext envelope" capability, and add `tests/renode/xas-receive-mock.robot`:

```robot
*** Test Cases ***
Should decrypt and display one canned message
    Create Xas Machine With Mocked Signal Server (Pre-linked, with one queued envelope)
    Start Emulation
    Wait For Line On Uart    xas: linked, starting receive    testerId=0    timeout=30
    Wait For Line On Uart    xas: received: hello from mock   testerId=0    timeout=20
    # Verify QueueEmpty triggers a flush
    Wait For Line On Uart    xas: flushed N sessions          testerId=0    timeout=10
```

The canned envelope is constructed offline (from a known plaintext + a known recipient identity stored in a fixture PDDB) and queued for delivery in the mock server's WS pipe. This exercises the real Triple Ratchet decrypt path against a known-good ciphertext.

**Stop conditions.** If decrypt fails: it's almost certainly a session-store bug. Verify `SessionStore::store_session` is actually persisting on flush (call `Pddb::sync()` after the flush completes). If decrypt succeeds but display is empty: check that `Content::body` is being unwrapped to a `DataMessage::body` (which is a `String`). If the Renode test passes but hardware doesn't: timing issue — investigate WS keepalive interaction.

---

## Stage 12 — MVP flow #3: Send a single message

**Goal.** App sends a 1:1 text message from Precursor to a paired contact.

**Prerequisites.** Stage 11 complete.

**Deliverables.**

- `Cmd::SendMessage { recipient: ServiceId, body: String }` / `Event::SendComplete(timestamp)` / `Event::SendError(...)`.
- Worker handler that calls [`Manager::send_message`](https://github.com/whisperfish/presage/blob/main/presage/src/manager/registered.rs#L989).
- App input flow: user enters a recipient UUID and a message body (via Precursor keyboard); presses send; app dispatches the IPC.

**Steps.**

1. Add the opcode + handler.
2. Worker:
   ```rust
   Cmd::SendMessage { recipient, body } => {
       executor.spawn(async move {
           let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
           let content_body = ContentBody::DataMessage(DataMessage {
               body: Some(body), timestamp: Some(timestamp), ..Default::default()
           });
           let res = manager.send_message(recipient, content_body, timestamp).await;
           // forward result event
       }).detach();
   }
   ```
3. App side: collect input on the Precursor keyboard, build the `Cmd`, dispatch.
4. Test: send a message from Precursor to the paired phone. Verify it appears on the phone in the Signal app.

**Verification.**

- Send from Precursor → message visible on the paired phone within seconds.
- The sent message persists in `signal.threads.<thread_id>` dictionary on the Precursor side.
- If send is initiated when offline (no network), it should fail gracefully with an error event.

**Renode test.** `tests/renode/xas-send-mock.robot`:

```robot
*** Test Cases ***
Should encrypt and POST one message to the mocked /v1/messages endpoint
    Create Xas Machine With Mocked Signal Server (Pre-linked, with a known recipient session)
    Start Emulation
    Wait For Line On Uart    xas: ready                             testerId=0    timeout=30
    Inject IPC Cmd           SendMessage                            recipient=AAAAAAAA-...    body=hello-from-xas
    Wait For Line On Uart    xas: send complete (ts=...)            testerId=0    timeout=20
    Verify Mock Server Received    PUT /v1/messages/AAAAAAAA-... contains plausible ciphertext
```

The mock server validates that a request with the right shape arrived (correct recipient UUID, correct sender cert, plausible ciphertext bytes — though we can't decrypt without the recipient's private key, we can check the CiphertextMessage protobuf parses).

**Stop conditions.** If send fails with `MessageSenderError::NotFound`: the recipient device didn't have an active session, and the code didn't fall through to the prekey-bundle path. Check that `Manager::send_message` is being called with a fresh sender certificate. If the Renode mock receives the message but real Signal doesn't: TLS or routing issue — confirm endpoint pinning.

---

## After MVP — what to build next (out of scope for this roadmap)

Once Stages 0–12 are done, the MVP is complete. Future stages would add:

- **Multi-message thread display** with scrollback.
- **Group send/receive** (CALL_GRAPH §5.2 and §6.5).
- **Profile management** (RetrieveProfile, UpdateProfile).
- **Attachments** (image, voice, etc.; CALL_GRAPH §5.1 attachment subroutine).
- **Sticker pack support**.
- **Re-link UX** when registration is invalidated.
- **Keepalive on a separate thread** so PDDB write spikes don't break the WS keepalive (per the dev's earlier suggestion).
- **Real-hardware performance profiling** — actual session write latencies, RAM working-set under bursts, stack high-water marks. Inform the next round of write-strategy tuning.
- **CDSI** if/when boring-ssl gets a portable backend.

---

## Time estimates (rough, agent-driven)

| Stage | Hosted/Hardware | Agent-time estimate | Wallclock |
|---|---|---|---|
| 0 | hosted | 1 session | 30–60 min |
| 1 | hosted | 1 session | 30–60 min |
| 2 | hosted | 1 session | 1 hr |
| 3 | hosted | 1 session | 1 hr |
| 4 | hosted | 1 session | 2–3 hr |
| 5a | hosted | 1 session | 2–3 hr |
| 5b | hosted | 1 session | 1–2 hr |
| 5c | hosted | 1–2 sessions | 3–4 hr |
| 6 | hosted | 2–3 sessions | 4–6 hr |
| 7 | hosted | 1 session | 1–2 hr |
| 8 | hosted | 1 session | 1–2 hr |
| 9 | hardware | 1–2 sessions | 2–4 hr (heavy on dev iter) |
| 10 | hardware | 1–2 sessions | 3–5 hr |
| 11 | hardware | 1 session | 1–2 hr |
| 12 | hardware | 1 session | 1–2 hr |

**Total agent-time: ~25–40 hours of focused work**, spread across ~15 sessions. Stages 6 (libsignal-service-rs fork) and 10 (link UX) are the biggest unknowns; the rest are mostly mechanical given the design groundwork.

**Critical path**: 0 → 1 → 2 → 3 → 4 → 5 → (6 || 7) → 8 → 9 → 10 → 11 → 12.

**Parallelization opportunities**: 5a/5b/5c (three trait groups), 6/7 (two forks).

---

## Risks specific to this build-out (beyond the design risks in REPORT.md)

1. **rv32 toolchain mismatch.** The Xous Rust fork (`betrusted-io/rust`) might not support `edition = "2024"` (Rust 1.85+) yet. Stage 0 catches this.
2. **PDDB lock contention.** If multiple parts of the worker hold the `Pddb` handle across `.await` points, you'll deadlock. Convention: never hold `Pddb` across `.await`; always scope the handle in a small block.
3. **WS reconnection loops not implemented.** Stages 11 needs the gurk-rs Fibonacci-backoff reconnect pattern; if skipped, transient WiFi drops will brick the receive flow.
4. **Stack overflow under group-send batches.** Stage 9's 4 MiB initial stack guess might be wrong. zkgroup's endorsement code can be stack-hungry. Watch for this in Stage 12 (1:1 send) and especially in any future group-send work.
5. **PDDB heap cap (~2 MiB).** `list_keys` on a very busy `signal.threads.<thread>` dictionary will run out. Cap thread sizes or implement pagination at the store level. Not blocking for MVP, but flag in Stage 5c.
