//! Formal busy-turn spec (integration): public API invariants after observe-style work.
//!
//! In-crate table tests live in `tui/run/events.rs` and `tui/run/observe.rs`.

use std::path::Path;
use std::sync::Arc;

use lightrs::agent::Agent;
use lightrs::provider::LlmClient;
use lightrs::tools::ShellPolicy;

#[test]
fn streaming_assistant_chars_zero_on_fresh_agent() {
    let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
    let agent = Agent::new(client, Arc::from(Path::new(".")), ShellPolicy::default());
    assert_eq!(agent.streaming_assistant_chars(), 0);
}