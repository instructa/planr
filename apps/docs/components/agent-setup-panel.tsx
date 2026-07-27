'use client';

import { ArrowRight, Check, Copy } from 'lucide-react';
import Image from 'next/image';
import Link from 'next/link';
import { useState } from 'react';
import {
  agentClientIds,
  agentRecipes,
  type AgentClientId,
} from '@/lib/agent-recipes';

export function AgentSetupPanel() {
  const [copyState, setCopyState] = useState<{
    client: AgentClientId | null;
    status: 'idle' | 'copied' | 'error';
  }>({ client: null, status: 'idle' });

  async function copyInstallPrompt(client: AgentClientId) {
    const prompt = agentRecipes[client].setupPrompt;
    try {
      await navigator.clipboard.writeText(prompt);
    } catch {
      try {
        const fallback = document.createElement('textarea');
        fallback.value = prompt;
        fallback.setAttribute('readonly', '');
        fallback.style.position = 'fixed';
        fallback.style.opacity = '0';
        document.body.append(fallback);
        fallback.select();
        const copied = document.execCommand('copy');
        fallback.remove();
        if (!copied) throw new Error('Fallback copy was rejected');
      } catch {
        setCopyState({ client, status: 'error' });
        window.setTimeout(() => setCopyState({ client: null, status: 'idle' }), 2400);
        return;
      }
    }

    setCopyState({ client, status: 'copied' });
    window.setTimeout(() => setCopyState({ client: null, status: 'idle' }), 1600);
  }

  return (
    <section id="agent-setup" className="agent-setup-shell" aria-labelledby="agent-setup-title">
      <div className="agent-setup-heading">
        <p>Set up with your agent</p>
        <h2 id="agent-setup-title">One prompt. Project-scoped by default.</h2>
        <span>
          Choose your coding agent, copy the install prompt, and hand off setup in one click.
        </span>
      </div>

      <div className="agent-setup-actions" aria-label="Copy an install prompt for your coding agent">
        {agentClientIds.map((client) => {
          const option = agentRecipes[client];
          const currentStatus = copyState.client === client ? copyState.status : 'idle';
          return (
            <button
              key={client}
              type="button"
              data-agent-copy={client}
              data-copy-state={currentStatus}
              aria-label={`${currentStatus === 'copied' ? 'Copied' : currentStatus === 'error' ? 'Copy failed for' : 'Copy install prompt for'} ${option.displayName}`}
              onClick={() => copyInstallPrompt(client)}
            >
              <span className="agent-setup-action__mark">
                <Image src={option.logoPath} width={40} height={40} alt={option.logoAlt} />
              </span>
              <span className="agent-setup-action__label">
                <small>{option.displayName}</small>
                <strong>{currentStatus === 'copied' ? 'Copied' : currentStatus === 'error' ? 'Try again' : 'Copy install prompt'}</strong>
              </span>
              {currentStatus === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
            </button>
          );
        })}
      </div>

      <span className="sr-only" role="status" aria-live="polite">
        {copyState.client && copyState.status === 'copied'
          ? `${agentRecipes[copyState.client].displayName} install prompt copied to clipboard.`
          : copyState.client && copyState.status === 'error'
            ? `Could not copy the ${agentRecipes[copyState.client].displayName} install prompt.`
            : ''}
      </span>

      <div className="agent-setup-links">
        <Link prefetch={false} href="/docs/agents/quickstart">
          Full Agent Quickstart <ArrowRight aria-hidden="true" />
        </Link>
        <Link prefetch={false} href="/docs/agents/quickstart.md">
          Open Markdown contract
        </Link>
      </div>

      <noscript>
        <div className="agent-setup-noscript">
          JavaScript is not required for setup guidance. Open the permanent recipe for
          {' '}<Link href="/docs/integrations/codex">Codex</Link>,
          {' '}<Link href="/docs/integrations/claude-code">Claude Code</Link>,
          {' '}<Link href="/docs/integrations/cursor">Cursor</Link>, or
          {' '}<Link href="/docs/integrations/grok-build">Grok Build</Link>.
        </div>
      </noscript>

      <p className="agent-setup-manual">
        Prefer to stay in the terminal? <Link prefetch={false} href="/docs/getting-started/installation">Install Planr manually</Link> and keep every step under your control.
      </p>
    </section>
  );
}
