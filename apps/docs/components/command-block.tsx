'use client';

import { Check, Copy } from 'lucide-react';
import { useState } from 'react';

type CommandBlockProps = {
  command: string;
  label?: string;
};

export function CommandBlock({ command, label = 'Terminal' }: CommandBlockProps) {
  const [copied, setCopied] = useState(false);

  async function copyCommand() {
    try {
      await navigator.clipboard.writeText(command);
    } catch {
      const fallback = document.createElement('textarea');
      fallback.value = command;
      fallback.setAttribute('readonly', '');
      fallback.style.position = 'fixed';
      fallback.style.opacity = '0';
      document.body.append(fallback);
      fallback.select();
      document.execCommand('copy');
      fallback.remove();
    }
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1600);
  }

  return (
    <div className="command-block not-prose" data-testid="command-block">
      <div className="command-block__header">
        <span>{label}</span>
        <button
          type="button"
          className="command-block__copy"
          data-testid="copy-command"
          aria-label={copied ? 'Command copied' : 'Copy command'}
          onClick={copyCommand}
        >
          {copied ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
          <span>{copied ? 'Copied' : 'Copy'}</span>
        </button>
      </div>
      <pre tabIndex={0} aria-label={`${label} command`}>
        <code>{command}</code>
      </pre>
    </div>
  );
}
