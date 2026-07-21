//! Human-visual CLI rendering for the map graph: a one-line project state
//! summary plus a box-drawing dependency tree with status icons, critical-lane
//! markers, and transitive pressure counts.

use crate::model::Item;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone, Debug)]
pub(crate) struct RenderEdge {
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) kind: String,
}

const OPEN_EXCLUDED: [&str; 3] = ["closed", "closed_partial", "cancelled"];
const DIAGRAM_CONTENT_WIDTH: usize = 54;
const ANSI_RESET: &str = "\x1b[0m";

pub(crate) fn colorize_map(output: &str, enabled: bool) -> String {
    if !enabled {
        return output.to_string();
    }

    // Match once from left to right. Longer status phrases precede their bare
    // icons so verbose nodes receive one ANSI span while condensed nodes can
    // color the standalone icon without nesting escape sequences.
    let tokens = [
        ("◐ closed_partial", "32"),
        ("◐ CLOSED_PARTIAL", "32"),
        ("✓ closed", "32"),
        ("✓ CLOSED", "32"),
        ("✓ done", "32"),
        ("○ ready", "36"),
        ("○ READY", "36"),
        ("◎ picked", "33"),
        ("◎ PICKED", "33"),
        ("◉ running", "1;33"),
        ("◉ RUNNING", "1;33"),
        ("◇ in_review", "35"),
        ("◇ IN_REVIEW", "35"),
        ("◇ review", "35"),
        ("· pending", "2"),
        ("· PENDING", "2"),
        ("⊖ blocked", "31"),
        ("⊖ BLOCKED", "31"),
        ("✗ failed", "1;31"),
        ("✗ FAILED", "1;31"),
        ("⊘ cancelled", "2"),
        ("⊘ CANCELLED", "2"),
        ("blocks✓─▶", "2"),
        ("then ─▶", "2"),
        ("◐", "32"),
        ("✓", "32"),
        ("○", "36"),
        ("◎", "33"),
        ("◉", "1;33"),
        ("◇", "35"),
        ("·", "2"),
        ("⊖", "31"),
        ("✗", "1;31"),
        ("⊘", "2"),
        ("blocks─▶", "31"),
        ("blocks ─▶", "31"),
        ("hands_to─▶", "36"),
        ("hands_to ─▶", "36"),
        ("⏶", "33"),
        ("★", "1;33"),
        ("⚠ cycle:", "1;31"),
        ("WORKFLOW MAP", "1;36"),
    ];

    let mut colored = String::with_capacity(output.len());
    let mut remaining = output;
    while !remaining.is_empty() {
        let offset = output.len() - remaining.len();
        if let Some((token, sgr)) = tokens.iter().find(|(token, _)| {
            remaining.starts_with(token)
                && (!is_bare_status_icon(token) || is_condensed_status_position(output, offset))
        }) {
            colored.push_str(&format!("\x1b[{sgr}m{token}{ANSI_RESET}"));
            remaining = &remaining[token.len()..];
            continue;
        }

        let character = remaining.chars().next().expect("remaining is not empty");
        colored.push(character);
        remaining = &remaining[character.len_utf8()..];
    }
    colored
}

fn is_bare_status_icon(token: &str) -> bool {
    matches!(
        token,
        "◐" | "✓" | "○" | "◎" | "◉" | "◇" | "·" | "⊖" | "✗" | "⊘"
    )
}

fn is_condensed_status_position(output: &str, offset: usize) -> bool {
    let line_start = output[..offset].rfind('\n').map_or(0, |index| index + 1);
    if output[line_start..offset].trim_start() != "│ " || line_start == 0 {
        return false;
    }

    let previous_end = line_start - 1;
    let previous_start = output[..previous_end]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let previous = output[previous_start..previous_end].trim_start();
    previous.starts_with('┌') && previous.ends_with('┐')
}

pub(crate) fn status_icon(status: &str) -> &'static str {
    match status {
        "closed" => "✓",
        "closed_partial" => "◐",
        "running" => "◉",
        "picked" => "◎",
        "in_review" => "◇",
        "ready" => "○",
        "pending" => "·",
        "blocked" => "⊖",
        "failed" => "✗",
        "cancelled" => "⊘",
        _ => "?",
    }
}

pub(crate) fn render_map(
    project: &str,
    items: &[Item],
    edges: &[RenderEdge],
    critical: &HashSet<String>,
    cycles: &[Vec<String>],
) -> String {
    let mut out = String::new();
    out.push_str(&state_line(project, items));
    if items.is_empty() {
        out.push_str("\n(no items)");
        return out;
    }
    out.push('\n');
    out.push('\n');
    out.push_str(&render_tree(items, edges, critical));
    for cycle in cycles {
        out.push_str(&format!("\n⚠ cycle: {}", cycle.join(" → ")));
    }
    out
}

/// A deliberately human-first companion to the compact tree renderer.
///
/// The graph projection is identical to `render_map`; only presentation
/// changes. Nodes stay in deterministic dependency-tree order, while boxes,
/// route labels, and join references make the shape easier to supervise in a
/// terminal without introducing a second graph/layout dependency.
pub(crate) fn render_diagram_map(
    project: &str,
    items: &[Item],
    edges: &[RenderEdge],
    critical: &HashSet<String>,
    cycles: &[Vec<String>],
    full: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("── {project} · WORKFLOW MAP {}\n", "─".repeat(32)));
    out.push_str(&state_line(project, items));
    out.push_str("\n\n");
    out.push_str("legend  ○ ready  · pending  ◎ picked  ◉ running  ◇ review  ✓ done\n");
    out.push_str("        ⊖ blocked  ✗ failed  ⊘ cancelled\n");
    out.push_str("        ★ critical  ⏶ downstream  blocks ─▶ active dependency\n");
    out.push_str("        then ─▶ satisfied dependency  hands_to ─▶ handoff\n");
    if items.is_empty() {
        out.push_str("\n(no items)");
        return out;
    }

    out.push('\n');
    out.push_str(&render_diagram_tree(items, edges, critical, full));
    for cycle in cycles {
        out.push_str(&format!("\n⚠ cycle: {}", cycle.join(" → ")));
    }
    out.trim_end().to_string()
}

fn state_line(project: &str, items: &[Item]) -> String {
    let total = items.len();
    let count = |statuses: &[&str]| {
        items
            .iter()
            .filter(|item| statuses.contains(&item.status.as_str()))
            .count()
    };
    let done = count(&["closed", "closed_partial"]);
    let percent = (done * 100).checked_div(total).unwrap_or(0);
    let mut line = format!(
        "{project}: {done}/{total} done ({percent}%) | ready {} | active {} | in_review {} | blocked {}",
        count(&["ready"]),
        count(&["picked", "running"]),
        count(&["in_review"]),
        count(&["pending", "blocked"]),
    );
    let failed = count(&["failed"]);
    if failed > 0 {
        line.push_str(&format!(" | failed {failed}"));
    }
    let cancelled = count(&["cancelled"]);
    if cancelled > 0 {
        line.push_str(&format!(" | cancelled {cancelled}"));
    }
    line
}

fn render_tree(items: &[Item], edges: &[RenderEdge], critical: &HashSet<String>) -> String {
    let known = items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let edges = edges
        .iter()
        .filter(|edge| known.contains(edge.from.as_str()) && known.contains(edge.to.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let by_id = items
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<HashMap<_, _>>();
    let mut outgoing: BTreeMap<&str, Vec<&RenderEdge>> = BTreeMap::new();
    let mut incoming: HashSet<&str> = HashSet::new();
    let mut seen_edges = HashSet::new();
    for edge in &edges {
        if !seen_edges.insert((edge.from.as_str(), edge.to.as_str(), edge.kind.as_str())) {
            continue;
        }
        outgoing.entry(edge.from.as_str()).or_default().push(edge);
        incoming.insert(edge.to.as_str());
    }
    for children in outgoing.values_mut() {
        children.sort_by(|a, b| a.to.cmp(&b.to).then_with(|| a.kind.cmp(&b.kind)));
    }
    let pressure = pressure_counts(items, &edges);

    let mut roots = items
        .iter()
        .filter(|item| !incoming.contains(item.id.as_str()))
        .collect::<Vec<_>>();
    roots.sort_by_key(|item| std::cmp::Reverse(item.priority));

    let mut out = String::new();
    let mut printed = HashSet::new();
    for root in &roots {
        render_node(
            &root.id,
            None,
            "",
            "",
            &by_id,
            &outgoing,
            critical,
            &pressure,
            &mut printed,
            &mut out,
        );
    }
    // Items unreachable from any root (e.g. every node in a cycle has an
    // incoming edge) still need to appear once.
    for item in items {
        if !printed.contains(item.id.as_str()) {
            render_node(
                &item.id,
                None,
                "",
                "",
                &by_id,
                &outgoing,
                critical,
                &pressure,
                &mut printed,
                &mut out,
            );
        }
    }
    out
}

fn render_diagram_tree(
    items: &[Item],
    edges: &[RenderEdge],
    critical: &HashSet<String>,
    full: bool,
) -> String {
    let known = items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let edges = edges
        .iter()
        .filter(|edge| known.contains(edge.from.as_str()) && known.contains(edge.to.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let by_id = items
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<HashMap<_, _>>();
    let mut outgoing: BTreeMap<&str, Vec<&RenderEdge>> = BTreeMap::new();
    let mut incoming: HashSet<&str> = HashSet::new();
    let mut seen_edges = HashSet::new();
    for edge in &edges {
        if !seen_edges.insert((edge.from.as_str(), edge.to.as_str(), edge.kind.as_str())) {
            continue;
        }
        outgoing.entry(edge.from.as_str()).or_default().push(edge);
        incoming.insert(edge.to.as_str());
    }
    for children in outgoing.values_mut() {
        children.sort_by(|a, b| a.to.cmp(&b.to).then_with(|| a.kind.cmp(&b.kind)));
    }
    let mut undirected: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &edges {
        undirected
            .entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
        undirected
            .entry(edge.to.as_str())
            .or_default()
            .push(edge.from.as_str());
    }

    let pressure = pressure_counts(items, &edges);
    let mut roots = items
        .iter()
        .filter(|item| !incoming.contains(item.id.as_str()))
        .collect::<Vec<_>>();
    roots.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut out = String::new();
    let mut printed = HashSet::new();
    let mut assigned_to_component = HashSet::new();
    let mut component = 0;
    let seeds = roots
        .iter()
        .map(|item| item.id.as_str())
        .chain(items.iter().map(|item| item.id.as_str()))
        .collect::<Vec<_>>();
    for seed in seeds {
        if assigned_to_component.contains(seed) {
            continue;
        }
        let mut component_ids = HashSet::new();
        let mut stack = vec![seed];
        while let Some(id) = stack.pop() {
            if !component_ids.insert(id) {
                continue;
            }
            assigned_to_component.insert(id);
            if let Some(neighbors) = undirected.get(id) {
                stack.extend(neighbors.iter().copied());
            }
        }

        component += 1;
        if component > 1 {
            out.push('\n');
        }
        out.push_str(&format!("component {component}\n\n"));
        let component_roots = roots
            .iter()
            .filter(|root| component_ids.contains(root.id.as_str()))
            .collect::<Vec<_>>();
        for (index, root) in component_roots.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            render_diagram_node(
                &root.id,
                None,
                "",
                true,
                &by_id,
                &outgoing,
                critical,
                &pressure,
                &mut printed,
                &mut out,
                full,
            );
        }

        // Rootless cycle components and any node not reachable following edge
        // direction still need a deterministic entry point.
        for item in items {
            if !component_ids.contains(item.id.as_str()) || printed.contains(item.id.as_str()) {
                continue;
            }
            if !component_roots.is_empty() || item.id.as_str() != seed {
                out.push('\n');
            }
            render_diagram_node(
                &item.id,
                None,
                "",
                true,
                &by_id,
                &outgoing,
                critical,
                &pressure,
                &mut printed,
                &mut out,
                full,
            );
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn render_diagram_node(
    id: &str,
    incoming_edge: Option<&RenderEdge>,
    prefix: &str,
    is_last: bool,
    by_id: &HashMap<&str, &Item>,
    outgoing: &BTreeMap<&str, Vec<&RenderEdge>>,
    critical: &HashSet<String>,
    pressure: &HashMap<String, usize>,
    printed: &mut HashSet<String>,
    out: &mut String,
    full: bool,
) {
    let Some(item) = by_id.get(id) else {
        return;
    };
    let repeat = !printed.insert(id.to_string());
    let node_prefix = if let Some(edge) = incoming_edge {
        let connector = if is_last { "└─" } else { "├─" };
        let label = edge_display_label(edge, by_id, EdgeSurface::Diagram);
        out.push_str(&format!("{prefix}{connector} {label} ─▶\n"));
        format!("{prefix}{}  ", if is_last { " " } else { "│" })
    } else {
        prefix.to_string()
    };

    for line in diagram_box_lines(item, critical.contains(id), pressure.get(id), repeat, full) {
        out.push_str(&node_prefix);
        out.push_str(&line);
        out.push('\n');
    }
    if repeat {
        return;
    }

    let children = outgoing.get(id).cloned().unwrap_or_default();
    if children.is_empty() {
        return;
    }
    out.push_str(&node_prefix);
    out.push_str("│\n");
    for (index, edge) in children.iter().enumerate() {
        render_diagram_node(
            &edge.to,
            Some(edge),
            &node_prefix,
            index == children.len() - 1,
            by_id,
            outgoing,
            critical,
            pressure,
            printed,
            out,
            full,
        );
    }
}

fn diagram_box_lines(
    item: &Item,
    is_critical: bool,
    pressure: Option<&usize>,
    repeat: bool,
    full: bool,
) -> Vec<String> {
    let content = if full {
        full_diagram_content(item, is_critical, pressure, repeat)
    } else {
        condensed_diagram_content(item, repeat)
    };

    let mut lines = vec![format!("┌{}┐", "─".repeat(DIAGRAM_CONTENT_WIDTH + 2))];
    for line in content {
        for wrapped in wrap_diagram_text(&line, DIAGRAM_CONTENT_WIDTH) {
            lines.push(format!("│ {} │", pad_diagram_line(&wrapped)));
        }
    }
    lines.push(format!("└{}┘", "─".repeat(DIAGRAM_CONTENT_WIDTH + 2)));
    lines
}

fn full_diagram_content(
    item: &Item,
    is_critical: bool,
    pressure: Option<&usize>,
    repeat: bool,
) -> Vec<String> {
    let mut status = format!(
        "{} {}",
        status_icon(item.status.as_str()),
        item.status.as_str().to_ascii_uppercase()
    );
    if is_critical {
        status.push_str(" · ★ critical");
    }
    if let Some(count) = pressure.filter(|count| **count > 0) {
        status.push_str(&format!(" · ⏶{count} downstream"));
    }

    let mut content = vec![status, item.id.clone()];
    if repeat {
        content.push("↳ joins a node already shown above".to_string());
    } else {
        content.extend(wrap_diagram_text(&item.title, DIAGRAM_CONTENT_WIDTH));
        if let Some(worker) = &item.worker_id {
            if matches!(item.status.as_str(), "picked" | "running") {
                content.push(format!("worker: {worker}"));
            }
        }
    }

    content
}

fn condensed_diagram_content(item: &Item, repeat: bool) -> Vec<String> {
    let repeat_marker = if repeat { " ↳ above" } else { "" };
    let summary = format!(
        "{} {} → {}{}",
        status_icon(item.status.as_str()),
        item.id,
        item.title,
        repeat_marker
    );
    wrap_diagram_text_limited(&summary, DIAGRAM_CONTENT_WIDTH, 2)
}

fn wrap_diagram_text(value: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in value.split_whitespace() {
        if word.chars().count() > width {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            let chars = word.chars().collect::<Vec<_>>();
            for chunk in chars.chunks(width) {
                lines.push(chunk.iter().collect());
            }
            continue;
        }
        let separator = usize::from(!line.is_empty());
        if line.chars().count() + separator + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn wrap_diagram_text_limited(value: &str, width: usize, max_lines: usize) -> Vec<String> {
    let mut lines = wrap_diagram_text(value, width);
    if lines.len() <= max_lines {
        return lines;
    }

    lines.truncate(max_lines);
    if let Some(last) = lines.last_mut() {
        let keep = width.saturating_sub(1);
        let mut truncated = last.chars().take(keep).collect::<String>();
        truncated.push('…');
        *last = truncated;
    }
    lines
}

fn pad_diagram_line(value: &str) -> String {
    let length = value.chars().count();
    format!(
        "{value}{}",
        " ".repeat(DIAGRAM_CONTENT_WIDTH.saturating_sub(length))
    )
}

#[allow(clippy::too_many_arguments)]
fn render_node(
    id: &str,
    incoming_edge: Option<&RenderEdge>,
    prefix: &str,
    connector: &str,
    by_id: &HashMap<&str, &Item>,
    outgoing: &BTreeMap<&str, Vec<&RenderEdge>>,
    critical: &HashSet<String>,
    pressure: &HashMap<String, usize>,
    printed: &mut HashSet<String>,
    out: &mut String,
) {
    let Some(item) = by_id.get(id) else {
        return;
    };
    let repeat = !printed.insert(id.to_string());
    let mut line = format!("{prefix}{connector}");
    if let Some(edge) = incoming_edge {
        let label = edge_display_label(edge, by_id, EdgeSurface::Tree);
        line.push_str(&format!("{label}─▶ "));
    }
    line.push_str(&format!(
        "{} {} {} {}",
        status_icon(item.status.as_str()),
        item.status,
        item.id,
        item.title
    ));
    if let Some(worker) = &item.worker_id {
        if matches!(item.status.as_str(), "picked" | "running") {
            line.push_str(&format!(" ({worker})"));
        }
    }
    if critical.contains(id) {
        line.push_str(" ★");
    }
    if let Some(count) = pressure.get(id) {
        if *count > 0 {
            line.push_str(&format!(" ⏶{count}"));
        }
    }
    if repeat {
        line.push_str(" (see above)");
    }
    out.push_str(&line);
    out.push('\n');
    if repeat {
        return;
    }
    let children = outgoing.get(id).cloned().unwrap_or_default();
    let child_prefix = match connector {
        "" => prefix.to_string(),
        "└─" => format!("{prefix}   "),
        _ => format!("{prefix}│  "),
    };
    for (index, edge) in children.iter().enumerate() {
        let child_connector = if index == children.len() - 1 {
            "└─"
        } else {
            "├─"
        };
        render_node(
            &edge.to,
            Some(edge),
            &child_prefix,
            child_connector,
            by_id,
            outgoing,
            critical,
            pressure,
            printed,
            out,
        );
    }
}

#[derive(Clone, Copy)]
enum EdgeSurface {
    Tree,
    Diagram,
}

fn edge_display_label<'a>(
    edge: &'a RenderEdge,
    by_id: &HashMap<&str, &Item>,
    surface: EdgeSurface,
) -> &'a str {
    let satisfied = edge.kind == "blocks"
        && by_id
            .get(edge.from.as_str())
            .is_some_and(|item| matches!(item.status.as_str(), "closed" | "closed_partial"));
    if !satisfied {
        return edge.kind.as_str();
    }

    match surface {
        EdgeSurface::Tree => "blocks✓",
        EdgeSurface::Diagram => "then",
    }
}

/// Transitive count of open items blocked by each open item, matching the
/// active-item semantics of `App::pressure`.
fn pressure_counts(items: &[Item], edges: &[RenderEdge]) -> HashMap<String, usize> {
    let open = items
        .iter()
        .filter(|item| !OPEN_EXCLUDED.contains(&item.status.as_str()))
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let mut downstream: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in edges {
        if open.contains(edge.from.as_str()) && open.contains(edge.to.as_str()) {
            downstream
                .entry(edge.from.as_str())
                .or_default()
                .push(edge.to.as_str());
        }
    }
    let mut counts = HashMap::new();
    for id in &open {
        let mut seen = BTreeSet::new();
        let mut stack = downstream.get(id).cloned().unwrap_or_default();
        while let Some(next) = stack.pop() {
            if next != *id && seen.insert(next) {
                if let Some(children) = downstream.get(next) {
                    stack.extend(children.iter().copied());
                }
            }
        }
        counts.insert((*id).to_string(), seen.len());
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ItemStatus, WorkType};

    fn item(id: &str, title: &str, status: &str) -> Item {
        Item {
            id: id.to_string(),
            project_id: "prj_test".to_string(),
            parent_item_id: None,
            title: title.to_string(),
            description: String::new(),
            status: ItemStatus::try_from(status).expect("valid test status"),
            work_type: WorkType::Generic,
            priority: 0,
            worker_id: None,
            plan_path: None,
        }
    }

    fn edge(from: &str, to: &str, kind: &str) -> RenderEdge {
        RenderEdge {
            from: from.to_string(),
            to: to.to_string(),
            kind: kind.to_string(),
        }
    }

    #[test]
    fn renders_linear_chain_as_nested_tree() {
        let items = vec![
            item("itm_a", "First", "running"),
            item("itm_b", "Second", "ready"),
            item("itm_c", "Third", "pending"),
        ];
        let edges = vec![
            edge("itm_a", "itm_b", "blocks"),
            edge("itm_b", "itm_c", "blocks"),
        ];
        let out = render_map("demo", &items, &edges, &HashSet::new(), &[]);
        assert!(
            out.starts_with("demo: 0/3 done (0%) | ready 1 | active 1 | in_review 0 | blocked 1")
        );
        assert!(out.contains("◉ running itm_a First ⏶2"));
        assert!(out.contains("└─blocks─▶ ○ ready itm_b Second ⏶1"));
        assert!(out.contains("   └─blocks─▶ · pending itm_c Third"));
    }

    #[test]
    fn diamond_renders_shared_node_once_with_repeat_marker() {
        let items = vec![
            item("itm_a", "Left", "ready"),
            item("itm_b", "Right", "ready"),
            item("itm_c", "Join", "pending"),
        ];
        let edges = vec![
            edge("itm_a", "itm_c", "blocks"),
            edge("itm_b", "itm_c", "blocks"),
        ];
        let out = render_map("demo", &items, &edges, &HashSet::new(), &[]);
        assert_eq!(out.matches("itm_c Join").count(), 2);
        assert_eq!(out.matches("(see above)").count(), 1);
    }

    #[test]
    fn cycle_does_not_hang_and_is_reported() {
        let items = vec![
            item("itm_a", "One", "ready"),
            item("itm_b", "Two", "pending"),
        ];
        let edges = vec![
            edge("itm_a", "itm_b", "blocks"),
            edge("itm_b", "itm_a", "blocks"),
        ];
        let cycles = vec![vec![
            "itm_a".to_string(),
            "itm_b".to_string(),
            "itm_a".to_string(),
        ]];
        let out = render_map("demo", &items, &edges, &HashSet::new(), &cycles);
        assert!(out.contains("itm_a One"));
        assert!(out.contains("itm_b Two"));
        assert!(out.contains("⚠ cycle: itm_a → itm_b → itm_a"));
    }

    #[test]
    fn critical_marker_worker_and_status_icons() {
        let mut running = item("itm_a", "Hot path", "running");
        running.worker_id = Some("worker-7".to_string());
        let items = vec![
            running,
            item("itm_b", "Reviewing", "in_review"),
            item("itm_c", "Shipped", "closed"),
        ];
        let edges = vec![edge("itm_a", "itm_b", "hands_to")];
        let critical = ["itm_a".to_string(), "itm_b".to_string()]
            .into_iter()
            .collect::<HashSet<_>>();
        let out = render_map("demo", &items, &edges, &critical, &[]);
        assert!(out.contains("◉ running itm_a Hot path (worker-7) ★ ⏶1"));
        assert!(out.contains("└─hands_to─▶ ◇ in_review itm_b Reviewing ★"));
        assert!(out.contains("✓ closed itm_c Shipped"));
        assert!(out.contains("1/3 done (33%)"));
    }

    #[test]
    fn closed_items_carry_no_pressure() {
        let items = vec![
            item("itm_a", "Done", "closed"),
            item("itm_b", "Next", "ready"),
        ];
        let edges = vec![edge("itm_a", "itm_b", "blocks")];
        let out = render_map("demo", &items, &edges, &HashSet::new(), &[]);
        assert!(!out.contains("itm_a Done ⏶"));
        assert!(out.contains("blocks✓─▶"), "{out}");
    }

    #[test]
    fn edge_labels_distinguish_satisfied_from_active_without_changing_kind() {
        for (status, tree, diagram) in [
            ("closed", "blocks✓", "then"),
            ("closed_partial", "blocks✓", "then"),
            ("ready", "blocks", "blocks"),
            ("failed", "blocks", "blocks"),
            ("cancelled", "blocks", "blocks"),
        ] {
            let source = item("source", "Source", status);
            let by_id = [(source.id.as_str(), &source)].into_iter().collect();
            let edge = edge("source", "target", "blocks");
            assert_eq!(edge.kind, "blocks");
            assert_eq!(
                edge_display_label(&edge, &by_id, EdgeSurface::Tree),
                tree,
                "tree label for {status}"
            );
            assert_eq!(
                edge_display_label(&edge, &by_id, EdgeSurface::Diagram),
                diagram,
                "diagram label for {status}"
            );
        }

        let source = item("source", "Source", "closed");
        let by_id = [(source.id.as_str(), &source)].into_iter().collect();
        let handoff = edge("source", "target", "hands_to");
        assert_eq!(
            edge_display_label(&handoff, &by_id, EdgeSurface::Tree),
            "hands_to"
        );
        assert_eq!(
            edge_display_label(&handoff, &by_id, EdgeSurface::Diagram),
            "hands_to"
        );
    }

    #[test]
    fn empty_map_renders_placeholder() {
        let out = render_map("demo", &[], &[], &HashSet::new(), &[]);
        assert!(out.contains("(no items)"));
        assert!(out.contains("0/0 done (0%)"));
    }

    #[test]
    fn diagram_groups_shared_roots_and_separates_truly_disconnected_nodes() {
        let items = vec![
            item("itm_a", "Root A", "ready"),
            item("itm_b", "Root B", "ready"),
            item("itm_join", "Shared join", "pending"),
            item("itm_detached", "Detached", "ready"),
        ];
        let edges = vec![
            edge("itm_a", "itm_join", "blocks"),
            edge("itm_b", "itm_join", "hands_to"),
        ];

        let out = render_diagram_map("demo", &items, &edges, &HashSet::new(), &[], true);

        assert_eq!(out.matches("component ").count(), 2, "{out}");
        assert!(out.contains("blocks ─▶"), "{out}");
        assert!(out.contains("hands_to ─▶"), "{out}");
        assert!(out.contains("↳ joins a node already shown above"), "{out}");
        assert!(out.contains("itm_detached"), "{out}");
    }

    #[test]
    fn diagram_renders_linear_flow_with_operational_annotations() {
        let mut root = item("itm_root", "Root work", "running");
        root.worker_id = Some("worker-7".to_string());
        let items = vec![root, item("itm_leaf", "Leaf work", "pending")];
        let edges = vec![edge("itm_root", "itm_leaf", "blocks")];
        let critical = ["itm_root".to_string(), "itm_leaf".to_string()]
            .into_iter()
            .collect::<HashSet<_>>();

        let out = render_diagram_map("demo", &items, &edges, &critical, &[], true);

        assert!(out.contains("── demo · WORKFLOW MAP"), "{out}");
        assert!(
            out.contains("◉ RUNNING · ★ critical · ⏶1 downstream"),
            "{out}"
        );
        assert!(out.contains("worker: worker-7"), "{out}");
        assert!(out.contains("└─ blocks ─▶"), "{out}");
        assert!(out.contains("· PENDING · ★ critical"), "{out}");
    }

    #[test]
    fn diagram_empty_map_keeps_summary_and_placeholder() {
        let out = render_diagram_map("demo", &[], &[], &HashSet::new(), &[], false);

        assert!(out.contains("demo: 0/0 done (0%)"), "{out}");
        assert!(out.contains("(no items)"), "{out}");
        assert!(!out.contains("component 1"), "{out}");
    }

    #[test]
    fn diagram_cycle_terminates_and_reports_the_route() {
        let items = vec![
            item("itm_a", "One", "ready"),
            item("itm_b", "Two", "pending"),
        ];
        let edges = vec![
            edge("itm_a", "itm_b", "blocks"),
            edge("itm_b", "itm_a", "blocks"),
        ];
        let cycles = vec![vec![
            "itm_a".to_string(),
            "itm_b".to_string(),
            "itm_a".to_string(),
        ]];

        let out = render_diagram_map("demo", &items, &edges, &HashSet::new(), &cycles, true);

        assert_eq!(out.matches("component ").count(), 1, "{out}");
        assert_eq!(out.matches("│ itm_a").count(), 2, "{out}");
        assert!(out.contains("↳ joins a node already shown above"), "{out}");
        assert!(out.contains("⚠ cycle: itm_a → itm_b → itm_a"), "{out}");
    }

    #[test]
    fn condensed_diagram_uses_icon_id_arrow_title_with_two_line_limit() {
        let item = item(
            "i-a-realistically-long-planr-item-id-1234",
            "A deliberately long title that must wrap and then truncate before a third content line can make the graph too tall",
            "closed",
        );

        let lines = diagram_box_lines(&item, true, Some(&9), false, false);
        let content = &lines[1..lines.len() - 1];

        assert_eq!(content.len(), 2, "{lines:#?}");
        assert!(content[0].contains("✓ i-a-realistically-long-planr-item-id-1234 →"));
        assert!(content[1].contains('…'), "{lines:#?}");
        assert!(!lines.join("\n").contains("CLOSED"));
        assert!(!lines.join("\n").contains("critical"));
        assert!(!lines.join("\n").contains("downstream"));
    }

    #[test]
    fn full_diagram_preserves_verbose_node_details() {
        let mut item = item("itm_a", "Full title", "running");
        item.worker_id = Some("worker-7".to_string());

        let output = diagram_box_lines(&item, true, Some(&2), false, true).join("\n");

        assert!(output.contains("◉ RUNNING · ★ critical · ⏶2 downstream"));
        assert!(output.contains("worker: worker-7"));
    }

    #[test]
    fn colorizes_condensed_icons_without_nesting_verbose_status_spans() {
        let condensed = colorize_map(
            "── demo · WORKFLOW MAP\n┌──┐\n│ ✓ itm_a → Done · title ◉ │\n└──┘\n   ┌──┐\n   │ · itm_b → Waiting ✓ │\n   └──┘\nlegend  ○ ready\n└─ blocks ─▶",
            true,
        );
        assert!(condensed.contains("\x1b[32m✓\x1b[0m itm_a"));
        assert!(condensed.contains("\x1b[2m·\x1b[0m itm_b"));
        assert!(condensed.contains("demo · \x1b[1;36mWORKFLOW MAP\x1b[0m"));
        assert!(condensed.contains("Done · title ◉"));
        assert!(condensed.contains("Waiting ✓"));
        assert!(condensed.contains("\x1b[36m○ ready\x1b[0m"));
        assert!(condensed.contains("\x1b[31mblocks ─▶\x1b[0m"));

        let edges = colorize_map("blocks─▶\nblocks✓─▶\nblocks ─▶\nthen ─▶", true);
        assert_eq!(
            edges,
            "\x1b[31mblocks─▶\x1b[0m\n\x1b[2mblocks✓─▶\x1b[0m\n\x1b[31mblocks ─▶\x1b[0m\n\x1b[2mthen ─▶\x1b[0m"
        );

        let verbose = colorize_map("│ ✓ CLOSED · ★ critical · ⏶2 downstream │\n◉ RUNNING", true);
        assert_eq!(
            verbose,
            "│ \x1b[32m✓ CLOSED\x1b[0m · \x1b[1;33m★\x1b[0m critical · \x1b[33m⏶\x1b[0m2 downstream │\n\x1b[1;33m◉ RUNNING\x1b[0m"
        );

        let full_title = item("itm_full", "✓ title glyph stays plain", "closed");
        let full_box = colorize_map(
            &diagram_box_lines(&full_title, false, None, false, true).join("\n"),
            true,
        );
        assert!(
            full_box.contains("│ ✓ title glyph stays plain"),
            "{full_box:?}"
        );

        let long_id = "i".repeat(49);
        let wrapped_title = item(&long_id, "✓ wrapped glyph stays plain", "pending");
        let compact_box = colorize_map(
            &diagram_box_lines(&wrapped_title, false, None, false, false).join("\n"),
            true,
        );
        assert!(compact_box.contains("\x1b[2m·\x1b[0m"), "{compact_box:?}");
        assert!(
            compact_box.contains("│ ✓ wrapped glyph stays plain"),
            "{compact_box:?}"
        );
    }
}
