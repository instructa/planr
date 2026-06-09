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

#[allow(clippy::too_many_arguments)]
fn render_node(
    id: &str,
    edge_kind: Option<&str>,
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
    if let Some(kind) = edge_kind {
        line.push_str(&format!("{kind}─▶ "));
    }
    line.push_str(&format!(
        "{} {} {} {}",
        status_icon(&item.status),
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
            Some(&edge.kind),
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

    fn item(id: &str, title: &str, status: &str) -> Item {
        Item {
            id: id.to_string(),
            project_id: "prj_test".to_string(),
            parent_item_id: None,
            title: title.to_string(),
            description: String::new(),
            status: status.to_string(),
            work_type: "generic".to_string(),
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
        let edges = vec![edge("itm_a", "itm_b", "feeds_into")];
        let critical = ["itm_a".to_string(), "itm_b".to_string()]
            .into_iter()
            .collect::<HashSet<_>>();
        let out = render_map("demo", &items, &edges, &critical, &[]);
        assert!(out.contains("◉ running itm_a Hot path (worker-7) ★ ⏶1"));
        assert!(out.contains("└─feeds_into─▶ ◇ in_review itm_b Reviewing ★"));
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
    }

    #[test]
    fn empty_map_renders_placeholder() {
        let out = render_map("demo", &[], &[], &HashSet::new(), &[]);
        assert!(out.contains("(no items)"));
        assert!(out.contains("0/0 done (0%)"));
    }
}
