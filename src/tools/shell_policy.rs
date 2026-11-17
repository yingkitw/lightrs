#[derive(Clone, Debug, Default)]
pub struct ShellPolicy {
    prefixes: Vec<String>,
    require_approval: bool,
    /// `[sandbox]` — OS confinement for spawned children (seatbelt/bwrap).
    sandbox: super::sandbox::SandboxCfg,
    /// Extra **read-only** roots beyond workdir (`extra_read_dirs`/`--add-dir`).
    read_roots: Vec<std::path::PathBuf>,
    /// Session cwd for `run_command` — `cd` persists across calls (shared
    /// cell; policy clones see the same state). `reset_session_cwd` on workdir
    /// change / sub-agent spawn.
    session_cwd: std::sync::Arc<std::sync::Mutex<Option<std::path::PathBuf>>>,
}

impl PartialEq for ShellPolicy {
    /// Policy identity = configuration; `session_cwd` is ephemeral runtime
    /// state and is deliberately excluded.
    fn eq(&self, other: &Self) -> bool {
        self.prefixes == other.prefixes
            && self.require_approval == other.require_approval
            && self.sandbox == other.sandbox
            && self.read_roots == other.read_roots
    }
}

impl ShellPolicy {
    pub fn new(allowlist: Vec<String>, require_approval: bool) -> Self {
        Self {
            prefixes: allowlist
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            require_approval,
            sandbox: super::sandbox::SandboxCfg::default(),
            read_roots: Vec::new(),
            session_cwd: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Prompt in TUI before `run_command` when true for every command, or when allowlist rejects.
    pub fn needs_interactive_approval(&self, command: &str) -> bool {
        if self.require_approval {
            return !super::shell_readonly::auto_approve_read_only(command);
        }
        self.is_restrictive() && !self.is_allowed(command)
    }

    pub fn requires_all_commands_approved(&self) -> bool {
        self.require_approval
    }

    /// Empty allowlist = no restriction (default).
    pub fn is_allowed(&self, command: &str) -> bool {
        if self.prefixes.is_empty() {
            return true;
        }
        let cmd = command.trim();
        self.prefixes
            .iter()
            .any(|p| cmd == p.as_str() || cmd.starts_with(p.as_str()))
    }

    pub fn is_restrictive(&self) -> bool {
        !self.prefixes.is_empty()
    }

    pub fn allowlist(&self) -> &[String] {
        &self.prefixes
    }

    pub fn sandbox(&self) -> &super::sandbox::SandboxCfg {
        &self.sandbox
    }

    pub fn set_sandbox(&mut self, sandbox: super::sandbox::SandboxCfg) {
        self.sandbox = sandbox;
    }

    pub fn read_roots(&self) -> &[std::path::PathBuf] {
        &self.read_roots
    }

    pub fn set_read_roots(&mut self, roots: Vec<std::path::PathBuf>) {
        self.read_roots = roots;
    }

    /// Shared session-cwd cell — `run_command_session` reads/writes it.
    pub fn session_cwd(&self) -> std::sync::Arc<std::sync::Mutex<Option<std::path::PathBuf>>> {
        self.session_cwd.clone()
    }

    /// Detach a fresh (unset) cwd cell — for `/cd` workdir changes and
    /// `spawn_agent` children (a child's `cd` must not move the parent).
    pub fn reset_session_cwd(&mut self) {
        self.session_cwd = std::sync::Arc::new(std::sync::Mutex::new(None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_allows_all() {
        let p = ShellPolicy::default();
        assert!(p.is_allowed("rm -rf /"));
    }

    #[test]
    fn require_approval_skips_read_only() {
        let p = ShellPolicy::new(vec![], true);
        assert!(!p.needs_interactive_approval("ls"));
        assert!(!p.needs_interactive_approval("git status"));
        assert!(p.needs_interactive_approval("rm foo"));
    }

    #[test]
    fn prefix_match() {
        let p = ShellPolicy::new(vec!["cargo ".into(), "git status".into()], false);
        assert!(p.is_allowed("cargo test"));
        assert!(p.is_allowed("git status"));
        assert!(!p.is_allowed("curl evil"));
    }

    #[test]
    fn allowlist_block_needs_approval_without_require_all() {
        let p = ShellPolicy::new(vec!["cargo ".into()], false);
        assert!(p.needs_interactive_approval("curl evil"));
        assert!(!p.needs_interactive_approval("cargo test"));
    }
}