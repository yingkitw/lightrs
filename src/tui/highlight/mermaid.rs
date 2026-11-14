use std::collections::HashMap;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::tui::theme::{self, palette};

use super::wrap::wrap_plain_lines;

#[derive(Debug, Clone)]
struct Edge {
    from: String,
    to: String,
    label: Option<String>,
    arrow: &'static str,
}

#[derive(Debug, Clone)]
struct SeqMsg {
    from: String,
    to: String,
    label: String,
    dashed: bool,
}

/// Turn common Mermaid sources into TUI-friendly ASCII (flowchart / sequence).
pub fn render_mermaid(source: &str, width: usize) -> Vec<Line<'static>> {
    let view = interpret(source);
    let header_style = Style::default()
        .fg(palette::MERMAID_ACCENT)
        .add_modifier(Modifier::BOLD);
    let edge_style = Style::default().fg(palette::MERMAID_EDGE);
    let note_style = theme::body_muted();

    let mut out = Vec::new();
    out.push(Line::from(Span::styled(
        format!("── mermaid ({}) ──", view.kind),
        header_style,
    )));

    if view.body.is_empty() {
        out.push(Line::from(Span::styled(
            "(no diagram edges; see source below)",
            note_style,
        )));
    } else {
        for row in &view.body {
            out.extend(wrap_plain_lines(row, width, edge_style));
        }
    }

    out.push(Line::from(Span::styled("source:", note_style)));
    let code_style = theme::code_block();
    out.extend(wrap_plain_lines(
        source.trim_end_matches('\n'),
        width,
        code_style,
    ));
    out.push(Line::from(""));
    out
}

struct MermaidView {
    kind: &'static str,
    body: Vec<String>,
}

fn interpret(source: &str) -> MermaidView {
    let lines: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%"))
        .collect();

    let kind = lines
        .first()
        .map(|l| diagram_kind(l))
        .unwrap_or("diagram");

    let mut labels = HashMap::new();
    let mut edges = Vec::new();
    let mut seq = Vec::new();

    for line in &lines {
        collect_labels(line, &mut labels);
        if let Some(e) = parse_edge(line) {
            edges.push(e);
        }
        if let Some(m) = parse_sequence(line) {
            seq.push(m);
        }
    }

    let body = if kind == "sequence" && !seq.is_empty() {
        format_sequence(&seq, &labels)
    } else if !edges.is_empty() {
        format_flowchart(&edges, &labels)
    } else {
        fallback_lines(&lines, kind)
    };

    MermaidView { kind, body }
}

fn diagram_kind(header: &str) -> &'static str {
    let h = header.to_lowercase();
    if h.starts_with("sequencediagram") {
        "sequence"
    } else if h.starts_with("flowchart") || h.starts_with("graph ") {
        "flowchart"
    } else if h.starts_with("statediagram") {
        "state"
    } else if h.starts_with("classdiagram") {
        "class"
    } else if h.starts_with("erdiagram") {
        "er"
    } else if h.starts_with("gantt") {
        "gantt"
    } else {
        "diagram"
    }
}

fn fallback_lines(lines: &[&str], kind: &'static str) -> Vec<String> {
    if kind == "gantt" || kind == "class" {
        return lines
            .iter()
            .skip(1)
            .take(12)
            .map(|l| format!("  {}", abbrev_line(l, 72)))
            .collect();
    }
    Vec::new()
}

fn abbrev_line(s: &str, max: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let head: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

fn collect_labels(line: &str, map: &mut HashMap<String, String>) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        while i < chars.len() && is_id_char(chars[i]) {
            i += 1;
        }
        if i > start {
            let id: String = chars[start..i].iter().collect();
            if i < chars.len() {
                let open = chars[i];
                if open == '[' || open == '(' || open == '{' {
                    let close = match open {
                        '[' => ']',
                        '(' => ')',
                        '{' => '}',
                        _ => continue,
                    };
                    i += 1;
                    let body_start = i;
                    while i < chars.len() && chars[i] != close {
                        i += 1;
                    }
                    if i < chars.len() {
                        let label: String = chars[body_start..i].iter().collect();
                        if !label.is_empty() {
                            map.insert(id, label.trim().to_string());
                        }
                        i += 1;
                    }
                }
            }
        } else {
            i += 1;
        }
    }
}

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

const ARROWS: &[(&str, &str)] = &[
    ("-.->", "··►"),
    ("==>", "═►"),
    ("-->", "──►"),
    ("--", "──"),
    ("->>", "──►"),
    ("->", "──►"),
];

fn parse_edge(line: &str) -> Option<Edge> {
    for (token, glyph) in ARROWS {
        if let Some(pos) = line.find(token) {
            let left = line[..pos].trim();
            let mut rest = line[pos + token.len()..].trim();
            let mut label = None;
            if rest.starts_with('|')
                && let Some(end) = rest[1..].find('|') {
                    label = Some(rest[1..1 + end].trim().to_string());
                    rest = rest[1 + end + 1..].trim();
                }
            let from = extract_node_id(left)?;
            let to = extract_node_id(rest)?;
            if from.is_empty() || to.is_empty() {
                return None;
            }
            return Some(Edge {
                from,
                to,
                label,
                arrow: glyph,
            });
        }
    }
    None
}

fn extract_node_id(segment: &str) -> Option<String> {
    let segment = segment.trim();
    if segment.is_empty() {
        return None;
    }
    let mut id = String::new();
    for c in segment.chars() {
        if is_id_char(c) {
            id.push(c);
        } else if !id.is_empty() {
            break;
        }
    }
    if id.is_empty() {
        let token = segment.split_whitespace().next()?.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '_'
        });
        if token.is_empty() {
            None
        } else {
            Some(token.to_string())
        }
    } else {
        Some(id)
    }
}

fn parse_sequence(line: &str) -> Option<SeqMsg> {
    let line = line.trim();
    let (token, dashed) = if line.contains("->>") {
        ("->>", false)
    } else if line.contains("-->>") {
        ("-->>", true)
    } else if line.contains("-->") {
        ("-->", true)
    } else {
        return None;
    };
    let pos = line.find(token)?;
    let left = line[..pos].trim();
    let rest = line[pos + token.len()..].trim();
    let (to, label) = if let Some(colon) = rest.find(':') {
        (rest[..colon].trim(), rest[colon + 1..].trim())
    } else {
        (rest, "")
    };
    let from = extract_node_id(left)?;
    let to = extract_node_id(to)?;
    Some(SeqMsg {
        from,
        to,
        label: label.to_string(),
        dashed,
    })
}

fn display_node(id: &str, labels: &HashMap<String, String>) -> String {
    labels
        .get(id)
        .map(|t| format!("[{t}]"))
        .unwrap_or_else(|| id.to_string())
}

fn format_flowchart(edges: &[Edge], labels: &HashMap<String, String>) -> Vec<String> {
    edges
        .iter()
        .map(|e| {
            let from = display_node(&e.from, labels);
            let to = display_node(&e.to, labels);
            if let Some(lbl) = &e.label {
                if lbl.is_empty() {
                    format!("  {from} {} {to}", e.arrow)
                } else {
                    format!("  {from} {} {lbl} {} {to}", e.arrow, e.arrow)
                }
            } else {
                format!("  {from} {} {to}", e.arrow)
            }
        })
        .collect()
}

fn format_sequence(msgs: &[SeqMsg], labels: &HashMap<String, String>) -> Vec<String> {
    msgs.iter()
        .map(|m| {
            let from = display_node(&m.from, labels);
            let to = display_node(&m.to, labels);
            let arrow = if m.dashed { "···►" } else { "──►" };
            if m.label.is_empty() {
                format!("  {from} {arrow} {to}")
            } else {
                format!("  {from} {arrow} {to}: {}", m.label)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flowchart_edges() {
        let src = "flowchart TD\n    A[Start] --> B[End]";
        let v = interpret(src);
        assert_eq!(v.kind, "flowchart");
        assert_eq!(v.body.len(), 1);
        assert!(v.body[0].contains("Start"));
        assert!(v.body[0].contains("End"));
    }

    #[test]
    fn sequence_message() {
        let src = "sequenceDiagram\n    Alice->>Bob: hi";
        let v = interpret(src);
        assert_eq!(v.kind, "sequence");
        assert_eq!(v.body.len(), 1);
        assert!(v.body[0].contains("Alice"));
        assert!(v.body[0].contains("hi"));
    }
}