//! Heuristic read-only shell classification for auto-approve under `require_approval`.

use super::shell_deny;

/// When `require_approval` is on, skip the TUI prompt for likely read-only commands.
pub fn auto_approve_read_only(command: &str) -> bool {
    if shell_deny::deny_reason(command).is_some() {
        return false;
    }
    if has_shell_redirection(command) {
        return false;
    }
    for segment in split_segments(command) {
        if segment.is_empty() {
            continue;
        }
        if !segment_is_read_only(&segment) {
            return false;
        }
    }
    true
}

fn has_shell_redirection(command: &str) -> bool {
    let c = command;
    c.contains('>') || c.contains("<<") || c.contains(">>")
}

fn split_segments(command: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in command.chars() {
        if ch == '|' || ch == ';' {
            parts.push(cur.trim().to_string());
            cur.clear();
        } else if ch == '&' {
            let rest = cur.trim();
            if rest.is_empty() {
                parts.push(String::new());
            } else {
                parts.push(rest.to_string());
            }
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        parts.push(cur.trim().to_string());
    }
    parts
}

fn segment_is_read_only(segment: &str) -> bool {
    let mut s = segment.trim();
    while let Some((next, remainder)) = peel_env_assignment(s) {
        if next.is_empty() && remainder.is_empty() {
            return true;
        }
        s = remainder;
    }
    let Some(bin) = first_token(s) else {
        return true;
    };
    let lower = bin.to_ascii_lowercase();
    match lower.as_str() {
        "ls" | "dir" | "cat" | "head" | "tail" | "less" | "more" | "wc" | "pwd" | "which"
        | "whereis" | "whoami" | "id" | "echo" | "printf" | "true" | "false" | "env"
        | "printenv" | "date" | "uname" | "hostname" | "uptime" | "ps" | "top" | "du" | "df"
        | "file" | "stat" | "diff" | "rg" | "grep" | "ag" | "fd" => true,
        "find" => !segment_contains_write_find(s),
        "git" => git_segment_read_only(s),
        "cargo" => cargo_segment_read_only(s),
        "npm" | "pnpm" | "yarn" | "bun" => false,
        "rm" | "mv" | "cp" | "chmod" | "chown" | "touch" | "mkdir" | "rmdir" | "truncate"
        | "tee" | "sed" | "vi" | "vim" | "nano" | "emacs" | "curl" | "wget" | "python"
        | "python3" | "node" | "ruby" | "perl" | "sh" | "bash" | "zsh" | "fish" => false,
        _ => false,
    }
}

fn peel_env_assignment(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    let eq = s.find('=')?;
    let key = &s[..eq];
    if key.is_empty() || key.contains(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        return None;
    }
    let after = s[eq + 1..].trim_start();
    let value_end = after
        .find(' ')
        .unwrap_or(after.len());
    let rest = after[value_end..].trim_start();
    Some((&s[..eq + 1 + value_end], rest))
}

fn first_token(s: &str) -> Option<&str> {
    s.split_whitespace().next()
}

fn segment_contains_write_find(segment: &str) -> bool {
    let lower = segment.to_ascii_lowercase();
    lower.contains("-delete") || lower.contains("-exec")
}

fn git_segment_read_only(segment: &str) -> bool {
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    let Some(sub) = tokens.get(1) else {
        return true;
    };
    let sub = sub.to_ascii_lowercase();
    matches!(
        sub.as_str(),
        "status"
            | "log"
            | "diff"
            | "show"
            | "branch"
            | "rev-parse"
            | "describe"
            | "tag"
            | "remote"
            | "fetch"
            | "ls-files"
            | "ls-tree"
            | "blame"
            | "shortlog"
            | "whatchanged"
    ) || (sub == "stash" && tokens.get(2).is_some_and(|t| *t == "list"))
}

fn cargo_segment_read_only(segment: &str) -> bool {
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    let Some(sub) = tokens.get(1) else {
        return true;
    };
    matches!(
        *sub,
        "check" | "build" | "test" | "clippy" | "fmt" | "metadata" | "tree" | "doc" | "--version"
            | "-V"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approves_ls() {
        assert!(auto_approve_read_only("ls -la"));
    }

    #[test]
    fn rejects_rm() {
        assert!(!auto_approve_read_only("rm foo.txt"));
    }

    #[test]
    fn approves_git_status() {
        assert!(auto_approve_read_only("git status"));
    }

    #[test]
    fn rejects_git_push() {
        assert!(!auto_approve_read_only("git push"));
    }

    #[test]
    fn rejects_redirection() {
        assert!(!auto_approve_read_only("echo hi > out.txt"));
    }

    #[test]
    fn pipe_all_read_only() {
        assert!(auto_approve_read_only("git status | head"));
    }

    #[test]
    fn env_assignment_prefix_still_read_only() {
        assert!(auto_approve_read_only("FOO=bar ls -la"));
        assert!(auto_approve_read_only("A=1 B=2 git status"));
        assert!(auto_approve_read_only("FOO=1"));
    }

    #[test]
    fn env_prefix_does_not_launder_writes() {
        assert!(!auto_approve_read_only("FOO=bar rm x"));
        assert!(!auto_approve_read_only("PATH=/x sh -c id"));
    }

    #[test]
    fn and_chain_must_be_all_read_only() {
        assert!(auto_approve_read_only("ls && pwd"));
        assert!(!auto_approve_read_only("ls && rm x"));
        assert!(!auto_approve_read_only("ls; sh -c 'id'"));
    }

    #[test]
    fn find_is_read_only_without_write_actions() {
        assert!(auto_approve_read_only("find . -name '*.rs'"));
        assert!(!auto_approve_read_only("find . -name x -delete"));
        assert!(!auto_approve_read_only("find . -exec rm {} +"));
    }

    #[test]
    fn git_subcommand_table() {
        for ok in [
            "git",
            "git status",
            "git log --oneline",
            "git diff HEAD",
            "git show",
            "git branch -a",
            "git blame f.rs",
            "git stash list",
            "git fetch origin",
        ] {
            assert!(auto_approve_read_only(ok), "expected approve: {ok}");
        }
        for no in [
            "git push",
            "git commit -m x",
            "git stash pop",
            "git checkout main",
            "git rebase -i HEAD~2",
            "git clean -fd",
        ] {
            assert!(!auto_approve_read_only(no), "expected reject: {no}");
        }
    }

    #[test]
    fn cargo_subcommand_table() {
        for ok in ["cargo", "cargo check", "cargo test", "cargo fmt", "cargo tree"] {
            assert!(auto_approve_read_only(ok), "expected approve: {ok}");
        }
        for no in ["cargo publish", "cargo install x", "cargo run"] {
            assert!(!auto_approve_read_only(no), "expected reject: {no}");
        }
    }

    #[test]
    fn interpreters_and_package_managers_rejected() {
        for cmd in ["npm run build", "pnpm i", "python3 x.py", "node a.js", "sh -c ls", "bash s.sh"] {
            assert!(!auto_approve_read_only(cmd), "expected reject: {cmd}");
        }
    }

    #[test]
    fn append_and_here_doc_rejected() {
        assert!(!auto_approve_read_only("echo x >> f"));
        assert!(!auto_approve_read_only("cat << EOF"));
        assert!(!auto_approve_read_only("cmd 2>&1"));
    }
}