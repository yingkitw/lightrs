//! Formal verification harness: bounded model checking (Kani) + property tests (proptest).
//!
//! Security- and UX-critical pure functions are checked here against documented
//! invariants. Run `cargo test verify` for properties; `cargo kani` for proofs
//! (see `docs/verify.md`).

#[cfg(kani)]
mod kani_proofs;

#[cfg(test)]
mod properties;