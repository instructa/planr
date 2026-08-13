import assert from 'node:assert/strict';

export function verifyGeneratedMcpInventory(tools, source) {
  const schemas = new Map();
  const sections = source.split(/^## /mu).slice(1);
  for (const section of sections) {
    const heading = /^`([^`]+)`\s*$/mu.exec(section)?.[1];
    const schema = /```json\n([\s\S]*?)\n```/u.exec(section)?.[1];
    if (!heading || !schema) continue;
    assert.ok(!schemas.has(heading), `Generated MCP schema inventory duplicates tool ${heading}`);
    schemas.set(heading, JSON.parse(schema));
  }

  assert.equal(schemas.size, tools.length, 'Generated MCP schema inventory tool count differs from runtime tools/list');
  for (const tool of tools) {
    const schema = schemas.get(tool.name);
    assert.ok(schema, `Generated MCP schema inventory is missing runtime tool ${tool.name}`);
    assert.deepEqual(
      schema.required ?? [],
      tool.inputSchema.required ?? [],
      `Generated MCP schema inventory required fields differ for ${tool.name}`,
    );
  }
}
