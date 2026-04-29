# PDDB CI test (`pddbci.py`)

This document explains what `tools/pddbci.py` is, what it tests, and how to run it. It exists because the harness's behavior — what `--name` does, what `--runs` does, why `XOUS_SEED` matters — isn't obvious from the script itself.

## Two layers

The system is two stacked components: a Python harness that orchestrates many runs, and a Rust test that runs once per invocation inside a hosted Xous emulator.

### Layer 1: `pddbci.py` (the harness)

For each seed in `0..runs`:

1. Sets `XOUS_SEED=<seed>` in the child env so the simulation is deterministic.
2. Removes any stale `tools/pddb-images/<name>.bin` and `<name>.key` from a previous run.
3. Spawns `cargo xtask pddb-ci` — builds and boots a hosted Xous emulator with the `pddb/ci` and `pddb/deterministic` features. The `pddb/ci` feature shrinks the simulated disk to **4 MB** so that filling it (and exercising the recovery path) happens quickly.
4. Watches the emulator log for sentinels:
   - `INFO:pddb::tests: CI done` — test sequence completed.
   - `lack of free space`, `no free pages`, `Ran out of memory`, etc. — out-of-disk-space conditions (now distinguished from a passing run that triggers the recovery WARN line).
   - `Decryption auth error` — auth failure.
5. After the emulator step, runs `pddbdbg.py --name <name> --ci` against the dumped image. The analyzer reads the on-disk format and emits `All dicts were found.` if every expected dictionary checksums correctly.
6. Records PASS / FAIL / OOM / etc. and moves on to the next seed.

### Layer 2: `services/pddb/src/tests.rs::ci_tests` (the actual test)

A stress test of PDDB's allocator and basis machinery on the 4 MB CI disk. Each phase ends with a `dbg_dump(...)` so the analyzer has a concrete on-disk image to verify.

| Phase | What it does | Image dumped |
|---|---|---|
| 1. **basecase1e** | Format disk; create 4 dictionaries with 32 keys each on the system basis | `basecase1e.bin` |
| 2. **patche / patterne** | Patch some keys (overwrite mid-key data); checksums verify | `patche.bin`, `patterne.bin` |
| 3. **patche2 / patterne2** | Same patches under a second basis (encrypted "Basis2") | `patche2.bin`, `patterne2.bin` |
| 4. **dachecke** (delete-add-check) | Delete some keys, allocate fresh ones with extension | `dachecke.bin` |
| 5. **dachecke2, dachecke3** | Repeat delete-add-check with growing pressure on the allocator | `dachecke2.bin`, `dachecke3.bin` |
| 6. **dachecke4** | Same pattern with `dict_count = 6`. This **exhausts free space**, forcing the FastSpace deep-sweep recovery path | `dachecke4.bin` |
| 7. **remounte** | Unmount, remount; verify all keys survive a remount cycle | `remounte.bin` |
| 8. **basis2** | Open Basis2, add more keys, verify cross-basis isolation | `basis2.bin` |
| 9. — | Final log line `INFO:pddb::tests: CI done` | — |

The keys contain deterministic checksums computed from name + content pattern; the analyzer reads the dumped image and verifies every checksum.

## What the test is actually checking

- **Allocator correctness under pressure.** Phase 6 deliberately fills the 4 MB disk to exercise the FastSpace deep-sweep recovery. This is the *core* test scenario — not an accidental edge case.
- **Basis isolation.** Keys in Basis2 must not be visible from the system basis and vice versa.
- **Patch consistency.** Partial overwrites preserve the rest of the key cleanly.
- **Remount round-trip.** Close and reopen the disk; all keys should survive intact.
- **Coverage across RNG states.** Different `XOUS_SEED` values produce different key sizes and content patterns (the RNG drives `gen_key` in `tests.rs`), which exercise different allocator orderings and free-space patterns. The hope is that bugs that only manifest for some specific allocation sequence surface across the seed range.

## Valid `--name` values

The harness's `--name` argument controls **which dumped image the analyzer reads** — it does NOT control which test to run. The test always runs the same sequence (table above). So `--name X` succeeds only if the test writes `X.bin` AND that image contains analyzable dicts.

| `--name` value | Image source | Analyzable? | Notes |
|---|---|---|---|
| `dachecke` | Phase 4 dump | ✅ | Recommended default. First image with the full 4-dict structure intact after delete-add-check. |
| `dachecke2`, `dachecke3`, `dachecke4` | Phases 5, 6 | ✅ | Increasingly stressed allocator state. `dachecke4` reflects post-recovery state. |
| `basecase1e` | Phase 1 | ✅ | Earliest "test has run" image. Smallest scope. |
| `basis2` | Phase 8 | ✅ | Includes both system basis and Basis2 dicts. |
| `patche`, `patterne`, `patche2`, `patterne2` | Phases 2-3 | ✅ | Patched-key states. |
| `remounte` | Phase 7 | ✅ | Post-remount image. |
| **`full`** | NOT WRITTEN by the test | ❌ | The string `"full"` is used by `services/pddb/src/main.rs::dbg_dump("full")` in the **non-CI** PDDB startup path (when modals prompt the user to format a fresh disk). The CI test bypasses that path entirely (it calls `pddb_format` itself in `create_basis_testcase`), so `full.bin` is never written during a CI run. **Do not use `--name full`** — the analyzer will report `FAIL CI COULD NOT RUN` regardless of whether the test passed. |

## Running it

### Setup (one-time)

```bash
# Python deps for the analyzer step
python3 -m pip install --user pycryptodome pyaesni
```

### Single iteration, one seed (fast)

```bash
unset DISPLAY                        # see "DISPLAY note" below
mkdir -p tools/pddb-images
python3 tools/pddbci.py --runs 1 --name dachecke
echo "exit=$?"
```

Expected last lines:

```
INFO:root:Seed 0 PASS
INFO:root:Overall pass, exiting with 0
```

with exit code `0`.

### Full sweep (slow)

```bash
unset DISPLAY
mkdir -p tools/pddb-images
python3 tools/pddbci.py --runs 501 --name dachecke 2>&1 | tee /tmp/pddb-sweep.log
```

A full sweep takes hours on most hardware (each iteration is build-cache-warm cargo + boot + test + analyzer; ~30-90 s per iteration). Use a smaller `--runs` for spot checks.

### Direct run (bypasses the harness)

Useful when diagnosing a specific seed:

```bash
unset DISPLAY
export XOUS_SEED=42
cargo xtask pddb-ci 2>&1 | tee /tmp/seed42.log
# Watch for "INFO:pddb::tests: CI done"; Ctrl+C after you see it
# (the emulator does not exit on its own).
```

## DISPLAY note

The hosted-mode emulator has a `graphics-server` process. By default it tries to open an X11 window; failing that, it falls back to a no-window "headless" mode if the build includes that fallback. For CI:

- **Set `unset DISPLAY` before running** to opt into the headless path.
- If `DISPLAY` is set to a real X server, a window will pop up showing the emulator's screen — harmless for the test result, but visually noisy.
- If `DISPLAY` is set to a value the emulator can't open (e.g. an X server that's gone away), older builds will `std::process::abort()` the whole simulation; ensure the headless fallback path is included.

## Common failure modes

| Symptom | What it usually means |
|---|---|
| `FAIL CI COULD NOT RUN` | Analyzer didn't see `All dicts were found.` Either the image at `--name X.bin` is missing (wrong `--name`, or test never wrote it) or contains no test dicts. |
| `FAIL TIMEOUT (no CI done)` | Test ran the full timeout budget without emitting `CI done`. Check the log for a panic, deadlock, or "lack of free space"-followed-by-silence (FastSpace recovery hang). |
| `OOM` | Test legitimately ran out of disk space. The 4 MB CI disk is intentionally tight; this means the test couldn't recover free pages. |
| Modal window appears with "PDDB needs to allocate more free space..." | The build is missing the `feature = "ci"` gate on `pddb_get_all_keys` so the test is calling the user-interactive UX path. The recovery path requires either a user (impossible in CI) or a CI-aware build configuration. |

## See also

- `tools/pddbci.py` — the harness
- `tools/pddbdbg.py` — the analyzer
- `services/pddb/src/tests.rs` — the test sequence
- `services/pddb/src/backend/hw.rs` — the FastSpace allocator and recovery path
- `xous-rs/src/definitions.rs` — `TESTING_RNG_SEED` and `XOUS_SEED` env var documentation
