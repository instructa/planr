use super::App;
use anyhow::Result;
use serde_json::{json, Value};
use std::process::Command as StdCommand;

impl App {
    pub(crate) fn review_workspace_value(&self) -> Result<Value> {
        let reviews = self
            .list_items_by_type("review", Some("closed"))?
            .into_iter()
            .map(|review| {
                let target = self.review_target(&review.id)?;
                let mut annotations = self.list_contexts(Some(&review.id))?;
                if let Some(target) = &target {
                    annotations.extend(self.list_contexts(Some(&target.id))?);
                }
                annotations.retain(|entry| {
                    entry.get("kind").and_then(Value::as_str) == Some("review_annotation")
                });
                Ok(json!({
                    "review": review,
                    "target": target,
                    "annotations": annotations,
                    "artifact": self.latest_review_artifact_optional(&review.id)?,
                    "evidence": target
                        .as_ref()
                        .map(|item| self.review_evidence_value(&item.id))
                        .transpose()?,
                    "close_preview": target
                        .as_ref()
                        .map(|item| self.preview_close_value(&item.id))
                        .transpose()?
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({
            "project": self.default_project()?,
            "plans": self.list_plans(None)?,
            "map": self.map_status_value()?,
            "reviews": reviews,
            "diff": self.review_workspace_git_diff(),
            "routes": {
                "annotate": "/v1/items/:id/review-annotations",
                "feedback": "/v1/items/:id/review-feedback",
                "artifact": "/v1/reviews/:id/artifact",
                "close": "/v1/reviews/:id/close"
            }
        }))
    }

    pub(crate) fn review_workspace_html(&self) -> String {
        REVIEW_WORKSPACE_HTML.to_string()
    }

    fn latest_review_artifact_optional(&self, review_id: &str) -> Result<Value> {
        match self.latest_review_artifact(review_id) {
            Ok(value) => Ok(value),
            Err(_) => Ok(Value::Null),
        }
    }

    fn review_workspace_git_diff(&self) -> Value {
        let inside = StdCommand::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(&self.root)
            .output();
        if !matches!(inside, Ok(output) if output.status.success()) {
            return json!({
                "available": false,
                "message": "No Git worktree detected for scoped diff evidence.",
                "files": [],
                "stat": "",
                "source_content_included": false
            });
        }
        let files = StdCommand::new("git")
            .args(["diff", "--name-only"])
            .current_dir(&self.root)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let stat = StdCommand::new("git")
            .args(["diff", "--stat"])
            .current_dir(&self.root)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
            .unwrap_or_default();
        json!({
            "available": true,
            "message": if files.is_empty() { "No unstaged diff files detected." } else { "Unstaged Git diff evidence is available." },
            "files": files,
            "stat": stat,
            "source_content_included": false
        })
    }
}

const REVIEW_WORKSPACE_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Planr Review Workspace</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f7f7f4;
      --ink: #1e2428;
      --muted: #647076;
      --line: #d9ded8;
      --panel: #ffffff;
      --accent: #0f766e;
      --warn: #a16207;
      --bad: #b42318;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      background: var(--bg);
      color: var(--ink);
      font: 14px/1.4 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }
    header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 16px;
      padding: 14px 18px;
      border-bottom: 1px solid var(--line);
      background: #fff;
      position: sticky;
      top: 0;
      z-index: 2;
    }
    h1 { margin: 0; font-size: 18px; font-weight: 700; letter-spacing: 0; }
    h2 { margin: 0 0 10px; font-size: 14px; letter-spacing: 0; }
    main {
      display: grid;
      grid-template-columns: 280px minmax(360px, 1fr) 360px;
      gap: 1px;
      min-height: calc(100vh - 54px);
      background: var(--line);
    }
    section {
      min-width: 0;
      padding: 14px;
      background: var(--panel);
      overflow: auto;
    }
    button, textarea, select, input {
      font: inherit;
    }
    button {
      min-height: 32px;
      border: 1px solid var(--line);
      border-radius: 6px;
      background: #fff;
      color: var(--ink);
      padding: 6px 10px;
      cursor: pointer;
    }
    button.primary { border-color: var(--accent); color: #fff; background: var(--accent); }
    button.warn { border-color: var(--warn); color: var(--warn); }
    .row { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .stack { display: grid; gap: 10px; }
    .muted { color: var(--muted); }
    .pill {
      display: inline-flex;
      align-items: center;
      border: 1px solid var(--line);
      border-radius: 999px;
      padding: 2px 8px;
      font-size: 12px;
      color: var(--muted);
    }
    .item {
      width: 100%;
      text-align: left;
      border-radius: 6px;
      margin-bottom: 8px;
      padding: 10px;
    }
    .item.active { border-color: var(--accent); box-shadow: inset 3px 0 0 var(--accent); }
    pre {
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      border: 1px solid var(--line);
      border-radius: 6px;
      padding: 10px;
      background: #fbfbf9;
      max-height: 260px;
      overflow: auto;
    }
    textarea, input, select {
      width: 100%;
      border: 1px solid var(--line);
      border-radius: 6px;
      padding: 8px;
      background: #fff;
      color: var(--ink);
    }
    textarea { min-height: 90px; resize: vertical; }
    .statusline { min-height: 20px; color: var(--muted); }
    @media (max-width: 980px) {
      main { grid-template-columns: 1fr; }
      header { position: static; }
    }
  </style>
</head>
<body>
  <header>
    <h1>Planr Review Workspace</h1>
    <div class="row">
      <span id="project" class="pill">loading</span>
      <button id="refresh" title="Reload review workspace">Refresh</button>
    </div>
  </header>
  <main>
    <section>
      <h2>Review Queue</h2>
      <div id="reviews"></div>
      <h2>Plans</h2>
      <div id="plans" class="stack"></div>
    </section>
    <section>
      <h2>Selected Review</h2>
      <div id="detail" class="stack"></div>
      <h2>Diff Evidence</h2>
      <pre id="diff"></pre>
    </section>
    <section>
      <h2>Actions</h2>
      <div class="stack">
        <select id="severity">
          <option value="info">info</option>
          <option value="warning">warning</option>
          <option value="blocking">blocking</option>
        </select>
        <input id="file" placeholder="optional file path">
        <input id="line" placeholder="optional line">
        <textarea id="message" placeholder="annotation or finding"></textarea>
        <button id="annotate" class="primary">Add Annotation</button>
        <button id="changes" class="warn">Request Changes</button>
        <button id="artifact">Write Artifact</button>
        <button id="approve" class="primary">Approve Review</button>
        <div id="status" class="statusline"></div>
      </div>
    </section>
  </main>
  <script>
    let state = null;
    let selected = null;
    const el = (id) => document.getElementById(id);
    async function api(path, options = {}) {
      const response = await fetch(path, {
        ...options,
        headers: { 'content-type': 'application/json', ...(options.headers || {}) }
      });
      const text = await response.text();
      const data = text ? JSON.parse(text) : {};
      if (!response.ok || data.error) throw new Error(data.error?.message || response.statusText);
      return data;
    }
    function reviewTarget(row) {
      return row.target || row.review;
    }
    function render() {
      el('project').textContent = state.project.name;
      const reviews = state.reviews || [];
      if (!selected && reviews[0]) selected = reviews[0].review.id;
      el('reviews').innerHTML = reviews.map((row) => {
        const review = row.review;
        const target = reviewTarget(row);
        const active = review.id === selected ? ' active' : '';
        return `<button class="item${active}" data-review="${review.id}">
          <strong>${review.title}</strong><br>
          <span class="muted">${review.id} -> ${target?.id || 'no target'}</span>
        </button>`;
      }).join('') || '<p class="muted">No open reviews.</p>';
      document.querySelectorAll('[data-review]').forEach((button) => {
        button.onclick = () => { selected = button.dataset.review; render(); };
      });
      el('plans').innerHTML = (state.plans || []).map((plan) =>
        `<div><strong>${plan.title}</strong><br><span class="muted">${plan.stage} ${plan.path}</span></div>`
      ).join('') || '<p class="muted">No plans registered.</p>';
      const row = reviews.find((entry) => entry.review.id === selected);
      if (!row) {
        el('detail').innerHTML = '<p class="muted">Select a review.</p>';
      } else {
        const target = reviewTarget(row);
        el('detail').innerHTML = `
          <div class="row"><span class="pill">${row.review.status}</span><span class="pill">${row.review.work_type}</span></div>
          <div><strong>${row.review.title}</strong><p>${row.review.description}</p></div>
          <div><strong>Target</strong><p>${target?.title || 'No target linked'} <span class="muted">${target?.status || ''}</span></p></div>
          <div><strong>Annotations</strong><pre>${JSON.stringify(row.annotations || [], null, 2)}</pre></div>
          <div><strong>Review Evidence</strong><pre>${JSON.stringify(row.evidence || {}, null, 2)}</pre></div>
          <div><strong>Close Preview</strong><pre>${JSON.stringify(row.close_preview || {}, null, 2)}</pre></div>
        `;
      }
      const diff = state.diff || {};
      el('diff').textContent = `${diff.message || ''}\n\nFiles:\n${(diff.files || []).join('\n')}\n\n${diff.stat || ''}`;
    }
    async function load() {
      state = await api('/v1/review-workspace');
      render();
    }
    async function action(fn) {
      try {
        await fn();
        el('status').textContent = 'saved';
        await load();
      } catch (error) {
        el('status').textContent = error.message;
      }
    }
    function selectedRow() {
      return (state.reviews || []).find((entry) => entry.review.id === selected);
    }
    el('refresh').onclick = load;
    el('annotate').onclick = () => action(async () => {
      const row = selectedRow();
      const target = reviewTarget(row);
      await api(`/v1/items/${target.id}/review-annotations`, {
        method: 'POST',
        body: JSON.stringify({
          message: el('message').value || 'Review note',
          severity: el('severity').value,
          file: el('file').value || undefined,
          line: el('line').value ? Number(el('line').value) : undefined,
          author: 'local-review-workspace'
        })
      });
    });
    el('changes').onclick = () => action(async () => {
      const row = selectedRow();
      const target = reviewTarget(row);
      await api(`/v1/items/${target.id}/review-feedback`, {
        method: 'POST',
        body: JSON.stringify({
          reviewer: 'local-review-workspace',
          verdict: 'not-complete',
          findings: [el('message').value || 'Changes requested']
        })
      });
    });
    el('artifact').onclick = () => action(async () => {
      const row = selectedRow();
      await api(`/v1/reviews/${row.review.id}/artifact`, { method: 'POST', body: '{}' });
    });
    el('approve').onclick = () => action(async () => {
      const row = selectedRow();
      await api(`/v1/reviews/${row.review.id}/close`, {
        method: 'POST',
        body: JSON.stringify({ verdict: 'complete', findings: [] })
      });
    });
    load().catch((error) => { el('status').textContent = error.message; });
  </script>
</body>
</html>
"#;
