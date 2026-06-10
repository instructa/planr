use super::App;
use crate::model::Item;
use crate::storage::row_to_item;
use crate::util::collect_rows;
use anyhow::{bail, Result};
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone, Debug)]
pub(crate) struct GraphEdge {
    from: String,
    to: String,
    kind: String,
}

#[derive(Clone, Debug)]
pub(crate) struct CloseEffect {
    pub(crate) would_unlock: Vec<Item>,
    pub(crate) would_remain_blocked: Vec<Value>,
}

impl App {
    pub(crate) fn graph_status_value(&self) -> Result<Value> {
        let cycles = self.graph_cycles()?;
        Ok(json!({
            "critical": if cycles.is_empty() { self.critical_lane()? } else { Vec::<Item>::new() },
            "pressure": self.pressure()?,
            "cycles": cycles,
        }))
    }

    pub(crate) fn close_effect(&self, item_id: &str) -> Result<CloseEffect> {
        let mut would_unlock = Vec::new();
        let mut would_remain_blocked = Vec::new();
        let targets = self.direct_downstream_items(item_id)?;
        for target in targets {
            let remaining = self.blocking_items_for_excluding(&target.id, item_id)?;
            if remaining.is_empty() {
                would_unlock.push(target);
            } else {
                would_remain_blocked.push(json!({
                    "item": target,
                    "remaining_blockers": remaining,
                }));
            }
        }
        Ok(CloseEffect {
            would_unlock,
            would_remain_blocked,
        })
    }

    pub(crate) fn critical_lane(&self) -> Result<Vec<Item>> {
        let items = self.active_items()?;
        let edges = self.active_blocking_edges()?;
        let cycles = cycles_for(&items, &edges);
        if !cycles.is_empty() {
            bail!("cycle detected in map graph: {}", cycles[0].join(" -> "));
        }
        let item_by_id = items
            .iter()
            .cloned()
            .map(|item| (item.id.clone(), item))
            .collect::<HashMap<_, _>>();
        let downstream = downstream_map(&edges);
        let order = items
            .iter()
            .enumerate()
            .map(|(index, item)| (item.id.clone(), index))
            .collect::<HashMap<_, _>>();
        let mut memo: HashMap<String, Vec<String>> = HashMap::new();
        let mut best: Vec<String> = Vec::new();
        for item in &items {
            let candidate = longest_path_from(&item.id, &downstream, &mut memo);
            if path_is_better(&candidate, &best, &item_by_id, &order) {
                best = candidate;
            }
        }
        best.iter()
            .map(|id| {
                item_by_id
                    .get(id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("critical path referenced missing item {id}"))
            })
            .collect()
    }

    pub(crate) fn pressure(&self) -> Result<Vec<Value>> {
        let items = self.active_items()?;
        let edges = self.active_blocking_edges()?;
        let item_by_id = items
            .iter()
            .cloned()
            .map(|item| (item.id.clone(), item))
            .collect::<HashMap<_, _>>();
        let downstream = downstream_map(&edges);
        let mut rows = Vec::new();
        for item in &items {
            let direct = downstream.get(&item.id).cloned().unwrap_or_default();
            let reachable = reachable_from(&item.id, &downstream);
            let mut downstream_items = reachable
                .iter()
                .filter_map(|id| item_by_id.get(id).cloned())
                .collect::<Vec<_>>();
            downstream_items.sort_by(|a, b| a.id.cmp(&b.id));
            rows.push(json!({
                "id": item.id,
                "title": item.title,
                "item": item,
                "blocks": reachable.len(),
                "transitive_blocks": reachable.len(),
                "direct_blocks": direct.len(),
                "downstream": downstream_items,
            }));
        }
        rows.sort_by(|a, b| {
            let a_transitive = a["transitive_blocks"].as_u64().unwrap_or(0);
            let b_transitive = b["transitive_blocks"].as_u64().unwrap_or(0);
            let a_direct = a["direct_blocks"].as_u64().unwrap_or(0);
            let b_direct = b["direct_blocks"].as_u64().unwrap_or(0);
            b_transitive
                .cmp(&a_transitive)
                .then_with(|| b_direct.cmp(&a_direct))
                .then_with(|| {
                    a["id"]
                        .as_str()
                        .unwrap_or_default()
                        .cmp(b["id"].as_str().unwrap_or_default())
                })
        });
        rows.truncate(10);
        Ok(rows)
    }

    pub(crate) fn graph_cycles(&self) -> Result<Vec<Vec<String>>> {
        let items = self.active_items()?;
        let edges = self.active_blocking_edges()?;
        Ok(cycles_for(&items, &edges))
    }

    fn active_items(&self) -> Result<Vec<Item>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, parent_item_id, title, description, status, work_type, priority, worker_id, plan_path
             FROM items WHERE status NOT IN ('closed','closed_partial','cancelled') ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], row_to_item)?;
        collect_rows(rows)
    }

    fn active_blocking_edges(&self) -> Result<Vec<GraphEdge>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.from_item, l.to_item, l.kind
             FROM links l
             JOIN items upstream ON upstream.id = l.from_item
             JOIN items target ON target.id = l.to_item
             WHERE l.kind IN ('blocks','hands_to')
             AND upstream.status NOT IN ('closed','closed_partial','cancelled')
             AND target.status NOT IN ('closed','closed_partial','cancelled')
             ORDER BY l.id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(GraphEdge {
                from: row.get(0)?,
                to: row.get(1)?,
                kind: row.get(2)?,
            })
        })?;
        collect_rows(rows)
    }

    fn direct_downstream_items(&self, item_id: &str) -> Result<Vec<Item>> {
        let mut stmt = self.conn.prepare(
            "SELECT target.id, target.project_id, target.parent_item_id, target.title, target.description, target.status, target.work_type, target.priority, target.worker_id, target.plan_path
             FROM links direct JOIN items target ON target.id = direct.to_item
             WHERE direct.from_item = ?1
             AND direct.kind IN ('blocks','hands_to')
             AND target.status NOT IN ('closed','closed_partial','cancelled')
             ORDER BY target.priority DESC, target.created_at",
        )?;
        let rows = stmt.query_map(params![item_id], row_to_item)?;
        collect_rows(rows)
    }

    fn blocking_items_for_excluding(&self, item_id: &str, excluded: &str) -> Result<Vec<Item>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.project_id, i.parent_item_id, i.title, i.description, i.status, i.work_type, i.priority, i.worker_id, i.plan_path
             FROM links l JOIN items i ON i.id = l.from_item
             WHERE l.to_item = ?1
             AND l.from_item != ?2
             AND l.kind IN ('blocks','hands_to')
             AND i.status NOT IN ('closed','closed_partial','cancelled')
             ORDER BY i.created_at",
        )?;
        let rows = stmt.query_map(params![item_id, excluded], row_to_item)?;
        collect_rows(rows)
    }
}

fn downstream_map(edges: &[GraphEdge]) -> BTreeMap<String, Vec<String>> {
    let mut downstream: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in edges {
        let _ = &edge.kind;
        downstream
            .entry(edge.from.clone())
            .or_default()
            .push(edge.to.clone());
    }
    for values in downstream.values_mut() {
        values.sort();
        values.dedup();
    }
    downstream
}

fn longest_path_from(
    id: &str,
    downstream: &BTreeMap<String, Vec<String>>,
    memo: &mut HashMap<String, Vec<String>>,
) -> Vec<String> {
    if let Some(path) = memo.get(id) {
        return path.clone();
    }
    let mut best_tail = Vec::new();
    if let Some(children) = downstream.get(id) {
        for child in children {
            let child_path = longest_path_from(child, downstream, memo);
            if child_path.len() > best_tail.len() {
                best_tail = child_path;
            }
        }
    }
    let mut path = vec![id.to_string()];
    path.extend(best_tail);
    memo.insert(id.to_string(), path.clone());
    path
}

fn reachable_from(id: &str, downstream: &BTreeMap<String, Vec<String>>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = downstream.get(id).cloned().unwrap_or_default();
    while let Some(next) = stack.pop() {
        if seen.insert(next.clone()) {
            if let Some(children) = downstream.get(&next) {
                stack.extend(children.iter().cloned());
            }
        }
    }
    seen
}

fn cycles_for(items: &[Item], edges: &[GraphEdge]) -> Vec<Vec<String>> {
    let active = items
        .iter()
        .map(|item| item.id.clone())
        .collect::<HashSet<_>>();
    let downstream = downstream_map(edges);
    let mut state: HashMap<String, VisitState> = HashMap::new();
    let mut stack = Vec::new();
    let mut cycles = Vec::new();
    for item in items {
        visit_cycles(
            &item.id,
            &active,
            &downstream,
            &mut state,
            &mut stack,
            &mut cycles,
        );
    }
    cycles.sort();
    cycles.dedup();
    cycles
}

fn visit_cycles(
    id: &str,
    active: &HashSet<String>,
    downstream: &BTreeMap<String, Vec<String>>,
    state: &mut HashMap<String, VisitState>,
    stack: &mut Vec<String>,
    cycles: &mut Vec<Vec<String>>,
) {
    match state.get(id) {
        Some(VisitState::Done) => return,
        Some(VisitState::Visiting) => {
            if let Some(start) = stack.iter().position(|value| value == id) {
                let mut cycle = stack[start..].to_vec();
                cycle.push(id.to_string());
                cycles.push(cycle);
            }
            return;
        }
        None => {}
    }
    if !active.contains(id) {
        return;
    }
    state.insert(id.to_string(), VisitState::Visiting);
    stack.push(id.to_string());
    if let Some(children) = downstream.get(id) {
        for child in children {
            visit_cycles(child, active, downstream, state, stack, cycles);
        }
    }
    stack.pop();
    state.insert(id.to_string(), VisitState::Done);
}

fn path_is_better(
    candidate: &[String],
    best: &[String],
    item_by_id: &HashMap<String, Item>,
    order: &HashMap<String, usize>,
) -> bool {
    if candidate.len() != best.len() {
        return candidate.len() > best.len();
    }
    let candidate_first = candidate.first();
    let best_first = best.first();
    let candidate_priority = candidate_first
        .and_then(|id| item_by_id.get(id))
        .map(|item| item.priority)
        .unwrap_or_default();
    let best_priority = best_first
        .and_then(|id| item_by_id.get(id))
        .map(|item| item.priority)
        .unwrap_or_default();
    if candidate_priority != best_priority {
        return candidate_priority > best_priority;
    }
    let candidate_order = candidate_first
        .and_then(|id| order.get(id))
        .copied()
        .unwrap_or(usize::MAX);
    let best_order = best_first
        .and_then(|id| order.get(id))
        .copied()
        .unwrap_or(usize::MAX);
    candidate_order < best_order
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum VisitState {
    Visiting,
    Done,
}
