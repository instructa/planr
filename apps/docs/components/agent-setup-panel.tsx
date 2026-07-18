'use client';

import { ArrowRight, FileCheck2, ShieldCheck } from 'lucide-react';
import Image from 'next/image';
import Link from 'next/link';
import { useState, type KeyboardEvent } from 'react';
import { CommandBlock } from '@/components/command-block';
import {
  agentClientIds,
  agentRecipes,
  type AgentClientId,
} from '@/lib/agent-recipes';

export function AgentSetupPanel() {
  const [selectedClient, setSelectedClient] = useState<AgentClientId>('codex');

  function selectFromKeyboard(event: KeyboardEvent<HTMLButtonElement>, client: AgentClientId) {
    const currentIndex = agentClientIds.indexOf(client);
    let nextIndex: number | undefined;
    if (event.key === 'ArrowRight' || event.key === 'ArrowDown') nextIndex = (currentIndex + 1) % agentClientIds.length;
    if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') nextIndex = (currentIndex - 1 + agentClientIds.length) % agentClientIds.length;
    if (event.key === 'Home') nextIndex = 0;
    if (event.key === 'End') nextIndex = agentClientIds.length - 1;
    if (nextIndex === undefined) return;

    event.preventDefault();
    const nextClient = agentClientIds[nextIndex];
    setSelectedClient(nextClient);
    document.getElementById(`agent-tab-${nextClient}`)?.focus();
  }

  return (
    <section id="agent-setup" className="agent-setup-shell" aria-labelledby="agent-setup-title">
      <div className="agent-setup-heading">
        <p>Set up with your agent</p>
        <h2 id="agent-setup-title">One prompt. Project-scoped by default.</h2>
        <span>
          Pick your client and copy its complete setup contract. Your agent previews repository
          changes, preserves existing configuration, verifies diagnostics, and stops before product work.
        </span>
      </div>

      <div className="agent-setup-tabs" role="tablist" aria-label="Choose your coding agent">
        {agentClientIds.map((client) => {
          const option = agentRecipes[client];
          const selected = selectedClient === client;
          return (
            <button
              key={client}
              id={`agent-tab-${client}`}
              type="button"
              role="tab"
              aria-selected={selected}
              aria-controls={`agent-panel-${client}`}
              tabIndex={selected ? 0 : -1}
              data-agent-tab={client}
              onClick={() => setSelectedClient(client)}
              onKeyDown={(event) => selectFromKeyboard(event, client)}
            >
              <span className="agent-setup-tab__mark">
                <Image src={option.logoPath} width={40} height={40} alt={option.logoAlt} />
              </span>
              <span><strong>{option.displayName}</strong><small>{option.cardSummary}</small></span>
            </button>
          );
        })}
      </div>

      {agentClientIds.map((client) => {
        const recipe = agentRecipes[client];
        const selected = selectedClient === client;
        return (
          <div
            key={client}
            id={`agent-panel-${client}`}
            role="tabpanel"
            aria-labelledby={`agent-tab-${client}`}
            className="agent-setup-panel"
            data-agent-setup-panel={client}
            hidden={!selected}
          >
            <div className="agent-setup-prompt">
              <div className="agent-setup-prompt__heading">
                <span className="agent-setup-prompt__logo" aria-hidden="true">
                  <Image src={recipe.logoPath} width={54} height={54} alt="" />
                </span>
                <div>
                  <p>Canonical {recipe.displayName} contract</p>
                  <h3>Paste this into {recipe.displayName}</h3>
                </div>
              </div>
              <CommandBlock command={recipe.setupPrompt} label={`${recipe.displayName} setup prompt`} />
              <div className="agent-setup-links">
                <Link prefetch={false} href="/docs/agents/quickstart">
                  Full Agent Quickstart <ArrowRight aria-hidden="true" />
                </Link>
                <Link prefetch={false} href="/docs/agents/quickstart.md">
                  Open Markdown contract
                </Link>
              </div>
            </div>

            <div className="agent-setup-contract">
              <div>
                <span className="agent-setup-contract__icon"><ShieldCheck aria-hidden="true" /></span>
                <h3>Safety boundary</h3>
                <ul>
                  {recipe.safetyRequirements.slice(0, 4).map((requirement) => (
                    <li key={requirement}>{requirement}</li>
                  ))}
                </ul>
              </div>
              <div>
                <span className="agent-setup-contract__icon"><FileCheck2 aria-hidden="true" /></span>
                <h3>Required success receipt</h3>
                <ul>
                  {recipe.successReceiptFields.map((field) => <li key={field}>{field}</li>)}
                </ul>
              </div>
              <p className="agent-setup-reload"><strong>After setup:</strong> {recipe.reloadGuidance}</p>
            </div>
          </div>
        );
      })}

      <noscript>
        <div className="agent-setup-noscript">
          JavaScript is not required for setup guidance. Open the permanent recipe for
          {' '}<Link href="/docs/integrations/codex">Codex</Link>,
          {' '}<Link href="/docs/integrations/claude-code">Claude Code</Link>, or
          {' '}<Link href="/docs/integrations/cursor">Cursor</Link>.
        </div>
      </noscript>

      <p className="agent-setup-manual">
        Prefer to stay in the terminal? <Link prefetch={false} href="/docs/getting-started/installation">Install Planr manually</Link> and keep every step under your control.
      </p>
    </section>
  );
}
