//! Property-based checks (proptest) — filesystem-backed sandbox invariants.

use std::fs;
use std::path::Path;

use proptest::prelude::*;

use crate::config::{validate_read_path, validate_work_path};
use crate::tools::redact::{REDACTED, redact_secrets};
use crate::tools::shell::{escape_controls, strip_ansi};
use crate::tools::truncate_bytes;
use crate::tools::web::{DENIED_METADATA_HOSTS, metadata_host_denied};

proptest! {
    #[test]
    fn truncate_bytes_prefix_and_bound(s in "\\PC{0,200}", max in 0usize..512) {
        let t = truncate_bytes(&s, max);
        prop_assert!(s.starts_with(t));
        prop_assert!(t.is_char_boundary(t.len()));
        if max >= s.len() {
            prop_assert_eq!(t, s.as_str());
        } else {
            prop_assert!(t.len() <= max);
        }
    }

    #[test]
    fn escape_controls_has_no_control_chars(s in "\\PC{0,200}") {
        let out = escape_controls(&s);
        prop_assert!(!out.chars().any(char::is_control));
    }

    #[test]
    fn strip_ansi_has_no_esc(s in "\\PC{0,200}") {
        let out = strip_ansi(&s);
        let esc = '\u{001b}';
        prop_assert!(!out.contains(esc));
    }

    #[test]
    fn sk_token_redacted(body in "[a-zA-Z0-9_-]{8,40}") {
        let line = format!("key=sk-{body}");
        let out = redact_secrets(&line);
        prop_assert!(out.contains(REDACTED));
        prop_assert!(!out.contains(&body));
    }

}

#[test]
fn metadata_table_hosts_denied() {
    for h in DENIED_METADATA_HOSTS {
        assert!(metadata_host_denied(h), "{h}");
        assert!(metadata_host_denied(&format!("[{h}]")), "[{h}]");
    }
}

fn temp_workdir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("lightrs-formal-{label}-{}", std::process::id()))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn validate_work_path_accepts_nested_file(name in "[a-z][a-z0-9]{0,12}") {
        let dir = temp_workdir("work");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let rel = format!("sub/{name}.txt");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join(&rel), "x").unwrap();
        let resolved = validate_work_path(&dir, Path::new(&rel)).unwrap();
        let work = dir.canonicalize().unwrap();
        prop_assert!(resolved.starts_with(work));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_read_path_extra_root(name in "[a-z][a-z0-9]{0,8}") {
        let base = temp_workdir("extra");
        let work = base.join("work");
        let extra = base.join("extra");
        let _ = fs::remove_dir_all(&base);
        for d in [&work, &extra] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(extra.join(format!("{name}.rs")), "e").unwrap();
        let roots = vec![extra.clone()];
        let resolved = validate_read_path(&work, &roots, &extra.join(format!("{name}.rs"))).unwrap();
        prop_assert!(resolved.starts_with(extra.canonicalize().unwrap()));
        let _ = fs::remove_dir_all(&base);
    }
}