# Fuzzing

Uses [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) with libFuzzer.

## Setup

```bash
cargo +nightly install cargo-fuzz
```

## Run

```bash
# List targets
cargo +nightly fuzz list

# Run specific target (indefinitely)
cargo +nightly fuzz run patch_from_str

# Run with time limit (seconds)
cargo +nightly fuzz run patch_from_str -- -max_total_time=60

# Run all targets (quick smoke test)
for t in $(cargo +nightly fuzz list); do
  cargo +nightly fuzz run $t -- -max_total_time=10
done
```

## Targets

| Target | Tests |
|--------|-------|
| `patch_from_str` | `Patch::from_str()` |
| `patch_from_bytes` | `Patch::from_bytes()` |
| `patches_parse_gitdiff` | `Patches::parse(..., gitdiff())` |
| `patches_parse_unidiff` | `Patches::parse(..., unidiff())` |
| `patches_binary` | `Patches::parse(..., gitdiff().keep_binary())` |

## Crashes

Crash inputs are saved to `fuzz/artifacts/<target>/`. To reproduce:

```bash
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/crash-<hash>
```
