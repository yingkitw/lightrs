//! Per-model response dialects. The stream/non-stream parsers are lenient
//! (parts-array `content`, `reasoning_content`/`reasoning` fields, index-less
//! tool-call deltas are all accepted unconditionally); the dialect carries
//! *model-specific* policy a tolerant parser can't infer — today that's only
//! which models think inline via `<think>…</think>` inside `content`.

use crate::chat::ApiMessage;
use crate::provider::stream::StreamApply;

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

/// Routes streamed content deltas around `<think>` tags: text inside a tag
/// becomes `ReasoningDelta`, text outside stays `ContentDelta`. Tags can be
/// split across deltas (`<thi` + `nk>`), so a tail that could complete a tag
/// is held back until the next delta resolves it. Only constructed for
/// `inline_think` dialects — other models pass deltas through untouched.
#[derive(Default)]
pub struct ThinkSplitter {
    in_think: bool,
    saw_think: bool,
    buf: String,
}

impl ThinkSplitter {
    pub fn new() -> Self {
        Self {
            in_think: false,
            saw_think: false,
            buf: String::new(),
        }
    }

    /// Whether a `<think>` section was actually consumed — distinguishes
    /// "enabled, no tags seen" (post-hoc reasoning emit is still needed)
    /// from "tags split live" (emitting again would duplicate).
    pub fn saw_think(&self) -> bool {
        self.saw_think
    }

    pub fn push(&mut self, apply: StreamApply, emit: &mut dyn FnMut(StreamApply)) {
        match apply {
            StreamApply::ContentDelta(text) => {
                self.buf.push_str(&text);
                self.drain(emit);
            }
            other => emit(other),
        }
    }

    /// End of stream — whatever is buffered flushes in the current role.
    /// An unclosed think means the tail was truncated reasoning.
    pub fn finish(&mut self, emit: &mut dyn FnMut(StreamApply)) {
        let tail = std::mem::take(&mut self.buf);
        if tail.is_empty() {
            return;
        }
        let apply = if self.in_think {
            StreamApply::ReasoningDelta(tail)
        } else {
            StreamApply::ContentDelta(tail)
        };
        emit(apply);
    }

    fn drain(&mut self, emit: &mut dyn FnMut(StreamApply)) {
        loop {
            let (tag, in_think) = if self.in_think { (CLOSE, false) } else { (OPEN, true) };
            match self.buf.find(tag) {
                Some(i) => {
                    self.emit_range(0, i, self.in_think, emit);
                    self.buf.drain(..i + tag.len());
                    self.in_think = in_think;
                    self.saw_think |= in_think;
                }
                None => {
                    // Emit all but a tail that could complete the tag.
                    let hold = partial_tag_tail(&self.buf, tag);
                    let end = self.buf.len() - hold;
                    self.emit_range(0, end, self.in_think, emit);
                    self.buf.drain(..end);
                    return;
                }
            }
        }
    }

    fn emit_range(
        &self,
        start: usize,
        end: usize,
        as_reasoning: bool,
        emit: &mut dyn FnMut(StreamApply),
    ) {
        if start == end {
            return;
        }
        let text = self.buf[start..end].to_string();
        emit(if as_reasoning {
            StreamApply::ReasoningDelta(text)
        } else {
            StreamApply::ContentDelta(text)
        });
    }
}

/// Longest suffix of `buf` that is a prefix of `tag` (a tag possibly split
/// across stream deltas). ASCII tags → byte comparisons are char-safe.
fn partial_tag_tail(buf: &str, tag: &str) -> usize {
    let max = tag.len() - 1;
    (1..=max.min(buf.len()))
        .rev()
        .find(|n| buf.ends_with(&tag[..*n]))
        .unwrap_or(0)
}

/// Model-specific response policy, resolved once per client from the model
/// name. Extend fields here as provider quirks are observed, not speculated.
#[derive(Clone, Copy, Debug, Default)]
pub struct ModelDialect {
    /// Model emits reasoning inside `content` wrapped in `<think>…</think>`
    /// (DeepSeek-R1 / MiniMax-M1 style) rather than a separate
    /// `reasoning_content` field. Split it out so the thinking isn't stored
    /// or echoed back as answer text. GLM/Kimi thinking models use the
    /// `reasoning_content` field — the lenient parser covers them already.
    pub inline_think: bool,
}

impl ModelDialect {
    /// Resolve the dialect for a model id. Unknown models get the plain
    /// OpenAI dialect — never guess a stricter parse for them.
    pub fn for_model(model: &str) -> Self {
        let m = model.to_ascii_lowercase();
        let inline_think = m.contains("deepseek")
            || m.contains("minimax-m1")
            || m.contains("-r1")
            || m.contains("r1-");
        Self { inline_think }
    }

    /// Normalize a finished assistant message: for inline-think models, move
    /// `<think>` sections out of `content` into `reasoning_content`.
    /// Field-based reasoning wins — never split when the provider already
    /// separated the parts.
    pub fn adapt(&self, msg: &mut ApiMessage) {
        if !self.inline_think || msg.reasoning_content.is_some() {
            return;
        }
        let Some(text) = msg.text_content() else {
            return;
        };
        let (reasoning, rest) = split_think(&text);
        if reasoning.is_empty() {
            return;
        }
        msg.reasoning_content = Some(reasoning);
        let rest = rest.trim().to_string();
        msg.content = if rest.is_empty() {
            None
        } else {
            Some(serde_json::Value::String(rest))
        };
    }
}

/// Extract every `<think>…</think>` section as reasoning; return
/// `(reasoning, remaining_text)`. An unclosed `<think>` treats the rest as
/// reasoning (truncated thinking is still thinking).
pub fn split_think(text: &str) -> (String, String) {
    let mut reasoning = String::new();
    let mut rest = String::new();
    let mut cursor = text;
    loop {
        let Some(start) = cursor.find("<think>") else {
            rest.push_str(cursor);
            break;
        };
        rest.push_str(&cursor[..start]);
        let after = &cursor[start + "<think>".len()..];
        match after.find("</think>") {
            Some(end) => {
                push_section(&mut reasoning, &after[..end]);
                cursor = &after[end + "</think>".len()..];
            }
            None => {
                push_section(&mut reasoning, after);
                break;
            }
        }
    }
    (reasoning, rest)
}

fn push_section(out: &mut String, section: &str) {
    let section = section.trim();
    if section.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(section);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ApiRole;

    fn collect(parts: &[&str]) -> (Vec<StreamApply>, bool) {
        let mut sp = ThinkSplitter::new();
        let mut out = Vec::new();
        for p in parts {
            sp.push(StreamApply::ContentDelta(p.to_string()), &mut |a| out.push(a));
        }
        sp.finish(&mut |a| out.push(a));
        (out, sp.saw_think())
    }

    #[test]
    fn splitter_routes_around_tags_split_across_deltas() {
        // `<thi` `nk>` and `</th` `ink>` — tags split mid-delta must not leak.
        let (out, saw) = collect(&["<thi", "nk>why</th", "ink>answer"]);
        assert!(saw);
        assert_eq!(
            out,
            vec![
                StreamApply::ReasoningDelta("why".into()),
                StreamApply::ContentDelta("answer".into()),
            ]
        );
    }

    #[test]
    fn splitter_handles_text_around_think() {
        let (out, _) = collect(&["pre<think>x", "y</think>post"]);
        assert_eq!(
            out,
            vec![
                StreamApply::ContentDelta("pre".into()),
                StreamApply::ReasoningDelta("x".into()),
                StreamApply::ReasoningDelta("y".into()),
                StreamApply::ContentDelta("post".into()),
            ]
        );
    }

    #[test]
    fn splitter_passthrough_without_tags() {
        let (out, saw) = collect(&["just ", "text"]);
        assert!(!saw);
        assert_eq!(
            out,
            vec![
                StreamApply::ContentDelta("just ".into()),
                StreamApply::ContentDelta("text".into()),
            ]
        );
    }

    #[test]
    fn splitter_unclosed_think_flushes_as_reasoning() {
        let (out, saw) = collect(&["<think>trailing thought"]);
        assert!(saw);
        assert_eq!(out, vec![StreamApply::ReasoningDelta("trailing thought".into())]);
        // Truncated opener at end-of-stream flushes as content — never a
        // dangling partial tag reaching the UI.
        let (out, _) = collect(&["text<thi"]);
        assert_eq!(
            out,
            vec![
                StreamApply::ContentDelta("text".into()),
                StreamApply::ContentDelta("<thi".into()),
            ]
        );
    }

    #[test]
    fn for_model_flags_inline_think_names_only() {
        assert!(ModelDialect::for_model("deepseek-r1").inline_think);
        assert!(ModelDialect::for_model("MiniMax-M1").inline_think);
        assert!(ModelDialect::for_model("r1-distill-qwen").inline_think);
        assert!(!ModelDialect::for_model("glm-4.6").inline_think);
        assert!(!ModelDialect::for_model("kimi-for-coding").inline_think);
        assert!(!ModelDialect::for_model("unknown-x").inline_think);
    }

    #[test]
    fn split_think_extracts_sections() {
        let (r, t) = split_think("<think>plan A</think>answer");
        assert_eq!(r, "plan A");
        assert_eq!(t, "answer");
        // Multiple blocks + unclosed tail.
        let (r, t) = split_think("a<think>x</think>b<think>y");
        assert_eq!(r, "x\n\ny");
        assert_eq!(t, "ab");
        // No tags → untouched.
        let (r, t) = split_think("plain");
        assert!(r.is_empty() && t == "plain");
    }

    #[test]
    fn adapt_moves_think_to_reasoning() {
        let mut msg = ApiMessage {
            role: ApiRole::Assistant,
            content: Some(serde_json::json!("<think>why</think>the answer")),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        };
        ModelDialect {
            inline_think: true,
        }
        .adapt(&mut msg);
        assert_eq!(msg.reasoning_content.as_deref(), Some("why"));
        assert_eq!(msg.text_content().as_deref(), Some("the answer"));
        // Field-based reasoning wins — content left alone.
        let mut msg2 = ApiMessage {
            reasoning_content: Some("already".into()),
            content: Some(serde_json::json!("<think>stay</think>")),
            ..ApiMessage::user("")
        };
        ModelDialect {
            inline_think: true,
        }
        .adapt(&mut msg2);
        assert_eq!(msg2.text_content().as_deref(), Some("<think>stay</think>"));
        // Non-inline dialects never split.
        let mut msg3 = ApiMessage {
            role: ApiRole::Assistant,
            content: Some(serde_json::json!("<think>not thinking</think>")),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        };
        ModelDialect::for_model("glm-4.6").adapt(&mut msg3);
        assert!(msg3.reasoning_content.is_none());
    }
}
