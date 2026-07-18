'use client';

import { Check, Copy } from 'lucide-react';
import { useState } from 'react';

type CommandBlockProps = {
  command: string;
  label?: string;
};

export function CommandBlock({ command, label = 'Terminal' }: CommandBlockProps) {
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'error'>('idle');

  async function copyCommand() {
    try {
      await navigator.clipboard.writeText(command);
    } catch {
      try {
        const fallback = document.createElement('textarea');
        fallback.value = command;
        fallback.setAttribute('readonly', '');
        fallback.style.position = 'fixed';
        fallback.style.opacity = '0';
        document.body.append(fallback);
        fallback.select();
        const copied = document.execCommand('copy');
        fallback.remove();
        if (!copied) throw new Error('Fallback copy was rejected');
      } catch {
        setCopyState('error');
        window.setTimeout(() => setCopyState('idle'), 2400);
        return;
      }
    }
    setCopyState('copied');
    window.setTimeout(() => setCopyState('idle'), 1600);
  }

  const statusLabel = copyState === 'copied'
    ? 'Command copied'
    : copyState === 'error'
      ? 'Copy failed'
      : 'Copy command';

  return (
    <div className="command-block not-prose" data-testid="command-block" data-copy-state={copyState}>
      <div className="command-block__header">
        <span>{label}</span>
        <button
          type="button"
          className="command-block__copy"
          data-testid="copy-command"
          aria-label={statusLabel}
          onClick={copyCommand}
        >
          {copyState === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
          <span>{copyState === 'copied' ? 'Copied' : copyState === 'error' ? 'Try again' : 'Copy'}</span>
        </button>
        <span className="sr-only" role="status" aria-live="polite">
          {copyState === 'copied' ? `${label} copied to clipboard.` : copyState === 'error' ? `Could not copy ${label}. Select the text and copy it manually.` : ''}
        </span>
      </div>
      <pre tabIndex={0} aria-label={`${label} command`}>
        <code>{command}</code>
      </pre>
    </div>
  );
}
