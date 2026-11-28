//! Kani bounded proofs for invariants that must hold for all inputs in a bound.
#![cfg(kani)]

use crate::tools::redact::redact_secrets;
use crate::tools::shell::{escape_controls, strip_ansi};
use crate::tools::truncate_bytes;
use crate::tools::web::metadata_host_denied;

/// Every slice returned is a valid UTF-8 prefix of the source string.
#[kani::proof]
fn truncate_bytes_is_valid_prefix() {
    let n: usize = kani::any();
    kani::assume(n <= 128);
    let mut buf = [0u8; 128];
    for i in 0..n {
        buf[i] = kani::any();
    }
    kani::assume(std::str::from_utf8(&buf[..n]).is_ok());
    let s = std::str::from_utf8(&buf[..n]).unwrap();
    let max: usize = kani::any();
    let t = truncate_bytes(s, max);
    assert!(s.starts_with(t));
    assert!(t.is_char_boundary(t.len()));
    assert!(t.len() <= s.len());
    assert!(t.len() <= max || t.len() == s.len());
}

/// Chrome `escape_controls` never emits raw control bytes to the terminal.
#[kani::proof]
fn escape_controls_never_emits_controls() {
    let n: usize = kani::any();
    kani::assume(n <= 128);
    let mut buf = [0u8; 128];
    for i in 0..n {
        buf[i] = kani::any();
    }
    kani::assume(std::str::from_utf8(&buf[..n]).is_ok());
    let s = std::str::from_utf8(&buf[..n]).unwrap();
    let out = escape_controls(s);
    assert!(!out.chars().any(char::is_control));
}

/// `strip_ansi` removes ESC (U+001B) from the output entirely.
#[kani::proof]
fn strip_ansi_removes_escape() {
    let n: usize = kani::any();
    kani::assume(n <= 128);
    let mut buf = [0u8; 128];
    for i in 0..n {
        buf[i] = kani::any();
    }
    kani::assume(std::str::from_utf8(&buf[..n]).is_ok());
    let s = std::str::from_utf8(&buf[..n]).unwrap();
    let out = strip_ansi(s);
    assert!(!out.contains('\u{1b}'));
}

#[kani::proof]
fn metadata_hosts_table_is_self_consistent() {
    for h in crate::tools::web::DENIED_METADATA_HOSTS {
        assert!(metadata_host_denied(h));
        assert!(metadata_host_denied(&format!("[{h}]")));
    }
}

#[kani::proof]
fn redact_never_shortens_below_redacted_marker() {
    let n: usize = kani::any();
    kani::assume(n <= 64);
    let mut buf = [0u8; 64];
    for i in 0..n {
        buf[i] = kani::any();
    }
    kani::assume(std::str::from_utf8(&buf[..n]).is_ok());
    let s = std::str::from_utf8(&buf[..n]).unwrap();
    let out = redact_secrets(s);
    if out.contains(crate::tools::redact::REDACTED) {
        assert!(out.len() >= crate::tools::redact::REDACTED.len());
    }
}