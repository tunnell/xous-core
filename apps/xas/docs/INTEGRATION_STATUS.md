# INTEGRATION_STATUS.md — Stage 9b checkpoint

This file records what we tried, what we found, and the open
architectural question. It supersedes the speculative parts of
`INTEGRATION.md` (which assumed the merge would be mechanical).

## What's in this commit

`apps/xas/` contains the full `xous-app-signal` workspace as files —
crates, vendored upstream forks, docs, stage reports. **None of it
is wired into xous-core's build.** xous-core's top-level
`Cargo.toml`, `.cargo/config.toml`, and the
`services/{root-keys,shellchat}/Cargo.toml` files are unchanged from
upstream `dev-for-xas`. So:

- `cargo build` on `xous-core-for-xas/xas` builds xous-core normally
  (no `apps/xas/` regressions).
- The `apps/xas/` subtree compiles **only** as the standalone
  `xous-app-signal` workspace at
  `~/precursor-signal/xous-app-signal/`. That's where Stage 8's
  `xas: pong` smoke test runs and where Stages 4+5+6+7+8's tests
  pass.

The PR `xas → dev-for-xas` exists to make this **diff** visible:
"here is the code we want to integrate; here is the unchanged
xous-core". Reviewing the PR shows exactly what's pending.

## What happened when we tried the merge

INTEGRATION.md claimed the merge was a mechanical recipe. It is
not. The xous-core fork has three top-level `[patch.crates-io]`
entries that are **incompatible with libsignal at the API level**:

### 1. `[patch.crates-io.aes]` redirects to `services/aes`

xous-core's `services/aes` is a Xous-IPC shim around its hardware
AES engine. It declares itself as `aes 0.8.4` (semver-compatible
with libsignal's `aes = "0.8.3"`) but does not export
`Aes256Enc`, which `zkgroup::api::profiles::profile_key.rs:74` calls
directly:

```rust
let aes = ::aes::Aes256Enc::new((&self.bytes).into());
```

Replacing the patch (or modifying `services/aes` to add
`Aes256Enc`) is the cleanest fix, but neither is small. xous-core
ships its own `services/aes` to avoid pulling the whole
RustCrypto `aes` crate at the kernel level; flipping that decision
ripples through several services.

### 2. `[patch.crates-io.curve25519-dalek]` is at version 4.1.2

`betrusted-io/curve25519-dalek/main` (the patch target) is at
4.1.2 with the u32e IP-core driver and the `auto-release` /
`warn-fallback` features. libsignal's zkgroup pins
`curve25519-dalek = "4.1.3"` and uses methods (the lizard module)
that don't exist in betrusted-io's fork. We tried two paths:

- **Path A:** replace the patch with our vendored copy (4.1.3 +
  betrusted-io's u32e + lizard module port). This forced
  `=4.1.2 → =4.1.3` bumps in `services/{root-keys,shellchat}`'s
  `[dependencies.curve25519-dalek]` blocks. The version bump itself
  is behaviour-preserving (our vendored fork includes everything
  the betrusted-io fork has), but it's two more files of diff and
  cross-cuts xous-core's services. If a future xous-core upstream
  sync lands a `=4.1.2` re-pin we'd have to rebump. It is workable
  but not free.
- **Path B:** keep both. xous-core gets 4.1.2 from the original
  patch, libsignal gets 4.1.3 from our vendored copy via
  `[patch."https://github.com/signalapp/curve25519-dalek"]`. Two
  compiled copies of `curve25519-dalek` live in the binary. Cargo
  rejected this — root-keys's `=4.1.2` and zkgroup's `^4.1.3`
  share the same crates.io alias and can't resolve to two
  different versions through `[patch.crates-io]`.

Path A is the workable one but requires the xous-core service
edits.

### 3. `[patch.crates-io.getrandom]` redirects to `imports/getrandom`

xous-core's `imports/getrandom` is a getrandom 0.2 fork that uses
`xous-ipc` and (transitively) `rkyv`. The Signal stack pulls
`getrandom 0.3` (via libsignal's modern `rand`/`zkgroup` paths),
which is unaffected by xous-core's 0.2 patch. We added
`--cfg getrandom_backend="custom"` to `.cargo/config.toml` and an
`__getrandom_v03_custom` extern to our binary. That worked at the
linker level but exposed a **resolver-level cycle** between
`imports/getrandom 0.2 → uuid 1.x → rkyv 0.8 → imports/getrandom 0.2`
through optional features. `cargo update` papered over it but the
cycle is structurally there — adding any new transitive dep that
activates uuid's `rng` or `v4` feature would re-trigger it.

## The open architectural question

The merge approach assumed xous-core's `[patch.crates-io]` entries
would coexist with libsignal's deps. They don't. Three paths
forward, ordered by least-invasive:

### Option I — Separate workspace, custom xtask bundling

Keep `xous-app-signal` as a standalone workspace at
`~/precursor-signal/xous-app-signal/`. Build the rv32 binary
there. Bundle the resulting ELF into a Xous image via a custom
xtask that injects it into xous-core's image-builder pipeline.
The two trees stay decoupled.

**Pros**:
- xous-core's `[patch.crates-io]` is irrelevant to our build.
- Vendored upstream forks (libsignal-service-rs / presage /
  curve25519-dalek) live in our tree, not xous-core's.
- The `apps/xas/` subtree in `tunnell/xous-core` is documentation
  + reference code, not a build artifact. No PR drift over time.

**Cons**:
- We have to write the xtask integration.
- Some xous-core service path-deps (pddb, trng, xous-names, etc.)
  need to be reachable from our standalone — either by relative
  path (`../repos/xous-core/services/pddb`) or by re-vendoring.
  Path-deps are simpler.
- Users wanting to run xas need both repos cloned.

### Option II — Make services/aes API-compatible with upstream

Add `Aes256Enc`, `Aes256Dec`, etc. wrappers to
`services/aes/src/lib.rs` so `zkgroup` resolves them correctly.
The wrappers can route to existing Xous IPC calls. Plus the
curve25519-dalek and getrandom 0.3 work from our attempt.

**Pros**:
- Single workspace.
- Apps/xas/ is a real workspace member; the build pipeline is
  one command.
- Future Xous apps that pull the libsignal stack (or any RustCrypto
  trait-using crate) inherit the fix.

**Cons**:
- Adding `Aes256Enc` requires understanding what zkgroup expects
  vs what services/aes provides; not entirely mechanical.
- This is xous-core surgery; we'd be PRing nontrivial changes
  upstream eventually.

### Option III — Vendor upstream `aes` and use a per-package override

Use cargo's source-replacement / dependency-override features to
route just our crates' `aes` dep to upstream RustCrypto's `aes`
0.8.4 (from crates.io), bypassing xous-core's patch for our
sub-graph.

**Pros**:
- No xous-core surgery.
- Single workspace.

**Cons**:
- Cargo doesn't support per-subtree `[patch.crates-io]`.
  Workarounds (cargo's `[source.crates-io] replace-with =` or
  patched-Cargo.toml dependency-overrides) are fragile and don't
  compose well with workspace inheritance.
- Not a documented stable feature.

## Recommendation

**Option I (separate workspace, xtask bundling)** is the path of
least surprise. It preserves the standalone workspace investment
(Stages 0–8 all green there), keeps `xous-core-for-xas/xas` as a
human-readable fork-of-record showing what we want, and defers the
deeper xous-core surgery (Option II) until after the MVP flows
work.

Concretely, Stage 9b becomes:

1. **Stay in standalone**. The `apps/xas/` directory stays as a
   reference dump in `tunnell/xous-core/xas` — not a build target.
2. **Implement the real PDDB backend** as a path-dep on
   `~/precursor-signal/repos/xous-core/services/pddb` from our
   standalone `presage-store-pddb`. Same for `xous-names`, `trng`.
3. **Write `xtask`** in standalone that builds the `xas` binary
   for rv32 and copies it into xous-core's `apps/` directory at
   image-build time, OR registers it via the Renode boot script
   directly.
4. **Stage 9b's Renode test runs** out of the standalone, against
   an image built by xous-core's existing pipeline plus our
   injected ELF.

The PR `xas → dev-for-xas` stays open as the diff-of-record.

## What this means for the user

A design decision is needed before the next code milestone. The
options above are all technically reachable; the user's preference
on tradeoffs (Option I's two-repos cost vs. Option II's
xous-core-surgery cost vs. Option III's tooling-fragility cost)
chooses among them.

Until that decision lands, the `xas` branch is the
documentation-of-intent: `apps/xas/` shows the code that wants to
ship; `INTEGRATION.md` shows the merge plan; this file shows why
the merge plan needs revision; `SYNC.md` shows the branch model
that survives whichever option we pick.
