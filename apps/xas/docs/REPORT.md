# Reusing the Whisperfish Signal Rust Stack on Xous — Design Decisions

A design document for Xous developers building a Signal client by reusing the upstream Rust crates that Whisperfish (Sailfish OS) and others use, rather than reimplementing the Signal protocol on top of `libsignal`. Audience: Rust developers with passing crypto familiarity. All claims cite source.

---

## 1. Project values and what this design optimizes for

The driving goal is **end-user verifiability**: the kind of user who buys a Precursor wants to be able to read every line of Rust that ends up on their device. That puts hard constraints on what we build:

1. **Maximize "herd immunity" — rely on upstream community-maintained code.** Each line of bespoke Xous-specific glue is a line that only this project's contributors will ever review. Each line in `whisperfish/presage` or `signalapp/libsignal` is reviewed by the wider Signal ecosystem.
2. **Minimize the audit surface for what we *do* add.** Vendor small, scrutable primitives in preference to large, complex ones. A 21-kLoC vendored async runtime is reviewable by one person in a long sitting; a 100-kLoC one is not.
3. **Keep our forks small and obvious.** When we patch upstream, the diff should be in the tens of lines, not the thousands, and it should be one commit on top of an upstream tag.
4. **Use what xous-core already vendored.** Crypto primitives (`ring`, `sha2`, `aes`, `curve25519-dalek`, `getrandom`) all have rv32-friendly forks at `[patch.crates-io]` in [xous-core's workspace `Cargo.toml`](https://github.com/betrusted-io/xous-core/blob/main/Cargo.toml#L164-L196). New deps that pull duplicate copies of these are wasted bytes and a verifiability regression.

The reuse target is the Whisperfish stack — three Rust crates, top-down:

```
   ┌─────────────────────────────────────────────────────────────────┐
   │  presage                  whisperfish/presage     AGPL-3.0      │  Manager: linking,
   │                                                                 │  registration, send/receive,
   │                                                                 │  contacts, groups, attachments
   ├─────────────────────────────────────────────────────────────────┤
   │  libsignal-service-rs     whisperfish/libsignal-service-rs      │  HTTP REST + WebSocket
   │                                                                 │  framing, push delivery,
   │                                                                 │  sealed sender, attachments
   ├─────────────────────────────────────────────────────────────────┤
   │  libsignal                signalapp/libsignal     AGPL-3.0      │  X3DH, Triple Ratchet
   │     (rust/protocol)                                             │  (Double Ratchet + SPQR),
   │                                                                 │  six storage traits,
   │                                                                 │  sealed sender, group
   │                                                                 │  sender keys
   └─────────────────────────────────────────────────────────────────┘
```

What we build on top of Xous to host the stack: **three small components plus a tightly-scoped fork of two upstream crates.**

---

## 2. The Xous-specific components we have to build

```
   ┌─────────────────────────────────────────────────────────────────┐
   │  Xous Signal app    UI server + IPC contract for chat clients   │
   ├─────────────────────────────────────────────────────────────────┤
   │  xous-presage-ipc        NEW    bridge between presage::Manager │
   │                                 (executor thread) and Xous IPC  │
   ├─────────────────────────────────────────────────────────────────┤
   │   presage  +  libsignal-service-rs  (light fork)                │
   │   libsignal/rust/protocol            (vendored, unmodified)     │
   │   smol-rs primitives (vendored)                                 │
   ├─────────────────────────────────────────────────────────────────┤
   │  presage-store-pddb     NEW    9 storage traits over PDDB       │
   │  xous-net-bridge        NEW    sync TLS+WS pump + channel       │
   │                                bridge to async world            │
   ├─────────────────────────────────────────────────────────────────┤
   │  Xous OS:  pddb  |  services/net  |  services/dns  |  libs/tls  │
   └─────────────────────────────────────────────────────────────────┘
```

Estimated custom code: presage-store-pddb ~1.5–2 kLoC, xous-net-bridge ~0.5–1 kLoC, xous-presage-ipc ~1.5 kLoC. Plus a tightly-scoped patch series against `whisperfish/libsignal-service-rs` and `whisperfish/presage` — see §5 and §10.

---

## Decision 1: persist state in PDDB; no `presage-store-cipher`

Implement the storage traits directly against PDDB, with the entire Signal app's data living in a named, password-locked Basis (e.g. `signal`).

**Why.** PDDB already provides AES-256-GCM-SIV per page ([`xous-core/services/pddb/src/backend/dictionary.rs:8`](https://github.com/betrusted-io/xous-core/blob/main/services/pddb/src/backend/dictionary.rs#L8); page sizing at [`backend/hw.rs:58-60`](https://github.com/betrusted-io/xous-core/blob/main/services/pddb/src/backend/hw.rs#L58-L60)) and Bases that are locked are computationally indistinguishable from free space. The [`presage-store-cipher`](https://github.com/whisperfish/presage/blob/main/presage-store-cipher) layer on top would be redundant CPU and would weaken deniability (its key has to live somewhere; the only sensible somewhere is also PDDB, so the second cipher protects nothing). [Xous Book ch. 9](https://github.com/betrusted-io/xous-book/blob/master/src/ch09-00-pddb-overview.md) is the threat model.

**Trait surface.** Nine async traits totaling ~50 methods.

Six libsignal traits, all `#[async_trait(?Send)]` — so the futures we return from our store don't need to be `Send`. Source: [`libsignal/rust/protocol/src/storage/traits.rs`](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/protocol/src/storage/traits.rs):

| Trait | Lines | Methods |
|---|---|---|
| `IdentityKeyStore` | [49-82](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/protocol/src/storage/traits.rs#L49-L82) | 5 |
| `PreKeyStore` | [85-95](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/protocol/src/storage/traits.rs#L85-L95) | 3 |
| `SignedPreKeyStore` | [98-112](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/protocol/src/storage/traits.rs#L98-L112) | 2 |
| `KyberPreKeyStore` | [117-140](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/protocol/src/storage/traits.rs#L117-L140) | 3 + last-resort dedup |
| `SessionStore` | [149-160](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/protocol/src/storage/traits.rs#L149-L160) | 2 |
| `SenderKeyStore` | [163-180](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/protocol/src/storage/traits.rs#L163-L180) | 2 |

Three more from libsignal-service-rs:

| Trait | File | Methods |
|---|---|---|
| `KyberPreKeyStoreExt` | [`pre_keys.rs:23-51`](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/pre_keys.rs#L23-L51) | 5 (last-resort prekey storage + stale-time bookkeeping) |
| `PreKeysStore` | [`pre_keys.rs:57-90`](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/pre_keys.rs#L57-L90) | 8 (next-ID counters, counts) |
| `SessionStoreExt` | [`session_store.rs:13-86`](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/session_store.rs#L13-L86) | 5 (bulk session deletion, safety-number computation) |

Plus presage's own [`Store` / `StateStore` / `ContentsStore`](https://github.com/whisperfish/presage/blob/main/presage/src/store.rs#L36-L377) — registration data, message-by-thread, contacts, groups, profiles, sticker packs.

**Dictionary layout.**

| Dictionary | Holds | Read/write rate |
|---|---|---|
| `signal.state` | Per-field keys: `registration_data`, ACI/PNI identity keypairs, master_key, sender_certificate | cold |
| `signal.protocol.{aci,pni}.session` | One key per `(uuid, device_id)` | **very hot** |
| `signal.protocol.{aci,pni}.identity` | One key per peer ProtocolAddress | warm |
| `signal.protocol.{aci,pni}.prekey_bundle` | **Single packed key** holding all current one-time EC prekeys | warm |
| `signal.protocol.{aci,pni}.signed_prekey` | One key per id | cold |
| `signal.protocol.{aci,pni}.kyber_prekey` | One key per id (~3 KB; ML-KEM-768) | cold |
| `signal.protocol.{aci,pni}.kyber_meta` | Last-resort dedup set + stale marks | warm |
| `signal.protocol.{aci,pni}.sender_key` | Per `(addr, dist_uuid)` | warm |
| `signal.contacts` | Per `ServiceId` | warm |
| `signal.groups` | Per master_key (32 B) | warm |
| `signal.profiles` | Per `ServiceId` | warm |
| `signal.threads.<thread_hex>` | One **dictionary** per thread; one key per message timestamp | very hot |
| `signal.attachments` | Per content-hash; LRU eviction | cold |

The packed-key strategy for one-time prekeys exists because each PDDB write costs at least one 4 KiB page (per-page AEAD; see [`backend/hw.rs:58-60`](https://github.com/betrusted-io/xous-core/blob/main/services/pddb/src/backend/hw.rs#L58-L60)). A naïve "one PDDB key per prekey record" approach would burn 100 pages for 100 × 70-byte records. Pack them.

**PDDB capabilities to design around** (verified against [`services/pddb/src/lib.rs`](https://github.com/betrusted-io/xous-core/blob/main/services/pddb/src/lib.rs)):

- `Pddb::get(...) -> Result<PddbKey>` ([line 532-541](https://github.com/betrusted-io/xous-core/blob/main/services/pddb/src/lib.rs#L532-L541)) — `PddbKey` implements `std::io::Read + Write + Seek`, so values stream rather than load whole `Vec<u8>`.
- `delete_dict(dict, basis)` ([line 737](https://github.com/betrusted-io/xous-core/blob/main/services/pddb/src/lib.rs#L737)) is **atomic** — the right primitive for `clear_messages(thread)` and `clear_all_sessions`.
- `lock_basis(name)` ([line 472](https://github.com/betrusted-io/xous-core/blob/main/services/pddb/src/lib.rs#L472)) is callable from any process holding a Pddb handle.
- `list_keys(dict, basis) -> Vec<String>` ([line 831](https://github.com/betrusted-io/xous-core/blob/main/services/pddb/src/lib.rs#L831)) returns a fully-allocated Vec — there is no streaming Iter and no prefix-filter on the wire. Don't call this on hot paths; cache thread-→-message-key indices in process memory.
- **No transactions, no compare-and-swap, no `key_exists` primitive.** Approximate `key_exists` via `get(..., create_key: false)` + match `ErrorKind::NotFound`. Hold a per-dictionary mutex inside the store for CAS-like behavior. Document a startup consistency check.
- Per-key max ~32 GiB (effectively `usize`-bounded to 4 GiB on Precursor); PDDB-server heap caps at ~2 MiB ([Xous Book ch. 9.1, lines 158-195](https://github.com/betrusted-io/xous-book/blob/master/src/ch09-01-basis.md#L158-L195)). Don't buffer multi-MB blobs.

**Storage record sizes** (revised against current libsignal):

- `IdentityKey`: 33 B + trust metadata
- `PreKeyRecord`: ~70 B
- `SignedPreKeyRecord`: ~140 B
- `KyberPreKeyRecord`: ~3 KB (ML-KEM-768: pk 1184 B + sk 2400 B)
- `SessionRecord`: 5–15 KB typical, hundreds of KiB worst-case under heavy out-of-order traffic. Bounded by [`MAX_MESSAGE_KEYS = 2000`, `MAX_RECEIVER_CHAINS = 5`, `ARCHIVED_STATES_MAX_LENGTH = 40`](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/protocol/src/consts.rs#L8-L12). Now includes `pq_ratchet_state: bytes` (~5–7 KB) since libsignal v0.91 adds the [SPQR (Sparse Post-Quantum Ratchet) state](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/protocol/src/proto/storage.proto#L66) to every session.
- `SenderKeyRecord`: ~1 KB per chain.

For 100 contacts and 10 groups: roughly **1 MB** total protocol-store on disk after SPQR.

---

## Decision 2: vendor smol-rs primitives, **not** Tokio

The Whisperfish stack uses async Rust, so we need an executor. We vendor [`smol-rs/async-executor`](https://github.com/smol-rs/async-executor), [`async-task`](https://github.com/smol-rs/async-task), [`async-channel`](https://github.com/smol-rs/async-channel), [`async-lock`](https://github.com/smol-rs/async-lock), [`event-listener`](https://github.com/smol-rs/event-listener), [`futures-lite`](https://github.com/smol-rs/futures-lite), and [`futures-timer`](https://github.com/async-rs/futures-timer) as the runtime. We do **not** add Tokio.

**Why.**

- Vendored smol-rs primitives total **11,613 Code LoC** (`tokei`), with most of the unsafe concentrated in `async-task`'s task vtable (50 unsafe lines, all `// SAFETY`-annotated). `async-executor` itself is 785 LoC and `#![forbid(unsafe_code)]` outside of a handful of `Send`/`Sync` marker impls. [`futures-lite`](https://github.com/smol-rs/futures-lite/blob/master/src/lib.rs#L38) and [`async-channel`](https://github.com/smol-rs/async-channel/blob/master/src/lib.rs#L30) are zero-unsafe. Plus required transitive deps (`concurrent-queue`, `crossbeam-utils`, `event-listener-strategy`, `parking`, `fastrand`, `slab`, `pin-project-lite`, `futures-core`, `futures-io`) bring the total to roughly **~21 kLoC** of new vendored async surface.
- For comparison, **Tokio is well over 100 kLoC** of Rust source. ~5× larger audit surface for capability we don't need (work-stealing multi-threaded scheduler, mio-based I/O reactor, 50+ feature flags).
- [`async-executor::LocalExecutor::spawn`](https://github.com/smol-rs/async-executor/blob/master/src/lib.rs#L510) accepts non-`Send` futures with `'a` lifetime — covers both the `tokio::task::spawn_local` and `tokio::spawn` use cases on our single-threaded worker thread, because Send is irrelevant on a single thread.
- [`futures-timer::Delay`](https://github.com/async-rs/futures-timer/blob/master/src/native/delay.rs) does not require an ambient runtime context to construct — unlike `tokio::time::Sleep`, which panics on construction outside a Tokio runtime. The futures-timer reactor is one global helper thread using `std::thread::park_timeout`; works on Xous's libstd.
- Our worker thread is single-threaded by design (the storage layer is `?Send`; presage's [load-bearing comment at registered.rs:597-599](https://github.com/whisperfish/presage/blob/main/presage/src/manager/registered.rs#L597-L599) explicitly says "we can't do the classic `tokio::spawn` with a `oneshot::channel()` or `CancellationToken` because of `!Send` constraints in the Store"). `LocalExecutor` is exactly the right shape.

**Where presage and libsignal-service-rs use Tokio today.** Each call site is replaced by a single line of smol-equivalent code. The full diff is roughly **~30 lines across both crates**.

`presage/presage/src/manager/registered.rs`:

| Line | Current | Replacement |
|---|---|---|
| [47](https://github.com/whisperfish/presage/blob/main/presage/src/manager/registered.rs#L47) | `use tokio::sync::Mutex;` | `use async_lock::Mutex;` |
| [696](https://github.com/whisperfish/presage/blob/main/presage/src/manager/registered.rs#L696), [727](https://github.com/whisperfish/presage/blob/main/presage/src/manager/registered.rs#L727), [746](https://github.com/whisperfish/presage/blob/main/presage/src/manager/registered.rs#L746) | `tokio::task::spawn_local(...)` | `executor.spawn(...).detach()` |
| [816](https://github.com/whisperfish/presage/blob/main/presage/src/manager/registered.rs#L816), [1707](https://github.com/whisperfish/presage/blob/main/presage/src/manager/registered.rs#L1707) | `tokio::spawn(...)` | `executor.spawn(...).detach()` |
| [1255](https://github.com/whisperfish/presage/blob/main/presage/src/manager/registered.rs#L1255) | `tokio::task::spawn_blocking(decrypt_in_place)` | inline call (single-threaded; no other task to starve) |

`presage/presage/src/errors.rs`:

| Line | Current | Replacement |
|---|---|---|
| [65](https://github.com/whisperfish/presage/blob/main/presage/src/errors.rs#L65) | `Timeout(#[from] tokio::time::error::Elapsed)` | `Timeout` (custom unit error) |

`presage/presage/Cargo.toml`:

| Line | Current | Replacement |
|---|---|---|
| [27-31](https://github.com/whisperfish/presage/blob/main/presage/Cargo.toml#L27-L31) | `tokio = { features = ["rt", "sync", "time"] }` | remove `tokio`; add `async-executor`, `async-lock`, `futures-timer` |

`libsignal-service-rs/src/push_service/mod.rs`:

| Line | Current | Replacement |
|---|---|---|
| [184](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/push_service/mod.rs#L184) | `tokio::task::spawn(task);` | return `task` to caller; caller spawns on their executor |

`libsignal-service-rs/src/websocket/mod.rs`:

| Line | Current | Replacement |
|---|---|---|
| [15](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/websocket/mod.rs#L15) | `use tokio::time::Instant;` | `use std::time::Instant;` |
| [223](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/websocket/mod.rs#L223) | `tokio::time::interval_at(now, KEEPALIVE_TIMEOUT)` | `futures-timer::Delay`-driven loop in the existing `futures::select!` |

`libsignal-service-rs/Cargo.toml` line [50](https://github.com/whisperfish/libsignal-service-rs/blob/main/Cargo.toml#L50): drop `tokio = { version = "1.0", features = ["macros"] }` (the `macros` feature is for `#[tokio::test]` in dev-deps; kept there).

The `libsignal/rust/protocol` crate is already tokio-free (zero `tokio::` references in `src/`, no tokio Cargo dep) so no patches there.

**Worker thread layout** (the [`purple-presage`](https://github.com/hoehermann/purple-presage) and [`gurk-rs`](https://github.com/boxdot/gurk-rs) embedding pattern, adapted):

```rust
// On Xous app startup, in a dedicated Xous thread:
static EXECUTOR: LazyLock<LocalExecutor<'static>> = LazyLock::new(LocalExecutor::new);

let (cmd_tx, cmd_rx) = async_channel::bounded::<Cmd>(32);
let (event_tx, event_rx) = async_channel::bounded::<Event>(32);

futures_lite::future::block_on(EXECUTOR.run(async move {
    let manager = presage::Manager::load_registered(store, &EXECUTOR).await?;

    EXECUTOR.spawn(run_receive(manager.clone(), event_tx.clone())).detach();
    run_command_loop(manager, cmd_rx, event_tx).await;
}));
```

The `LocalExecutor::run(future)` drives the executor and the given future to completion on the current thread; spawned tasks share the same thread. The `&EXECUTOR` reference is what presage's spawn sites need (signature change to presage: thread the executor through `Manager::new`).

`presage::Manager` requires the `LocalExecutor` to be passed in (since it spawns work). The fork patch adds an `executor: &'static LocalExecutor<'static>` parameter to `Manager::new` / `Manager::load_registered` / `Manager::link_secondary_device`.

---

## Decision 3: replace the network transport with a sync worker thread bridged by a channel

A second Xous thread (separate from the executor thread) holds a sync `tungstenite::WebSocket<rustls::StreamOwned<ClientConnection, TcpStream>>` and pumps frames over an `async-channel` to the executor thread. Fork `libsignal-service-rs` to swap `reqwest_websocket::WebSocket` for a channel-backed `Stream<Item = Result<Frame>>`, and to swap `reqwest::Client` for a sync HTTP/1.1 client (`ureq`) over the same TLS+TCP stack.

**Why.** What `libsignal-service-rs` uses now ([`websocket/mod.rs:14`](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/websocket/mod.rs#L14)):

```rust
use reqwest_websocket::WebSocket;
```

`reqwest-websocket` is async + tokio-coupled (transitively via reqwest → hyper). The HTTP client at [`push_service/mod.rs:90-103`](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/push_service/mod.rs#L90-L103) is `reqwest::Client` with `.http1_only()` — HTTP/1.1 only, good — but still tokio-internal.

What Xous provides:

- Blocking `TcpStream` with `set_nonblocking` support ([`xous-core/services/net/src/std_tcpstream.rs:136, 202`](https://github.com/betrusted-io/xous-core/blob/main/services/net/src/std_tcpstream.rs#L136)). DNS via [`services/dns/src/hw.rs:26`](https://github.com/betrusted-io/xous-core/blob/main/services/dns/src/hw.rs#L26).
- `rustls 0.22.2` already integrated and exposed as `Tls::stream_owned(host, sock)` returning `rustls::StreamOwned<ClientConnection, TcpStream>` ([`xous-core/libs/tls/src/lib.rs:446-463`](https://github.com/betrusted-io/xous-core/blob/main/libs/tls/src/lib.rs#L446-L463)).
- Hardware RTC.
- No async runtime, no `tokio::net::*`, no native TLS-async stack.

A sync `tungstenite::WebSocket` frames cleanly over the rustls + Xous TcpStream pair. `async-channel`'s `recv_blocking()` lets the sync thread do a synchronous read while the async thread does `recv().await` — same channel object; the runtime mismatch lives in the channel and nowhere else:

```
   Xous TCP thread (sync, blocking)         Executor thread (async)
   ───────────────────────────────          ────────────────────────────
   loop {                                                Manager::receive_messages()
     frame = ws.read_message()?;                                   |
     in_tx.send_blocking(frame);     ──[in_tx/in_rx]──►   incoming.next().await
                                                                   |
     while let Ok(req) = out_rx.try_recv()                         |
       { ws.write_message(req)? }    ◄─[out_tx/out_rx]── sent via Manager
   }
```

**Why fork `libsignal-service-rs` rather than inject a `reqwest`-compatible backend.** `reqwest`'s API surface is large (hyper + h2 + connection pool + `hyper-util::TokioExecutor`). Replacing its backend means re-implementing reqwest's surface on top of our sync pump — wrong layer. The fork swaps the WS module + a small set of HTTP request paths to take a transport trait, swap in our sync pump. Diff target: ~2 kLoC, kept on a feature branch, rebased on libsignal-service-rs releases.

**HTTP client side.** [`ureq`](https://github.com/algesten/ureq) does sync HTTP/1.1 with rustls and integrates with our `rustls::StreamOwned<ClientConnection, TcpStream>`. Use it for the few REST endpoints presage hits (provisioning, prekey upload, profile, attachment CDN). The WebSocket is the primary message-delivery transport.

**HTTP/2 not required today.** libsignal-service-rs declares `.http1_only()` ([line 101](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/push_service/mod.rs#L101)), and libsignal v0.91's production endpoint configuration is HTTP/1.1 across the board ([`rust/net/src/env.rs:57, 131, 167, 261`](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/net/src/env.rs#L57)). Only the experimental `grpc.chat.signal.org` endpoint declares HTTP/2, gated behind `experimental_chat_h2_domain_config`.

**TLS root certificate.** libsignal-service-rs pins its own root CA in the reqwest builder ([line 92-96](https://github.com/whisperfish/libsignal-service-rs/blob/main/src/push_service/mod.rs#L92-L96)) and disables built-in roots. Carry that pinning forward to the ureq + rustls path; the certificate bytes are in [`libsignal-service-rs/certs/`](https://github.com/whisperfish/libsignal-service-rs/tree/main/certs).

---

## Decision 4: Manager owns nothing platform-specific; the IPC server forwards

`presage::Manager` lives entirely on the executor thread. The Xous IPC server thread (the thing chat UI talks to) is a forwarder: it receives Cmd opcodes from clients, pushes them onto an `async-channel` to the executor thread, and listens on a separate channel for events, which it dispatches via Xous IPC sends to subscribers.

This is the structure both [`gurk-rs`](https://github.com/boxdot/gurk-rs/blob/master/src/main.rs#L176-L215) and [`purple-presage`](https://github.com/hoehermann/purple-presage/blob/main/src/c/bridge.c#L150-L183) independently arrived at. presage's API is async + `!Send`; the Xous IPC server's API is sync + message-passing. The two cannot share a scheduler — each owns its event loop, communicating via queues:

```
   Chat UI (Xous client process) ──IPC──► Xous IPC Server thread
                                                  │
                                                  │  cmd_tx.send_blocking(cmd)
                                                  ▼
                                          ┌─────────────┐
                                          │ async-      │
                                          │ channel     │
                                          └──────┬──────┘
                                                 │  cmd_rx.recv().await
                                                 ▼
                                       smol::LocalExecutor (executor thread)
                                                 │
                                                 ├── presage::Manager
                                                 ├── receive_messages stream
                                                 ├── command-loop dispatcher
                                                 │
                                                 │  event_tx.send(event).await
                                          ┌──────▼──────┐
                                          │ async-      │
                                          │ channel     │
                                          └──────┬──────┘
                                                 │  event_rx.recv_blocking()
                                                 ▼
                                          Xous IPC Server thread ──IPC──► Chat UI
```

`async-channel` supports both sync (`send_blocking` / `recv_blocking`) and async (`send().await` / `recv().await`) ends on the same channel object.

**Reconnection.** The receive-stream-end + Fibonacci backoff loop ([`gurk-rs/src/main.rs:176-215`](https://github.com/boxdot/gurk-rs/blob/master/src/main.rs#L176-L215), [`gurk-rs/src/backoff.rs`](https://github.com/boxdot/gurk-rs/blob/master/src/backoff.rs)) belongs **inside the executor thread**, not in the IPC layer. The IPC server should never see "stream ended" as a state — it sees only fresh events and a terminal "manager unrecoverable" event (the UI handles that by surfacing re-link UX).

---

## Decision 5: PDDB write strategy — accumulate in RAM, flush in 4 KiB chunks

The receive hot path is the one that writes most: every received message advances the ratchet (`SessionStore::store_session`), and `SessionRecord` is 5–15 KB typical. A naive write-through approach issues one PDDB write per ratchet step.

Per Xous-side guidance: **PDDB performance is best when many small writes are combined into single accesses up to ~4 KiB** (the page size). Tens of byte-level writes scattered over the same key are far slower than one batched write. So the design: **accumulate dirty SessionRecords in RAM, flush them on a debounce timer or at quiescent points** (every `Received::QueueEmpty` from the message stream, every N seconds, or on `MAX_BATCH=32` advances per session).

```
   message arrives
        │
        ▼
   decrypt → ratchet advances → SessionRecord modified in RAM
        │
        ▼
   dirty_set.insert(addr)        ← in-memory, no PDDB write yet
        │
        ▼
   on timer tick OR Received::QueueEmpty OR session count > N:
     for addr in dirty_set:
       pddb.put("signal.protocol.aci.session", addr, &record)   ← batched flush
     dirty_set.clear();
```

**Trade-off.** A power loss between ratchet advance and flush leaves the session state slightly behind the peer's view. libsignal handles divergence by re-keying when sends fail — visible to the user as a fresh safety-number prompt. Acceptable trade-off for the latency win, and aligned with what the Xous community considers reasonable. (The alternative — write-through — burns one ~3-page PDDB write per received message and is too slow for offline-message-burst catch-up.)

**Empirical alignment.** Before fixing the dictionary layout for the very-hot dictionaries, observe what presage actually writes against its sqlite backend in CLI-driven receive traces. We expect the access pattern to be: many `update sessions set …` for the same primary key in quick succession during a burst, sparse rewrites during steady state. If the observed pattern matches, our packed-key + RAM-debounce design is right. If not, revise.

**Per-archived-state subkey** is a future optimization if profiling shows the SessionRecord rewrite is still a bottleneck even with debouncing. Split `SessionRecord` into `current` (~5 KB, hot) and `archived` (~tens of KB, cold) PDDB keys; reassemble on read. No libsignal API change required — handled entirely in our store impl.

**Avoid `presage-store-cipher`** on top of the RAM accumulator — same reason as Decision 1 (PDDB already provides AEAD).

---

## Decision 6: lean on xous-core's `[patch.crates-io]` for crypto deps

xous-core's workspace [`Cargo.toml`](https://github.com/betrusted-io/xous-core/blob/main/Cargo.toml#L164-L196) already vendors rv32-friendly forks of crypto crates that the Whisperfish stack transitively depends on:

| Crate | xous-core fork | Effect |
|---|---|---|
| `sha2` | [`betrusted-io/hashes` branch `sha2-v0.10.8-xous`](https://github.com/betrusted-io/hashes/blob/sha2-v0.10.8-xous/sha2/Cargo.toml) | Gates `sha2-asm` to x86/x86_64/aarch64 only — RISC-V skips it. **Solves the SPQR `sha2/asm` build issue without patching SPQR.** |
| `aes` | local path `services/aes` | Hardware-accelerated AES on Precursor's hardware engine. |
| `ring` | [`betrusted-io/ring-xous`](https://github.com/betrusted-io/ring-xous) | rv32-buildable replacement for upstream ring. Required by rustls. |
| `getrandom` | local path `imports/getrandom` | Routes to Precursor's TRNG. |

When we add presage to a workspace that includes these patches, the patches apply to the whole transitive tree — including SPQR. That's why we don't need to fork SPQR.

**`curve25519-dalek` strategy: vendor `betrusted-io/curve25519-dalek` (HW-accelerated for Precursor) with a version bump and the lizard-module port.** Bunnie confirmed (2026-05) the curve25519 IP core is on **Precursor only** — not on the Bao1x tape-out. We're targeting Precursor first; Bao1x support is a future swap (different PKE engine; different backend would need to be written).

The vendored copy at `vendor/curve25519-dalek/` is **`betrusted-io/curve25519-dalek`** with three small modifications:

1. Manifest version bumped from `4.1.2` → `4.1.3` so the `[patch.crates-io]` redirect matches what libsignal's `zkgroup` declares (`curve25519-dalek = "4.1.3"`).
2. The `src/lizard/` module ported verbatim from `signalapp/curve25519-dalek` (`signal-curve25519-4.1.3` tag) — 4 `RistrettoPoint` methods used by zkgroup (`lizard_encode<H>`, `lizard_decode<H>`, `from_uniform_bytes_single_elligator`, `decode_253_bits`). Additive vs the betrusted-io fork — no API conflicts.
3. One `pub mod lizard;` line in `src/lib.rs`.

That's the entire delta over the betrusted-io fork. The fork itself is upstream curve25519-dalek 4.1.2 + the u32e HW-accelerator backend at [`curve25519-dalek/src/backend/serial/u32e/`](https://github.com/betrusted-io/curve25519-dalek/tree/main/curve25519-dalek/src/backend/serial/u32e).

**Activation.** The u32e backend is selected at compile time by `--cfg curve25519_dalek_backend="u32e_backend"`. We auto-set this for the rv32-xous target via `.cargo/config.toml`:

```toml
[target.riscv32imac-unknown-xous-elf]
rustflags = ["--cfg", "curve25519_dalek_backend=\"u32e_backend\""]
```

On hosted Linux the same code falls back to the portable Rust backend, so tests/CI run unaffected. On Precursor hardware, ECC operations route through the IP core.

**Future-target story.** The choice is target-scoped, not workspace-scoped. Adding Bao1x support later is a matter of writing a new backend module (e.g. `src/backend/serial/bao1x_pke/`) and adding another `[target.…]` block in `.cargo/config.toml` to activate it. The Precursor decision doesn't lock us out.

The vendored fork is patched in via:

```toml
[patch.crates-io.curve25519-dalek]
path = "vendor/curve25519-dalek/curve25519-dalek"

[patch.crates-io.curve25519-dalek-derive]
path = "vendor/curve25519-dalek/curve25519-dalek-derive"

# libsignal also imports curve25519-dalek directly via the git URL alias
# `curve25519-dalek-signal = { git = "...signalapp/...", package = "curve25519-dalek" }`
# at libsignal/Cargo.toml:90. [patch.crates-io] doesn't redirect git sources,
# so we additionally patch the git URL.
[patch."https://github.com/signalapp/curve25519-dalek"]
curve25519-dalek = { path = "vendor/curve25519-dalek/curve25519-dalek" }
curve25519-dalek-derive = { path = "vendor/curve25519-dalek/curve25519-dalek-derive" }
```

This supersedes both the (incorrect) earlier reference to `tunnell/curve25519-dalek` in xous-core's `Cargo.toml:173-176` and the temporary use of `signalapp/curve25519-dalek`. xous-core's existing patch can be updated to point at the same vendored crate when this workspace merges in (Stage 9 follow-up).

---

## Decision 7: stay a separate workspace; bundle the binary into Xous images via xtask

**Choice.** `xous-app-signal/` lives as its own Cargo workspace at `~/precursor-signal/xous-app-signal/`. Its `[patch.crates-io]` is independent of xous-core's. To run on Xous, we *bundle* the cross-compiled rv32 ELF into a Xous image — we do not merge the workspace into a xous-core fork.

**Why not merge.** Stage 9b's first attempt tried to drop the workspace into a xous-core fork as `apps/xas/`. Three of xous-core's workspace-level `[patch.crates-io]` entries are incompatible with the Whisperfish stack's deps:

1. **`[patch.crates-io.aes] path = "services/aes"`** — xous-core's `services/aes` is a Xous-IPC shim that does not expose `Aes256Enc`. libsignal's [`zkgroup::api::profiles::profile_key.rs:74`](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/zkgroup/src/api/profiles/profile_key.rs#L74) calls `::aes::Aes256Enc::new` directly. Cargo applies workspace `[patch]` to all members; there is no per-subtree opt-out.
2. **`[patch.crates-io.curve25519-dalek] git = "...betrusted-io/...main"`** — at version 4.1.2; libsignal's `zkgroup` declares `curve25519-dalek = "4.1.3"` and uses methods from the lizard module. Replacing the patch with our vendored 4.1.3 would force `=4.1.2 → =4.1.3` rebumps in `services/{root-keys,shellchat}` (workable but cross-cutting).
3. **`[patch.crates-io.getrandom] path = "imports/getrandom"`** — getrandom 0.2 only. The Signal stack's modern `rand` paths pull getrandom 0.3, which we route via `--cfg getrandom_backend="custom"`. The combination triggers a resolver cycle through `imports/getrandom 0.2 → uuid 1.x → rkyv 0.8 → imports/getrandom 0.2` when uuid's `rng` feature is unified-on by anything in the new graph.

These conflicts are detailed in `apps/xas/docs/INTEGRATION_STATUS.md` on the `tunnell/xous-core/xas` branch. None of them are libsignal bugs or xous-core bugs — they're a structural mismatch between Xous's "every crypto crate gets replaced by a IPC shim" approach and Signal's "every crypto crate is RustCrypto-style upstream" approach.

**The xtask path avoids all three** because xous-core's `[patch.crates-io]` only applies inside xous-core's workspace. Our standalone workspace builds against unmodified upstream `aes 0.8.4` (RustCrypto), our vendored `curve25519-dalek 4.1.3`, and our `getrandom 0.3 + custom backend` — no conflict because xous-core's services live in a separate dep graph.

**The cost** is bundling complexity at Stage 9b. The `xtask` crate has to:

1. Cross-build the `xas` binary for rv32 with the `pddb-backend` feature.
2. Either inject the binary into xous-core's image build (the "app-loader" path) or load it directly via Renode's `sysbus.LoadELF` (the "Renode-direct" path).
3. Path-dep into xous-core's `services/{pddb,trng}` and `api/xous-names` for runtime client APIs (these are pure-Rust client libraries; the IPC shims they replace at the patch level live elsewhere).

**Why this matches the cryptography.rs analysis.** The user-supplied evaluation memo (`RESUME.md`, full text in conversation history) confirms libsignal v0.90+ uses pure-Rust crypto throughout — `aes 0.8`, `aes-gcm-siv 0.11`, `ctr 0.9`, `hkdf 0.12`, `hmac 0.12`, `sha2 0.10`, `subtle 2.6`, `libcrux-ml-kem 0.0.8`, `spqr` v1.5.1. Every one of those crates is pure-Rust on rv32. The cryptography.rs catalog at `https://cryptography.rs/` lists the upstream RustCrypto versions of the same primitives (with the one exception that cryptography.rs lists `ml-kem` rather than `libcrux-ml-kem`; both are pure-Rust). xous-core's `[patch.crates-io].aes` substitution is a *Xous-internal* optimization (route AES through Precursor's hardware engine via IPC); it doesn't apply to the Signal stack and we don't need it. **Staying out of xous-core's workspace is the natural alignment with cryptography.rs's curated picks.**

**Future-merge story.** If a later stage (post-MVP, hardware-deployed) wants to upstream `apps/xas/` into `betrusted-io/xous-core`, the sticking point is `services/aes` API surface — xous-core's services would need to add upstream-API-compatible wrappers (`Aes256Enc`, `Aes256Dec`, `BlockCipher`/`BlockEncrypt`/`BlockDecrypt` trait conformance) so that crates expecting RustCrypto-shaped `aes` can drop in. This is the option-II work from `INTEGRATION_STATUS.md`. It's xous-core surgery, not xas-internal work, and it's deferred.

---

## Binary size strategy

xous-core's release profile is at [`Cargo.toml:154-161`](https://github.com/betrusted-io/xous-core/blob/main/Cargo.toml#L154-L161):

```toml
[profile.release]
codegen-units = 1
debug = true       # DWARF kept for hardware crash debugging
strip = false
lto = "fat"
incremental = true
opt-level = "s"   # optimize for size
```

The `debug = true` + `strip = false` setting keeps DWARF in the on-disk binary for crash analysis. **For binary-size measurements**, build a parallel profile with `debug = false` + `strip = true` and compare — DWARF can dominate the on-disk size. This is the right way to evaluate whether adding presage + libsignal-service-rs is acceptable.

Hygiene checklist before merging any new dep:

- Run `cargo tree -e features` from a build target that mirrors the final config. Ensure no two copies of any crate (very easy to accidentally pull two `sha2` versions through different intermediate deps; `cargo tree -d` flags duplicates).
- Cross-reference against xous-core's `[patch.crates-io]` to make sure transitive deps that have rv32 forks are using those forks.
- Build `--release` and inspect with `cargo bloat --release --crates` to find the top 20 size contributors. Investigate any that look unexpected.
- Decide LTO scope: `fat` (whole-program; xous-core uses) is the right default for size; `thin` is faster at link time but produces larger binaries.

`opt-level = "s"` is right for size; `opt-level = "z"` is even smaller but pays a measurable runtime penalty on hot paths (the receive loop in particular). Default to `"s"` and measure.

---

## Build patches required

### 1. `libsignal-service-rs` transport patch (Decision 3)

Fork the WebSocket module and HTTP request paths to swap reqwest_websocket → channel-backed Stream and reqwest → ureq. Diff target: ~2 kLoC, single feature branch, rebased per upstream release. Track [`whisperfish/libsignal-service-rs`](https://github.com/whisperfish/libsignal-service-rs) main.

### 2. `presage` + `libsignal-service-rs` Tokio-removal patch (Decision 2)

The diff per the table in Decision 2 — roughly 30 lines across the two crates. Threads a `&LocalExecutor` parameter through `Manager` constructors. Single feature branch.

### 3. Disable `cdsi` feature in libsignal-service-rs

[`libsignal-service-rs/Cargo.toml:60`](https://github.com/whisperfish/libsignal-service-rs/blob/main/Cargo.toml#L60) declares `default = ["cdsi"]`. The cdsi feature pulls `libsignal-net` and `libsignal-net-infra`, which transitively pull `boring-signal` (Signal's BoringSSL fork — vendored C/C++) which doesn't target rv32. Build with `--no-default-features` and select only the features wanted (`phonenumber`).

This means **no Contact Discovery Service Intersection** — the feature that helps Signal users find which contacts in their phonebook are also on Signal. Acceptable trade-off; revisit if/when CDSI's transport stabilizes on a more portable backend.

### 4. Disable `rayon` in zkgroup

[`libsignal/rust/zkcredential`](https://github.com/signalapp/libsignal/tree/v0.91.0/rust/zkcredential) and [`libsignal/rust/zkgroup`](https://github.com/signalapp/libsignal/tree/v0.91.0/rust/zkgroup) use `rayon` for parallelizing endorsement batches. rayon needs `std::thread::spawn`; Xous's threading model doesn't match. Build with `default-features = false` to skip the `rayon` feature; serial path is the fallback.

### 5. SPQR Cargo.toml — already handled by xous-core

[SPQR `Cargo.toml:51`](https://github.com/signalapp/SparsePostQuantumRatchet/blob/v1.5.1/Cargo.toml#L51) requests `sha2 = { features = ["asm"] }` for non-Windows non-x86 targets, which on a vanilla rv32 target would fail. **Already solved by inheriting xous-core's `[patch.crates-io].sha2`** which gates the asm feature to x86/aarch64 only. No SPQR fork required.

---

## Toolchain requirements

- **Rust 1.85 or newer** for the Xous target. Required because [`libsignal/rust/protocol/Cargo.toml`](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/protocol/Cargo.toml#L11-L12) and [`libsignal/rust/zkgroup/Cargo.toml`](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/zkgroup/Cargo.toml) declare `edition = "2024"` + `rust-version = "1.85"`.
- libsignal pins `rust-toolchain = "nightly-2026-03-23"` for its own builds; downstream consumers can use stable 1.85+.
- Verify the [`betrusted-io/rust`](https://github.com/betrusted-io/rust) Xous toolchain fork tracks at least 1.85. xous-core has no pinned `rust-toolchain.toml` at the workspace level, so this varies per developer install — check before starting.

---

## Companion artifact: libsignal-service-rs ↔ libsignal call graph

Separate from this design report, a planned deliverable is **a complete diagram showing how `libsignal-service-rs` uses `libsignal/rust/protocol` to implement each subcommand exposed by `presage-cli`**. The diagram will live in a separate Markdown file, use mermaid sequence diagrams (one per subcommand), and cite source line numbers throughout. Subcommands enumerated from [`presage-cli/src/main.rs`](https://github.com/whisperfish/presage/blob/main/presage-cli/src/main.rs): `link`, `register`, `whoami`, `send`, `send-to-group`, `receive`, `list-contacts`, `list-groups`, `request-contacts-sync`, `request-keys-sync`, `unregister`, etc.

This is the right way to load the protocol back into context for code review. It also makes the project's design defensible to a reader who knows Rust + intro-level cryptography but hasn't read the Signal spec.

---

## Risks (in priority order)

1. **PDDB write throughput under bursts.** A 200-message offline-catch-up burst is ~200 SessionRecord writes × ~12 KB. Real-hardware testing is the only way to know. Mitigations in Decision 5.
2. **Stack pressure from ML-KEM-768 and zkgroup endorsement batches.** 4–8 MiB target stack on Precursor (16 MiB total RAM). Profile under realistic group-send batches.
3. **`curve25519-dalek` strategy: vendored `betrusted-io/curve25519-dalek` + version bump + lizard-module port (Precursor HW acceleration).** Resolution as of Stage 4: vendor the betrusted-io fork (carries the u32e IP-core driver), bump 4.1.2 → 4.1.3, port the `src/lizard/` module from `signalapp/curve25519-dalek` (additive — no API conflicts). Auto-activate the u32e backend for rv32-xous via `.cargo/config.toml`; hosted Linux falls back to the portable Rust backend. Precursor-only — Bao1x has a different PKE engine that would need its own backend (deferred). xous-core's existing `tunnell/curve25519-dalek` patch should be updated when this workspace merges in (Stage 9).
4. **libsignal-service-rs upstream drift.** The transport fork and tokio-removal patch need rebasing on each upstream commit. Mitigate by keeping each patch self-contained and small. Both patches are upstream-friendly (the tokio-removal especially, since it preserves API compatibility for non-Xous consumers).
5. **smol-rs vendoring health.** Track [`smol-rs/async-executor`](https://github.com/smol-rs/async-executor) and friends; they're well-maintained, but a vendored copy doesn't auto-update. Plan annual review.
6. **Signal moves chat path to HTTP/2.** The `experimental_chat_h2_domain_config` flag in [`libsignal/rust/net/src/env.rs:73`](https://github.com/signalapp/libsignal/blob/v0.91.0/rust/net/src/env.rs#L73) signals trajectory. When/if it goes default, rev the design — likely via a sync HTTP/2 client on rustls.
7. **No CDSI without boring-ssl.** Acceptable for a device-linked-only client; revisit when CDSI's transport stabilizes on a more portable backend.
8. **rv32 toolchain churn.** Coordinate any toolchain bump with the broader Xous community.
9. **No multi-key transactions in PDDB.** The protocol is robust to write-order race conditions, but a startup consistency check is worth adding.

---

## Summary table of design decisions

| # | Decision | Why |
|---|---|---|
| 1 | PDDB-backed storage in a password-locked Basis; no presage-store-cipher | PDDB already provides AEAD + plausible deniability |
| 2 | Vendor smol-rs primitives (~21 kLoC); patch presage and libsignal-service-rs to remove Tokio (~30-line diff) | 5× smaller audit surface than Tokio; single-threaded `LocalExecutor` matches `?Send` storage layer; patch is small enough to upstream |
| 3 | Sync TLS+WS worker thread bridged to async via `async-channel`; fork libsignal-service-rs to swap reqwest/reqwest-websocket for tungstenite/ureq (~2 kLoC) | Xous has no async network layer; sync transport over existing rustls + Xous TcpStream + DNS works today |
| 4 | Manager owns nothing platform-specific; IPC server forwards via `async-channel` | The two runtimes can't share a scheduler; queues bridge them |
| 5 | Accumulate dirty SessionRecords in RAM, batch-flush in ~4 KiB chunks on debounce/quiescence | PDDB performance is best with batched writes; aligns with `bunnie`'s and `kotval`'s guidance |
| 6 | **Mirror** xous-core's `[patch.crates-io]` for sha2/ring/getrandom in our standalone workspace (NOT the `aes` patch — see Decision 7); vendor `betrusted-io/curve25519-dalek` (Precursor HW-accel) + version bump + lizard-port; auto-activate u32e backend for rv32-xous via `.cargo/config.toml` | Avoids rv32 build failures; HW-accelerated ECC on Precursor; portable Rust fallback elsewhere |
| 7 | Stay a separate workspace; bundle the rv32 binary into Xous images via `xtask` rather than merging into a xous-core fork | xous-core's `[patch.crates-io]` (especially `aes` → `services/aes`) is API-incompatible with libsignal's RustCrypto-style deps; cargo doesn't allow per-subtree patches; staying separate is the natural alignment with cryptography.rs's curated picks |

Total custom code budget: roughly 4 kLoC of project code (presage-store-pddb + xous-net-bridge + xous-signal-bridge) plus ~2 kLoC of fork patches against libsignal-service-rs and presage. Plus the vendored `betrusted-io/curve25519-dalek` fork (~12 kLoC; the betrusted-io's own additions over upstream are the u32e backend and one extra feature flag; our own delta is the lizard-module port + a one-line version bump). Vendored async surface: ~21 kLoC of smol-rs primitives. Everything cryptographically sensitive stays inside upstream code: `libsignal/rust/protocol/`, `libsignal/rust/zkgroup/`, `signalapp/SparsePostQuantumRatchet`, plus the betrusted-io-curve25519-dalek + lizard vendored copy.

---

## Useful links

- [`whisperfish/presage`](https://github.com/whisperfish/presage) — top-level state machine
- [`whisperfish/libsignal-service-rs`](https://github.com/whisperfish/libsignal-service-rs) — service / transport layer
- [`signalapp/libsignal`](https://github.com/signalapp/libsignal/tree/v0.91.0) — protocol crate, pinned to v0.91.0
- [`signalapp/SparsePostQuantumRatchet`](https://github.com/signalapp/SparsePostQuantumRatchet/tree/v1.5.1) — Triple Ratchet's PQ component, pinned to v1.5.1
- [`betrusted-io/xous-core`](https://github.com/betrusted-io/xous-core) — host OS; PDDB at [`services/pddb`](https://github.com/betrusted-io/xous-core/tree/main/services/pddb), TLS at [`libs/tls`](https://github.com/betrusted-io/xous-core/tree/main/libs/tls)
- [`betrusted-io/xous-book`](https://github.com/betrusted-io/xous-book) — design docs; PDDB chapter at [`src/ch09-00-pddb-overview.md`](https://github.com/betrusted-io/xous-book/blob/master/src/ch09-00-pddb-overview.md)
- [`smol-rs/async-executor`](https://github.com/smol-rs/async-executor), [`async-task`](https://github.com/smol-rs/async-task), [`async-channel`](https://github.com/smol-rs/async-channel), [`async-lock`](https://github.com/smol-rs/async-lock), [`event-listener`](https://github.com/smol-rs/event-listener), [`futures-lite`](https://github.com/smol-rs/futures-lite), [`futures-timer`](https://github.com/async-rs/futures-timer)
- [`cryptography.rs`](https://cryptography.rs/) — curated index of pure-Rust crypto crates. Used as the verification standard at Stage 6.5: every crypto primitive libsignal's protocol path uses (`aes 0.8`, `aes-gcm-siv 0.11`, `ctr 0.9`, `hkdf 0.12`, `hmac 0.12`, `sha2 0.10`, `subtle 2.6`, `curve25519-dalek 4.1.x`, `ed25519-dalek 2.x`, `x25519-dalek 2.x`, `argon2 0.5`, `getrandom 0.3`, `rustls 0.22.x`, `webpki-roots`, `zeroize 1.x`) is on the cryptography.rs index. The one deviation is `libcrux-ml-kem` (libsignal's choice — formally verified in F\* via the hax toolchain) over cryptography.rs's `ml-kem` (RustCrypto, FIPS-203). Both are pure-Rust on rv32; we follow libsignal's choice to minimize divergence. Stage 6.5's verification matrix re-confirms this every time we rebase libsignal.

Reference embeddings to study (both use a similar shape — Tokio runtime in a worker thread forwarding to a non-Rust event loop; we substitute `LocalExecutor` for the runtime):

- [`boxdot/gurk-rs`](https://github.com/boxdot/gurk-rs) — TUI client
- [`hoehermann/purple-presage`](https://github.com/hoehermann/purple-presage) — Pidgin plugin

Signal protocol reading (one paper each, light):

- [Signal X3DH spec](https://signal.org/docs/specifications/x3dh/) — initial handshake
- [Signal Double Ratchet spec](https://signal.org/docs/specifications/doubleratchet/) — message ratchet
- [PQXDH](https://signal.org/docs/specifications/pqxdh/) — Kyber-augmented X3DH
