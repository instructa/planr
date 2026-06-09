# References

## Official External Sources

- OpenAI Codex CLI help on this machine, checked 2026-06-09 with `codex --help`, `codex exec --help`, `codex review --help`, and `codex mcp --help`.
- OpenAI Docs MCP: https://platform.openai.com/docs/docs-mcp
  - Used for current Codex MCP setup concept and Codex/IDE shared MCP configuration note.
- OpenAI Codex CLI Help Center: https://help.openai.com/en/articles/11096431
  - Used for Codex CLI local coding agent and approval workflow positioning.
- Claude Code MCP docs: https://code.claude.com/docs/en/mcp
  - Used for Claude Code MCP integration and project/user configuration assumptions.
- Cursor MCP docs: https://docs.cursor.com/context/model-context-protocol
  - Used for Cursor MCP transports, `.cursor/mcp.json`, global config, and security guidance.
- Model Context Protocol docs: https://modelcontextprotocol.io/specification/draft/server/prompts
  - Used for MCP prompt behavior and prompt security requirements.
- Model Context Protocol docs: https://modelcontextprotocol.io/specification/2025-03-26/server/tools
  - Used for MCP tool behavior and model-controlled tool concepts.
- Model Context Protocol docs: https://modelcontextprotocol.io/specification/draft/client/elicitation
  - Used for sensitive information constraints and elicitation safety assumptions.

## Source Freshness Notes

- MCP and coding-agent client behavior changes frequently. Re-check Codex, Claude Code, Cursor, and MCP docs before implementing install helpers or promising exact config commands.
- The spec intentionally prefers stable integration concepts over vendor-specific hidden config internals.

## Product Independence Notes

- Planr docs, command names, assets, and implementation should remain original.
- Any retained code or asset must have its license recorded before release.
