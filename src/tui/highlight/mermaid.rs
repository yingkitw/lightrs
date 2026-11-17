use std::collections::HashMap;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::tui::theme;

use super::wrap::wrap_code_lines;

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
    let view = interpret(source, width);
    let header_style = Style::default()
        .fg(crate::tui::theme::active().mermaid_accent)
        .add_modifier(Modifier::BOLD);
    let edge_style = Style::default().fg(crate::tui::theme::active().mermaid_edge);
    let note_style = theme::body_muted();

    let mut out = Vec::new();
    if view.pre_wrapped {
        // Solid panel: deepest bg for the layout area, deep-grey bg for node
        // boxes (via `node_mask`), padded to a common block width so the whole
        // diagram reads as one framed rectangle.
        let p = theme::active();
        let panel = Style::default().bg(p.code_bg);
        let styles = [
            edge_style.bg(p.code_bg),                        // edges, on panel
            Style::default().fg(p.muted).bg(p.inline_code_bg), // box borders
            Style::default().fg(p.text).bg(p.inline_code_bg),  // box labels
        ];
        let block_w = view
            .body
            .iter()
            .map(|r| r.chars().count())
            .max()
            .unwrap_or(0);
        let header = format!("── mermaid ({}) ", view.kind);
        out.push(Line::from(vec![
            Span::styled(header, header_style.bg(p.code_bg)),
            Span::styled(
                " ".repeat(block_w.saturating_sub(view.kind.len() + 13)),
                panel,
            ),
        ]));
        for (y, row) in view.body.iter().enumerate() {
            let mask = view.node_mask.get(y).map_or(&[][..], Vec::as_slice);
            out.push(Line::from(segment_row(row, block_w, mask, styles, panel)));
        }
    } else {
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
                out.extend(wrap_code_lines(
                    vec![Line::styled(row.clone(), edge_style)],
                    width,
                ));
            }
        }
    }

    out.push(Line::from(Span::styled("source:", note_style)));
    let code_style = theme::code_block();
    out.extend(wrap_code_lines(
        source
            .trim_end_matches('\n')
            .split('\n')
            .map(|l| Line::styled(l.to_string(), code_style))
            .collect(),
        width,
    ));
    out.push(Line::from(""));
    out
}

/// Split a diagram row into styled runs by cell kind (0 = edge/space,
/// 1 = box border, 2 = box label); the row is padded to `block_w` with
/// `panel` so the block is a solid rectangle regardless of trailing space.
fn segment_row(
    row: &str,
    block_w: usize,
    mask: &[u8],
    styles: [Style; 3],
    panel: Style,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut run = String::new();
    let mut kind = 0u8;
    let mut x = 0usize;
    for c in row.chars() {
        let cell = mask.get(x).copied().unwrap_or(0);
        if cell != kind && !run.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut run), styles[kind as usize]));
        }
        kind = cell;
        run.push(c);
        x += 1;
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, styles[kind as usize]));
    }
    if x < block_w {
        spans.push(Span::styled(" ".repeat(block_w - x), panel));
    }
    spans
}

struct MermaidView {
    kind: &'static str,
    body: Vec<String>,
    /// Rows already fitted to `width` (ASCII diagram) — skip word-wrap.
    pre_wrapped: bool,
    /// Per-row node-box cells for background segmentation (laid-out diagrams).
    node_mask: Vec<Vec<u8>>,
}

fn interpret(source: &str, width: usize) -> MermaidView {
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

    let (body, pre_wrapped, node_mask) = if kind == "sequence" && !seq.is_empty() {
        (format_sequence(&seq, &labels), false, Vec::new())
    } else if !edges.is_empty() {
        let dir = lines.first().and_then(|l| flowchart_direction(l));
        match layout_flowchart(&edges, &labels, dir, width) {
            Some(d) => (d.rows, true, d.mask),
            None => (flat_flowchart(&edges, &labels), false, Vec::new()),
        }
    } else {
        (fallback_lines(&lines, kind), false, Vec::new())
    };

    MermaidView {
        kind,
        body,
        pre_wrapped,
        node_mask,
    }
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

/// Per-edge fallback when a real layout isn't possible (cycles, BT/RL,
/// over-wide graphs, sequence-only syntax).
fn flat_flowchart(edges: &[Edge], labels: &HashMap<String, String>) -> Vec<String> {
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

// ── Box layout ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Dir {
    Td,
    Lr,
}

/// `flowchart TD` / `graph LR` direction. `None` → flat list (BT/RL and
/// diagram kinds we don't lay out).
fn flowchart_direction(header: &str) -> Option<Dir> {
    let mut parts = header.split_whitespace();
    let kw = parts.next()?;
    if !(kw.eq_ignore_ascii_case("flowchart") || kw.eq_ignore_ascii_case("graph")) {
        return None;
    }
    match parts.next().map(|s| s.to_ascii_uppercase()).as_deref() {
        None | Some("TD") | Some("TB") => Some(Dir::Td),
        Some("LR") => Some(Dir::Lr),
        _ => None,
    }
}

const MAX_NODES: usize = 48;
const MAX_ROWS: usize = 120;
const GAP: usize = 3; // TD: horizontal gap between sibling boxes
const VGAP: usize = 1; // LR: vertical gap between sibling boxes
const CONN_W: usize = 4; // LR: connector column width

/// Char grid + a `blocked` mask for box cells. Line glyphs write only into
/// unblocked cells holding a blank or another line glyph — a routed edge may
/// pass *behind* a box but never through its border or label.
struct Canvas {
    cells: Vec<Vec<char>>,
    /// Per-cell kind: 0 = layout area/edges, 1 = box border, 2 = box label.
    kind: Vec<Vec<u8>>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Self {
            cells: vec![vec![' '; w]; h],
            kind: vec![vec![0; w]; h],
        }
    }

    fn get(&self, x: usize, y: usize) -> char {
        self.cells
            .get(y)
            .and_then(|r| r.get(x))
            .copied()
            .unwrap_or(' ')
    }

    fn put(&mut self, x: usize, y: usize, c: char) {
        if let Some(cell) = self.cells.get_mut(y).and_then(|r| r.get_mut(x)) {
            *cell = c;
        }
    }

    fn put_line(&mut self, x: usize, y: usize, c: char) {
        if self
            .kind
            .get(y)
            .and_then(|r| r.get(x))
            .copied()
            .unwrap_or(1)
            > 0
        {
            return;
        }
        let cur = self.get(x, y);
        if cur == ' ' || LINE_GLYPHS.contains(cur) {
            self.put(x, y, c);
        }
    }

    fn draw_box(&mut self, x: usize, y: usize, w: usize, text: &str) {
        for row in self.kind.iter_mut().skip(y).take(3) {
            for cell in row.iter_mut().skip(x).take(w) {
                *cell = 1;
            }
        }
        self.put(x, y, '┌');
        self.put(x + w - 1, y, '┐');
        self.put(x, y + 2, '└');
        self.put(x + w - 1, y + 2, '┘');
        for i in 1..w - 1 {
            self.put(x + i, y, '─');
            self.put(x + i, y + 2, '─');
        }
        self.put(x, y + 1, '│');
        self.put(x + w - 1, y + 1, '│');
        for (i, ch) in text.chars().enumerate().take(w.saturating_sub(3)) {
            self.put(x + 2 + i, y + 1, ch);
            self.kind[y + 1][x + 2 + i] = 2;
        }
    }

    /// Merge a line cell into a junction glyph from the directions its
    /// neighbors occupy, so overlapping fan-out becomes `┬`/`├`/`┼`.
    fn junction(&mut self, x: usize, y: usize) {
        if self.kind[y][x] > 0 {
            return;
        }
        let cur = self.get(x, y);
        if cur != ' ' && cur != '─' && cur != '│' {
            return;
        }
        let bits = u8::from(is_vertish(self.get(x, y.wrapping_sub(1))))
            | u8::from(is_horizish(self.get(x + 1, y))) << 1
            | u8::from(is_vertish(self.get(x, y + 1))) << 2
            | u8::from(is_horizish(self.get(x.wrapping_sub(1), y))) << 3;
        if cur == ' ' && bits.count_ones() < 2 {
            return; // a lone neighbor must not spawn a dangling stub
        }
        let glyph = JUNCTIONS[bits as usize];
        if glyph != ' ' {
            self.put(x, y, glyph);
        }
    }

    /// Write `text` over the longest `─` run inside `[lo, hi]` on row `y`,
    /// keeping one dash of margin on each side.
    fn edge_label(&mut self, y: usize, lo: usize, hi: usize, text: &str) {
        let need = text.chars().count() + 2;
        let mut run_start = 0usize;
        let mut run_len = 0usize;
        for x in lo..=hi {
            if self.get(x, y) == '─' {
                if run_len == 0 {
                    run_start = x;
                }
                run_len += 1;
                if run_len >= need {
                    let start = run_start + (run_len - text.chars().count()) / 2;
                    for (i, ch) in text.chars().enumerate() {
                        self.put(start + i, y, ch);
                    }
                    return;
                }
            } else {
                run_len = 0;
            }
        }
    }

    /// Text rows + per-cell kind mask for background segmentation.
    fn into_rows(self) -> (Vec<String>, Vec<Vec<u8>>) {
        let rows = self
            .cells
            .into_iter()
            .map(|r| r.into_iter().collect::<String>().trim_end().to_string())
            .collect();
        (rows, self.kind)
    }
}

const LINE_GLYPHS: &str = "│─▼▲►◄┬┴├┤┼";
const VERTISH: &str = "│├┤┬┴┼▼▲";
const HORIZISH: &str = "─┌┐└┘├┤┬┴┼►◄";

fn is_vertish(c: char) -> bool {
    VERTISH.contains(c)
}

fn is_horizish(c: char) -> bool {
    HORIZISH.contains(c)
}

// bits: U=1 R=2 D=4 L=8
const JUNCTIONS: [char; 16] = [
    ' ', '│', '─', '└', '│', '│', '┌', '├', '─', '┘', '─', '┴', '┐', '┤', '┬', '┼',
];

/// Layered box layout for `flowchart TD`/`LR`; `None` → flat edge list.
fn layout_flowchart(
    edges: &[Edge],
    labels: &HashMap<String, String>,
    dir: Option<Dir>,
    width: usize,
) -> Option<Diagram> {
    let dir = dir?;
    let mut ids: Vec<&str> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for e in edges {
        for id in [&e.from, &e.to] {
            if let std::collections::hash_map::Entry::Vacant(en) = index.entry(id.as_str()) {
                en.insert(ids.len());
                ids.push(id.as_str());
            }
        }
    }
    let n = ids.len();
    if n == 0 || n > MAX_NODES || width < 24 {
        return None;
    }
    let mut parents: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in edges {
        children[index[e.from.as_str()]].push(index[e.to.as_str()]);
        parents[index[e.to.as_str()]].push(index[e.from.as_str()]);
    }

    // Longest-path ranks (Kahn). A cycle leaves nodes unprocessed → flat list.
    let mut rank = vec![0usize; n];
    let mut indeg: Vec<usize> = parents.iter().map(Vec::len).collect();
    let mut queue: Vec<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
    let mut head = 0;
    while head < queue.len() {
        let u = queue[head];
        head += 1;
        for &v in &children[u] {
            rank[v] = rank[v].max(rank[u] + 1);
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push(v);
            }
        }
    }
    if queue.len() != n {
        return None;
    }

    let n_layers = rank.iter().max()? + 1;
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); n_layers];
    for (v, &r) in rank.iter().enumerate() {
        layers[r].push(v);
    }

    let text_of = |v: usize| -> &str { labels.get(ids[v]).map_or(ids[v], String::as_str) };
    // TD packs boxes side-by-side (slot = box width); LR stacks them
    // vertically (slot = box height).
    let slot = |v: usize| -> usize {
        if dir == Dir::Td {
            text_of(v).chars().count() + 4
        } else {
            3
        }
    };

    // Stack-axis position (TD: x, LR: y); within a layer, nodes are ordered
    // by parent barycenter to reduce edge crossings.
    let mut origin = vec![0usize; n];
    let mut center = vec![0usize; n];
    let gap = if dir == Dir::Td { GAP } else { VGAP };
    for (l, layer) in layers.iter_mut().enumerate() {
        if l > 0 {
            layer.sort_by_key(|&v| {
                if parents[v].is_empty() {
                    usize::MAX
                } else {
                    parents[v].iter().map(|&p| center[p]).sum::<usize>() / parents[v].len()
                }
            });
        }
        let mut cursor = 0usize;
        for &v in layer.iter() {
            let w = slot(v);
            let desired = if parents[v].is_empty() {
                cursor + w / 2
            } else {
                parents[v].iter().map(|&p| center[p]).sum::<usize>() / parents[v].len()
            };
            let o = desired.saturating_sub(w / 2).max(cursor);
            origin[v] = o;
            center[v] = o + w / 2;
            cursor = o + w + gap;
        }
    }

    if dir == Dir::Td {
        draw_td(
            &layers, &rank, &index, &ids, edges, labels, &origin, &center, slot, width,
        )
    } else {
        draw_lr(
            &layers, &rank, &index, &ids, edges, labels, &origin, &center, width,
        )
    }
    .map(|(rows, mask)| Diagram {
        rows,
        mask,
    })
}

/// Laid-out diagram: text rows plus, per row, which cells are node boxes.
struct Diagram {
    rows: Vec<String>,
    mask: Vec<Vec<u8>>,
}

fn label_of<'a>(id: &'a str, labels: &'a HashMap<String, String>) -> &'a str {
    labels.get(id).map_or(id, String::as_str)
}

#[allow(clippy::too_many_arguments)]
fn draw_td(
    layers: &[Vec<usize>],
    rank: &[usize],
    index: &HashMap<&str, usize>,
    ids: &[&str],
    edges: &[Edge],
    labels: &HashMap<String, String>,
    origin: &[usize],
    center: &[usize],
    slot: impl Fn(usize) -> usize,
    width: usize,
) -> Option<(Vec<String>, Vec<Vec<u8>>)> {
    let layer_top = |l: usize| l * 5; // 3 box rows + 2 connector rows
    let total_w = layers
        .iter()
        .flatten()
        .map(|&v| origin[v] + slot(v))
        .max()?;
    if total_w > width {
        return None;
    }
    let mut c = Canvas::new(total_w, layer_top(layers.len() - 1) + 3);
    for (l, layer) in layers.iter().enumerate() {
        for &v in layer {
            c.draw_box(origin[v], layer_top(l), slot(v), label_of(ids[v], labels));
        }
    }
    // Drop `│` under each parent, run `─` on the row above each child, `▼`
    // into the child's top border. The junction pass merges overlapping runs.
    let mut arrowheads = Vec::new();
    let mut edge_labels = Vec::new();
    for e in edges {
        let (u, v) = (index[e.from.as_str()], index[e.to.as_str()]);
        let (cx_u, cx_v) = (center[u], center[v]);
        let r_arr = layer_top(rank[v]) - 1;
        c.put(cx_u, layer_top(rank[u]) + 2, '┴');
        for y in layer_top(rank[u]) + 3..r_arr {
            c.put_line(cx_u, y, '│');
        }
        let (lo, hi) = (cx_u.min(cx_v), cx_u.max(cx_v));
        for x in lo..=hi {
            c.put_line(x, r_arr, '─');
        }
        arrowheads.push((cx_v, r_arr));
        if let Some(lbl) = e.label.as_deref().filter(|l| !l.is_empty()) {
            edge_labels.push((r_arr, lo, hi, lbl));
        }
    }
    resolve_junctions(&mut c);
    for (y, lo, hi, lbl) in edge_labels {
        c.edge_label(y, lo, hi, lbl);
    }
    for (x, y) in arrowheads {
        c.put(x, y, '▼');
    }
    Some(c.into_rows())
}

#[allow(clippy::too_many_arguments)]
fn draw_lr(
    layers: &[Vec<usize>],
    rank: &[usize],
    index: &HashMap<&str, usize>,
    ids: &[&str],
    edges: &[Edge],
    labels: &HashMap<String, String>,
    origin: &[usize],
    center: &[usize],
    width: usize,
) -> Option<(Vec<String>, Vec<Vec<u8>>)> {
    // Columns = layers; each column is as wide as its widest box.
    let node_w = |v: usize| label_of(ids[v], labels).chars().count() + 4;
    let col_w: Vec<usize> = layers
        .iter()
        .map(|l| l.iter().map(|&v| node_w(v)).max().unwrap_or(4))
        .collect();
    let mut col_x = Vec::with_capacity(layers.len());
    let mut x = 0usize;
    for (l, w) in col_w.iter().enumerate() {
        col_x.push(x);
        x += w + if l + 1 == layers.len() { 0 } else { CONN_W };
    }
    let total_w = *col_x.last()? + col_w.last()?;
    let total_h = layers
        .iter()
        .filter_map(|l| l.last().map(|&v| origin[v] + 3))
        .max()?;
    if total_w > width || total_h > MAX_ROWS {
        return None;
    }
    let node_x = |v: usize| col_x[rank[v]] + (col_w[rank[v]] - node_w(v)) / 2;

    let mut c = Canvas::new(total_w, total_h);
    for layer in layers {
        for &v in layer {
            c.draw_box(node_x(v), origin[v], node_w(v), label_of(ids[v], labels));
        }
    }
    let mut arrowheads = Vec::new();
    let mut edge_labels = Vec::new();
    for e in edges {
        let (u, v) = (index[e.from.as_str()], index[e.to.as_str()]);
        let (cy_u, cy_v) = (center[u], center[v]);
        let right_u = node_x(u) + node_w(u);
        let left_v = node_x(v);
        let x_ch = left_v.saturating_sub(2); // channel two cells left of child
        c.put(right_u - 1, cy_u, '├');
        for x in right_u..=x_ch {
            c.put_line(x, cy_u, '─');
        }
        for y in cy_u.min(cy_v)..=cy_u.max(cy_v) {
            c.put_line(x_ch, y, '│');
        }
        for x in x_ch + 1..left_v.saturating_sub(1) {
            c.put_line(x, cy_v, '─');
        }
        arrowheads.push((left_v - 1, cy_v));
        if let Some(lbl) = e.label.as_deref().filter(|l| !l.is_empty()) {
            edge_labels.push((cy_u, right_u, x_ch, lbl));
        }
    }
    resolve_junctions(&mut c);
    for (y, lo, hi, lbl) in edge_labels {
        c.edge_label(y, lo, hi, lbl);
    }
    for (x, y) in arrowheads {
        c.put(x, y, '►');
    }
    Some(c.into_rows())
}

fn resolve_junctions(c: &mut Canvas) {
    for y in 0..c.cells.len() {
        for x in 0..c.cells[y].len() {
            c.junction(x, y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flowchart_edges() {
        let src = "flowchart TD\n    A[Start] --> B[End]";
        let v = interpret(src, 80);
        assert_eq!(v.kind, "flowchart");
        assert!(!v.body.is_empty());
        let text = v.body.join("\n");
        assert!(text.contains("Start"));
        assert!(text.contains("End"));
    }

    #[test]
    fn sequence_message() {
        let src = "sequenceDiagram\n    Alice->>Bob: hi";
        let v = interpret(src, 80);
        assert_eq!(v.kind, "sequence");
        assert_eq!(v.body.len(), 1);
        assert!(v.body[0].contains("Alice"));
        assert!(v.body[0].contains("hi"));
    }

    fn flat_text(view: &MermaidView) -> String {
        view.body.join("\n")
    }

    #[test]
    fn td_chain_renders_boxes_and_arrow() {
        let v = interpret("flowchart TD\n    A[Start] --> B[End]", 80);
        let text = flat_text(&v);
        assert!(v.pre_wrapped, "layout should own the body");
        assert!(text.contains("┌───────┐"), "{text}");
        assert!(text.contains("│ Start │"), "{text}");
        assert!(text.contains("▼"), "{text}");
        assert!(text.contains("│ End │"), "{text}");
    }

    #[test]
    fn td_fanout_merges_junction() {
        let src = "flowchart TD\n    A[Root] --> B[Left]\n    A --> C[Right]";
        let text = flat_text(&interpret(src, 80));
        assert!(text.contains("▼"), "{text}");
        assert!(text.contains("│ Left │") && text.contains("│ Right │"), "{text}");
    }

    #[test]
    fn lr_chain_renders_side_by_side() {
        let text = flat_text(&interpret("flowchart LR\n    A[In] --> B[Out]", 80));
        assert!(text.contains("├") && text.contains("►"), "{text}");
        // Both boxes on the same three rows.
        assert!(text.contains("│ In ├") && text.contains("│ Out │"), "{text}");
        assert!(!text.contains("▼"), "LR should not draw vertical arrows: {text}");
    }

    #[test]
    fn cycle_falls_back_to_flat_list() {
        let v = interpret("flowchart TD\n    A --> B\n    B --> A", 80);
        assert!(!v.pre_wrapped, "cycle must not lay out boxes");
        assert!(flat_text(&v).contains("A ──► B"));
    }

    #[test]
    fn over_width_graph_falls_back() {
        let v = interpret("flowchart TD\n    A[Start] --> B[End]", 10);
        assert!(!v.pre_wrapped, "narrow terminal should get the flat list");
    }

    #[test]
    fn panel_backgrounds_segment_nodes_from_edges() {
        let lines = render_mermaid("flowchart TD\n    A[Start] --> B[End]", 80);
        let p = theme::active();
        let mut saw_node = false;
        let mut saw_edge = false;
        for line in &lines {
            for span in &line.spans {
                if span.style.bg.is_none() {
                    continue; // the `source:` dump is unstyled
                }
                let text: &str = span.content.as_ref();
                if text.contains("Start") || text.contains("End") {
                    assert_eq!(
                        span.style.bg,
                        Some(p.inline_code_bg),
                        "node box should use the deep-grey fill: {span:?}"
                    );
                    saw_node = true;
                } else if text.contains('▼') {
                    assert_eq!(
                        span.style.bg,
                        Some(p.code_bg),
                        "edge cells should sit on the panel bg: {span:?}"
                    );
                    saw_edge = true;
                }
            }
        }
        assert!(saw_node && saw_edge);
    }

    #[test]
    fn edge_label_drawn_on_span() {
        // A long node name pushes the child right, giving the edge span room.
        let v = interpret(
            "flowchart TD\n    A[S] -->|ok| B[ThisIsAReallyLongLabelForB]",
            80,
        );
        assert!(v.pre_wrapped);
        let text = flat_text(&v);
        assert!(text.contains("ok"), "{text}");
    }

    /// Doc mermaid fences must render through our own grammar — a fence that
    /// falls back to flat edges or echo means the syntax drifted past what
    /// the TUI can lay out.
    #[test]
    fn docs_mermaid_fences_all_render() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = vec![root.join("ARCHITECTURE.md")];
        if let Ok(rd) = std::fs::read_dir(root.join("docs")) {
            files.extend(
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "md")),
            );
        }
        let mut fences = 0;
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            for block in text.split("```mermaid").skip(1) {
                fences += 1;
                let src = block.split("```").next().unwrap().trim();
                let v = interpret(src, 100);
                assert!(!v.body.is_empty(), "{}: empty render\n{src}", f.display());
                assert_ne!(
                    v.kind,
                    "diagram",
                    "{}: unrecognized diagram kind\n{src}",
                    f.display()
                );
                if v.kind == "flowchart" {
                    assert!(
                        v.pre_wrapped,
                        "{}: flowchart fell back to flat edges\n{src}",
                        f.display()
                    );
                }
            }
        }
        assert!(fences >= 2, "ARCHITECTURE.md mermaid fences should exist");
    }
}
