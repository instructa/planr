import { formatBasisPoints, previewCommand, statusLabel, visibleCompositions } from "./catalog-model.mjs";

const nodes = {
  selectA: document.querySelector("#composition-a"),
  selectB: document.querySelector("#composition-b"),
  cardA: document.querySelector("#comparison-a"),
  cardB: document.querySelector("#comparison-b"),
  comparison: document.querySelector("#comparison"),
  empty: document.querySelector("#empty-state"),
  emptyMessage: document.querySelector("#empty-message"),
  filter: document.querySelector("#recommended-only"),
  published: document.querySelector("#stat-published"),
  recommended: document.querySelector("#stat-recommended"),
  trust: document.querySelector("#stat-trust"),
  generated: document.querySelector("#generated-at"),
  copyStatus: document.querySelector("#copy-status"),
};

function catalogLocation() {
  return "./data/catalog.json";
}

function text(parent, element, value, className) {
  const node = document.createElement(element);
  if (className) node.className = className;
  node.textContent = value;
  parent.append(node);
  return node;
}

function formatDate(unix) {
  if (!Number.isFinite(unix)) return "Not published";
  return new Intl.DateTimeFormat("en", { dateStyle: "medium", timeZone: "UTC" }).format(new Date(unix * 1000));
}

function formatNumber(value) {
  return Number.isFinite(Number(value)) ? new Intl.NumberFormat("en").format(Number(value)) : "—";
}

function optionLabel(entry) {
  return `${entry.policy.id} + ${entry.binding.id} · ${statusLabel(entry.status)}`;
}

function fillSelect(select, entries, preferredIndex) {
  const previous = select.value;
  select.replaceChildren();
  for (const entry of entries) {
    const option = document.createElement("option");
    option.value = entry.id;
    option.textContent = optionLabel(entry);
    select.append(option);
  }
  select.value = entries.some((entry) => entry.id === previous) ? previous : entries[Math.min(preferredIndex, entries.length - 1)]?.id ?? "";
}

function metric(parent, label, value) {
  const wrapper = document.createElement("div");
  wrapper.className = "metric";
  text(wrapper, "span", label);
  text(wrapper, "strong", value);
  parent.append(wrapper);
}

function section(parent, title) {
  const wrapper = document.createElement("section");
  wrapper.className = "card-section";
  text(wrapper, "h4", title);
  parent.append(wrapper);
  return wrapper;
}

function definitionRow(list, termValue, detailValue) {
  const row = document.createElement("div");
  text(row, "dt", termValue);
  text(row, "dd", detailValue);
  list.append(row);
}

function renderCard(card, entry) {
  card.replaceChildren();
  if (!entry) return;
  card.dataset.compositionId = entry.id;

  const top = document.createElement("div");
  top.className = "card-top";
  const title = document.createElement("div");
  text(title, "span", `${entry.binding.id} · ${entry.binding.host}`, "binding-name");
  text(title, "h3", entry.policy.id);
  const badge = text(top, "span", statusLabel(entry.status), `badge status-${entry.status}`);
  badge.setAttribute("aria-label", `Status: ${statusLabel(entry.status)}`);
  top.prepend(title);
  card.append(top);
  text(card, "p", `Policy ${entry.policy.version} · Binding ${entry.binding.version} · Entry ${entry.entryVersion}`, "meta-line");

  const metrics = document.createElement("div");
  metrics.className = "metric-grid";
  metric(metrics, "Active agents", formatNumber(entry.policy.usage.max_active_agents));
  metric(metrics, "Parallel writers", formatNumber(entry.policy.usage.max_parallel_writers));
  metric(metrics, "Max depth", formatNumber(entry.policy.usage.max_depth));
  metric(metrics, "Metering", entry.policy.usage.metering ?? "unavailable");
  card.append(metrics);

  const compatibility = section(card, "Compatibility");
  const compatibilityList = document.createElement("dl");
  compatibilityList.className = "compatibility-list";
  definitionRow(
    compatibilityList,
    "Supported hosts",
    entry.compatibility.hosts?.length ? entry.compatibility.hosts.join(", ") : "No compatible host declared",
  );
  definitionRow(
    compatibilityList,
    "Planr versions",
    `${entry.compatibility.minPlanrVersion ?? "Any"} through ${entry.compatibility.maxPlanrVersion ?? "latest"}`,
  );
  definitionRow(
    compatibilityList,
    "Lifecycle",
    entry.replacement
      ? `${statusLabel(entry.status)}; replace with ${entry.replacement}`
      : statusLabel(entry.status),
  );
  compatibility.append(compatibilityList);

  const enforcement = section(card, "Enforcement matrix");
  const list = document.createElement("dl");
  list.className = "enforcement-list";
  for (const item of entry.enforcement) {
    const row = document.createElement("div");
    const term = text(row, "dt", item.dimension);
    text(term, "span", item.state.replaceAll("_", " "), "state");
    text(row, "dd", item.detail);
    list.append(row);
  }
  enforcement.append(list);

  const evidence = section(card, "Evaluation & provenance");
  text(evidence, "p", `${entry.evaluation.suiteId} ${entry.evaluation.suiteVersion} · evaluated ${formatDate(entry.evaluation.evaluatedAtUnix)} · review ${formatDate(entry.evaluation.reviewAtUnix)}`, "meta-line");
  text(evidence, "p", `Quality ${formatBasisPoints(entry.evaluation.metrics?.average_quality_score_bps)} · runs ${formatNumber(entry.evaluation.metrics?.runs)} · oracle passes ${formatNumber(entry.evaluation.metrics?.oracle_passes)}`, "meta-line");
  text(evidence, "p", `Manifest ${entry.registry.manifestSha256}`, "hash");
  text(
    evidence,
    "p",
    entry.registry.signatureVerified
      ? `Signer ${entry.registry.signer} · signature verified · trusted maintainer`
      : "Unsigned experimental publication · recommendation disabled",
    "meta-line",
  );

  const commandSection = section(card, "Preview safely");
  const commandBlock = document.createElement("div");
  commandBlock.className = "command-block";
  const command = previewCommand(entry.policy.id, entry.binding.id);
  text(commandBlock, "code", command);
  const button = text(commandBlock, "button", "Copy preview command", "copy-button");
  button.type = "button";
  button.dataset.command = command;
  commandSection.append(commandBlock);
}

function wireCopy() {
  document.addEventListener("click", async (event) => {
    const button = event.target.closest("button[data-command]");
    if (!button) return;
    try {
      await navigator.clipboard.writeText(button.dataset.command);
      button.textContent = "Copied";
      nodes.copyStatus.textContent = `Copied preview command for ${button.closest(".preset-card").dataset.compositionId}`;
      setTimeout(() => { button.textContent = "Copy preview command"; }, 1400);
    } catch {
      nodes.copyStatus.textContent = "Clipboard access was unavailable; select the command text to copy it manually.";
    }
  });
}

function render(catalog) {
  const entries = visibleCompositions(catalog, nodes.filter.checked);
  nodes.published.textContent = formatNumber(catalog.compositions.length);
  nodes.recommended.textContent = formatNumber(catalog.compositions.filter((entry) => entry.recommended).length);
  const signed = catalog.compositions.some((entry) => entry.registry?.signatureVerified);
  nodes.trust.textContent = signed ? "Signed registry" : "Integrity verified";
  nodes.generated.textContent = Number.isFinite(catalog.generatedAtUnix)
    ? `Projection generated ${formatDate(catalog.generatedAtUnix)} · ${catalog.source.trust}`
    : "No verified registry projection published.";
  fillSelect(nodes.selectA, entries, 0);
  fillSelect(nodes.selectB, entries, 1);
  const empty = entries.length === 0;
  nodes.empty.hidden = !empty;
  nodes.comparison.hidden = empty;
  if (empty) {
    nodes.emptyMessage.textContent = catalog.source?.message ?? (nodes.filter.checked ? "No published composition currently carries an evaluation-backed recommendation." : "No trusted compositions are published.");
    return;
  }
  const find = (id) => entries.find((entry) => entry.id === id);
  renderCard(nodes.cardA, find(nodes.selectA.value));
  renderCard(nodes.cardB, find(nodes.selectB.value));
}

async function start() {
  wireCopy();
  const response = await fetch(catalogLocation(), { cache: "no-store" });
  if (!response.ok) throw new Error(`catalog request failed with HTTP ${response.status}`);
  const catalog = await response.json();
  if (catalog.schemaVersion !== 1 || !Array.isArray(catalog.compositions)) throw new Error("unsupported catalog data");
  const update = () => render(catalog);
  nodes.filter.addEventListener("change", update);
  nodes.selectA.addEventListener("change", update);
  nodes.selectB.addEventListener("change", update);
  update();
}

start().catch((error) => {
  nodes.empty.hidden = false;
  nodes.comparison.hidden = true;
  nodes.emptyMessage.textContent = `Catalog unavailable: ${error.message}`;
  nodes.trust.textContent = "Unavailable";
});
