#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellPolicy {
    prefixes: Vec<String>,
    require_approval: bool,
    /// `[sandbox]` — OS confinement for spawned children (seatbelt/bwrap).
    sandbox: super::sandbox::SandboxCfg,
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