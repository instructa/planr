import assert from 'node:assert/strict';
import { verifyGeneratedMcpInventory } from './generated-mcp-inventory.mjs';

const runtimeTools = [{
  name: 'planr_new_runtime_tool',
  inputSchema: { required: ['item_id'] },
}];
const conceptualGuide = 'Tool families are explained here; exact names and fields live in the generated inventory.';
const generated = `## \`planr_new_runtime_tool\`

Description

\`\`\`json
{
  "required": [
    "item_id"
  ]
}
\`\`\`
`;

assert.ok(!conceptualGuide.includes(runtimeTools[0].name), 'manual guide fixture intentionally has no mutable tool inventory');
assert.doesNotThrow(
  () => verifyGeneratedMcpInventory(runtimeTools, generated),
  'a runtime tool present in generated inventory must not depend on a manual guide table',
);
assert.throws(
  () => verifyGeneratedMcpInventory(runtimeTools, ''),
  /tool count differs|missing runtime tool/u,
  'a runtime tool missing from generated inventory must fail',
);
assert.throws(
  () => verifyGeneratedMcpInventory(runtimeTools, generated.replace('"item_id"', '"plan_id"')),
  /required fields differ/u,
  'generated required-field drift must fail',
);

console.log('generated_mcp_inventory_regression=passed manual_inventory_dependency=false missing_generated_fails=true required_field_drift_fails=true');
