# Sketch: rkyv CheckBytes for IPC memory messages

**Status:** conversation aid, not a proposal. This file exists only on
branch `sketch/rkyv-checkbytes-xous-names` so it's easy to drop.

## Why this sketch exists

`xous_ipc::Buffer::to_original` uses `rkyv::access_unchecked`, which
dereferences attacker-controllable offsets, enum tags, and length
prefixes without validation. This is a long-standing known issue.
The question is: **what fix is worth the effort?**

The sketch below is one direction — rkyv's checked-access API — with
its limitations stated up front so the discussion isn't about a
strawman.

## The sketch

### 1. Enable `bytecheck` on rkyv in crates that produce or consume IPC payloads

```toml
# api/xous-api-names/Cargo.toml
rkyv = { version = "0.8.8", default-features = false, features = [
    "std",
    "alloc",
    "bytecheck",       # <-- new
] }

# xous-ipc/Cargo.toml — same addition
```

### 2. Opt each IPC payload type into CheckBytes derivation

```rust
// api/xous-api-names/src/api.rs
#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[rkyv(derive(bytecheck::CheckBytes))]   // <-- new
pub struct Disconnect {
    pub name: String,
    pub token: [u32; 4],
}
```

### 3. Add a sibling `to_original_checked` on Buffer

```rust
// xous-ipc/src/buffer.rs
pub fn to_original_checked<T, U>(&self) -> Result<T, Error>
where
    T: rkyv::Archive<Archived = U>,
    U: Portable,
    U: for<'a> bytecheck::CheckBytes<
        rkyv::api::high::HighValidator<'a, rkyv::rancor::Error>,
    >,
    <T as rkyv::Archive>::Archived:
        rkyv::Deserialize<T, rkyv::api::high::HighDeserializer<rkyv::rancor::Error>>,
{
    let r = rkyv::access::<U, rkyv::rancor::Error>(&self.slice[..self.used])
        .map_err(|_| Error::InvalidData)?;           // <-- refuses malformed bytes
    rkyv::deserialize::<T, rkyv::rancor::Error>(r)
        .map_err(|_| Error::InternalError)
}
```

Existing `to_original` stays for now (so migration is per-call, not
flag-day).

### 4. Switch one call site as a demo

```rust
// services/xous-names/src/main.rs
Some(api::Opcode::Disconnect) => {
    let mem = msg.body.memory_message_mut().unwrap();
    let mut buffer = unsafe { Buffer::from_memory_message_mut(mem) };
    let disconnect = match buffer.to_original_checked::<Disconnect, _>() {
        Ok(d) => d,
        Err(_) => {
            error!("Disconnect: malformed rkyv payload, rejecting");
            buffer.replace(api::Return::Failure).ok();
            continue;
        }
    };
    // ... rest unchanged
}
```

## What this sketch does protect against

rkyv with CheckBytes validates, per `bytecheck`'s own documentation and
rkyv's archive layout:

1. **Primitive bit patterns**: `bool` that isn't 0 or 1; `char` that
   isn't a valid Unicode scalar; `NonZero*` types with zero bytes.
2. **Enum discriminants**: discriminant not matching any declared
   variant → validation fails.
3. **Internal relative pointers** (rkyv's archived types use relative
   pointers for e.g. `String`, `Vec`, `Option<Box<…>>`): the pointer
   must resolve within the archived bytes and to the correct
   alignment/type.
4. **Length fields**: slice/string/Vec lengths that would overflow
   the archived bytes → validation fails.
5. **Structural layout**: the archived representation must match the
   declared `#[repr]` and the field order of the archived type.

Together, **these catch the UB cases** that motivated F-004 — the
kind where you `match` on an enum whose discriminant is out-of-range,
or deref a pointer whose offset lands on the next page, or read past
the end of a Vec's heap allocation.

## What this sketch does *not* protect against

The obvious counter-question is "can bytes be forged to pass
CheckBytes and still cause harm downstream?" Yes — at two layers:

### Layer A — "forging" valid CheckBytes

CheckBytes validation is **deterministic and spec'd**. An attacker
can, in principle, construct any byte sequence that passes validation:
build a valid archived representation of a `Disconnect { name:
"victim-server", token: [u32::MAX; 4] }`, write those bytes into a
memory message, and the server will happily deserialize into a
well-typed Rust value. **That's not a bug in CheckBytes — it's by
design**: CheckBytes validates the *bit-level representation* is
safe to interpret as the declared type. It does not attest that the
*values* are semantically authorized.

So CheckBytes *does not* prevent an attacker from handing the server
a structurally valid but semantically hostile payload. That's a
**different problem**, usually called semantic validation or
authorization, and needs to live in the server itself (e.g. does this
caller have permission to disconnect `victim-server`? does this token
actually match?).

### Layer B — gaps in what CheckBytes validates

Even at the bit-level layer, CheckBytes has holes worth being honest
about:

1. **`#[archive(as = "…")]` and custom `Portable`**: types the author
   marks as trivially-portable skip CheckBytes. If any IPC payload
   type uses these, a crafted buffer skips validation for that field.
   Need to audit every derive in the xous-ipc graph.
2. **`bytecheck` bugs**: bytecheck is a third-party crate; has had
   soundness issues historically. Tracking a fixed version is
   important.
3. **rkyv archive-layout bugs**: there have been rkyv issues
   (e.g. before 0.7) where the layout check didn't cover every
   reachable pointer. rkyv 0.8 is better but not audited.
4. **Compound types with internal invariants**: e.g. `String` is
   bytes + length, validated as valid UTF-8 by CheckBytes — but
   UTF-8 is not the same as "contains no control characters" or
   "is a valid ServerName." If a server `match`es on `str` contents,
   the attacker controls the match arm.

## So: is this worth doing?

Honest answer: **CheckBytes closes one class of bug (memory-safety UB
from malformed archives), and does not close another (semantic
authorization of structurally-valid payloads).** Both classes matter;
the second is where most exploitable bugs live in practice once the
first is fixed.

### Recommended path (for discussion)

1. **Short term, low effort**: migrate `Buffer::to_original` to
   `rkyv::access` unconditionally (not a parallel method — swap the
   call). Derive CheckBytes on every IPC payload type in a single
   workspace pass. This closes the memory-safety UB door. It's a
   flag-day change — worth a parallel branch,
   testing on both targets, and Xobs review.
2. **Medium term**: audit each server's reply-path for trust in
   deserialized values (the "can an attacker ask for someone else's
   server token" question). This is where the real pocket-device
   exploits live. Not rkyv's problem to solve.
3. **Longer term, if that's still insufficient**: consider whether
   the whole "zero-copy memory message" primitive is the right shape
   for mutually-distrusting processes, or whether a byte-copy + typed
   validation layer is worth the perf cost. This is where Xobs
   probably has strong opinions.

## What this sketch does not contain

- No actual code change on this branch yet; it's markdown only.
- No `cargo build` verification. Enabling `bytecheck` may cascade
  through rkyv's feature tree and hit other crates.
- No performance analysis. Checked access is O(archived-size) instead
  of O(1) and could matter on the IPC hot path.
- No analysis of server reply paths (which is where most of the real
  risk lives per the "forge CheckBytes" concern).

## Questions for the discussion

1. Is the memory-safety UB layer (Layer A) worth closing on its own,
   or do you want a wider design before any code change?
2. If yes, should we do the whole workspace in one flag-day PR, or
   migrate server-by-server with `to_original_checked` running in
   parallel until every site is converted?
3. Where do you want the discussion to happen — GitHub Discussions, a
   branch README, a design doc in `docs/`? I can draft the Discussion
   body if you like.
4. Is Xobs available to weigh in? If so, I can draft an invite.
