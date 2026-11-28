# Formal verification

lightrs uses **two complementary formal methods** on security- and UX-critical code:

| Layer | Tool | What it proves |
|-------|------|----------------|
| Bounded model checking | [Kani](https://model-checking.github.io/kani/) | Pure functions: `truncate_bytes`, `escape_controls`, `strip_ansi`, metadata host deny-list, redact shape |
| Property-based testing | [proptest](https://docs.rs/proptest/) | Same invariants on larger random inputs; path sandbox (`validate_work_path`, `validate_read_path`) on tempdirs |

Sources live under `src/verify/` (`kani_proofs.rs`, `properties.rs`).

## Run locally

```bash
# Property tests (CI-friendly, no extra toolchain)
cargo test verify::

# Kani — install once: cargo install kani-verifier; then:
cargo kani --tests -p lightrs
```

Kani compiles the crate with `cfg(kani)` and symbolically explores bounded input spaces. Path resolution proofs are **not** in Kani (they need a real filesystem); those properties use proptest + the existing deterministic path tests in `config/mod.rs`.

## Documented invariants

1. **`truncate_bytes`** — result is always a UTF-8 char-boundary prefix of the input, byte length ≤ `max` (or full string when shorter).
2. **`escape_controls`** — output contains no `char::is_control()` bytes (terminal injection / forged rows).
3. **`strip_ansi`** — output never contains U+001B (ESC).
4. **`metadata_host_denied`** — every host in `DENIED_METADATA_HOSTS` is denied, including bracketed IPv6 form.
5. **`redact_secrets`** — well-formed `sk-…` tokens with ≥8 body chars do not appear verbatim after redaction.
6. **`validate_work_path` / `validate_read_path`** — resolved paths stay under workdir or declared extra read roots (proptest on nested files).

When adding a new invariant, extend this list, add a Kani proof where the function is pure, and add a proptest where I/O is required.