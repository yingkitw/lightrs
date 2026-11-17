//! OS sandbox for spawned children (`[sandbox]` config) — a thin wrap around
//! `sh -c` spawns, not a process-wide jail: the agent's own file tools stay
//! governed by `validate_work_path` (they legitimately write outside workdir —
//! sessions, config — so seatbelt can't help there anyway).
//!
//! Backends: `sandbox-exec` (seatbelt) on macOS, `bwrap` on Linux, fail-closed
//! elsewhere — a user who opted into confinement gets a clear error, never
//! silent unsandboxed execution.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// `[sandbox]` — `on` gates everything; `writable` adds roots beyond
/// workdir+tmp (toolchain caches: `~/.cargo`, `~/.rustup`, `~/.npm`); `network`
/// false closes the exfil path a readable-file leak would otherwise take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxCfg {
    pub on: bool,
    pub network: bool,
    pub writable: Vec<String>,
}

impl Default for SandboxCfg {
    fn default() -> Self {
        Self {
            on: false,
            network: true,
            writable: Vec::new(),
        }
    }
}

/// Wrap `(prog, args)` in the OS sandbox when enabled. Off → passthrough, so
/// spawn sites call this unconditionally.
pub(crate) fn wrap_spawn(
    cfg: &SandboxCfg,
    workdir: &Path,
    prog: &str,
    args: &[String],
) -> Result<(String, Vec<String>)> {
    if !cfg.on {
        return Ok((prog.to_string(), args.to_vec()));
    }
    if cfg!(target_os = "macos") {
        let argv: Vec<String> = ["-p".to_string(), seatbelt_profile(workdir, cfg), prog.to_string()]
            .into_iter()
            .chain(args.iter().cloned())
            .collect();
        Ok(("sandbox-exec".into(), argv))
    } else if on_path("bwrap") {
        let mut v = vec![
            "--die-with-parent".to_string(),
            "--ro-bind".into(),
            "/".into(),
            "/".into(),
            "--bind".into(),
            workdir.to_string_lossy().to_string(),
            workdir.to_string_lossy().to_string(),
            "--tmpfs".into(),
            "/tmp".into(),
            "--dev".into(),
            "/dev".into(),
            "--proc".into(),
            "/proc".into(),
        ];
        if !cfg.network {
            v.push("--unshare-net".into());
        }
        for w in writable_roots(workdir, cfg).into_iter().skip(1) {
            // skip(1): workdir already bound above
            let s = w.to_string_lossy().to_string();
            v.extend(["--bind".into(), s.clone(), s]);
        }
        v.push(prog.to_string());
        v.extend(args.iter().cloned());
        Ok(("bwrap".into(), v))
    } else {
        bail!(
            "sandbox enabled but no backend on this platform — install bubblewrap (`bwrap`) or set [sandbox].on = false"
        )
    }
}

/// Writable roots: canonicalized workdir first, then tmp dirs, then configured
/// extras (`~` expanded). Seatbelt needs realpaths — a workdir reached through
/// a symlink would otherwise still deny writes.
fn writable_roots(workdir: &Path, cfg: &SandboxCfg) -> Vec<PathBuf> {
    let mut roots = vec![workdir.canonicalize().unwrap_or_else(|_| workdir.to_path_buf())];
    if cfg!(target_os = "macos") {
        roots.extend(
            ["/tmp", "/private/tmp", "/private/var/folders"]
                .iter()
                .map(PathBuf::from),
        );
    }
    for w in &cfg.writable {
        roots.push(expand_tilde(w));
    }
    roots
}

fn expand_tilde(s: &str) -> PathBuf {
    match s.strip_prefix("~/") {
        Some(rest) => crate::paths::home_dir().join(rest),
        _ => PathBuf::from(s),
    }
}

/// Seatbelt profile — the workspace-write shape verified on macOS:
/// read/exec/network allowed, writes confined to the writable roots, `/dev`
/// writable for shell redirections, ipc/sysctl/mach needed by toolchains.
fn seatbelt_profile(workdir: &Path, cfg: &SandboxCfg) -> String {
    let writes: String = writable_roots(workdir, cfg)
        .iter()
        .map(|p| format!("(subpath \"{}\")", p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join("");
    let net = if cfg.network { "(allow network*)" } else { "" };
    format!(
        "(version 1)(deny default)(allow file-read*)(allow file-write-data (regex #\"^/dev/\"))(allow file-ioctl)(allow process-exec process-fork signal)(allow file-write* {writes})(allow sysctl-read)(allow mach-lookup)(allow ipc-posix-shm)(allow system-socket){net}"
    )
}

/// `command -v` without a subprocess — stat the PATH dirs directly.
fn on_path(prog: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_passthrough() {
        let (prog, args) = wrap_spawn(
            &SandboxCfg::default(),
            Path::new("/w"),
            "sh",
            &["-c".into(), "ls".into()],
        )
        .unwrap();
        assert_eq!(prog, "sh");
        assert_eq!(args, vec!["-c", "ls"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_profile_confines_writes() {
        let cfg = SandboxCfg {
            on: true,
            network: true,
            writable: vec!["~/.cargo".into()],
        };
        let p = seatbelt_profile(Path::new("/w dir"), &cfg);
        assert!(p.contains("(subpath \"/w dir\")"), "{p}");
        assert!(p.contains("(allow network*)"), "{p}");
        assert!(p.contains(".cargo"), "{p}");
        let off_net = seatbelt_profile(
            Path::new("/w"),
            &SandboxCfg { network: false, ..cfg },
        );
        assert!(!off_net.contains("network"), "{off_net}");
    }

    #[test]
    fn on_without_backend_fails_closed() {
        // Only testable where seatbelt is absent — macOS always has it.
        #[cfg(not(target_os = "macos"))]
        {
            let cfg = SandboxCfg { on: true, ..Default::default() };
            let r = wrap_spawn(&cfg, Path::new("/w"), "sh", &[]);
            if !on_path("bwrap") {
                assert!(r.is_err());
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_actually_confines() {
        // Live check — spawn a real sandboxed write inside vs outside workdir.
        let dir = std::env::temp_dir().join(format!("lightrs-sbx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = SandboxCfg { on: true, ..Default::default() };
        let inside = format!("touch {}/in", dir.display());
        let (prog, args) = wrap_spawn(&cfg, &dir, "sh", &["-c".into(), inside]).unwrap();
        let ok = std::process::Command::new(prog).args(&args).status().unwrap();
        assert!(ok.success());
        assert!(dir.join("in").exists());
        let outside = format!("touch {}/out", dir.parent().unwrap().join(format!("sbx-escape-{}", std::process::id())).display());
        let (prog, args) = wrap_spawn(&cfg, &dir, "sh", &["-c".into(), outside.clone()]).unwrap();
        let st = std::process::Command::new(prog).args(&args).status().unwrap();
        assert!(!st.success(), "write outside workdir must be denied");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap().join(format!("sbx-escape-{}", std::process::id())));
    }
}
