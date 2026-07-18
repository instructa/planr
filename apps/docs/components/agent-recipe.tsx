import Link from 'next/link';
import { CommandBlock } from '@/components/command-block';
import { getAgentRecipe, type AgentClientId } from '@/lib/agent-recipes';

export function AgentRecipe({ client }: { client: AgentClientId }) {
  const recipe = getAgentRecipe(client);

  return (
    <section className="not-prose my-8 space-y-6 rounded-2xl border bg-fd-card/45 p-5 sm:p-7" data-agent-recipe={client}>
      <div className="space-y-2">
        <p className="text-xs font-semibold uppercase tracking-[0.14em] text-fd-primary">
          Canonical agent setup
        </p>
        <h2 className="text-2xl font-semibold tracking-tight">Set up Planr with {recipe.displayName}</h2>
        <p className="max-w-3xl text-sm leading-6 text-fd-muted-foreground">
          Copy this complete prompt into {recipe.displayName}. It previews repository changes,
          preserves existing configuration, verifies diagnostics, and stops before product work.
        </p>
      </div>

      <CommandBlock command={recipe.setupPrompt} label={`${recipe.displayName} setup prompt`} />

      <div className="grid gap-5 lg:grid-cols-2">
        <div>
          <h3 className="font-semibold">Expected integration artifacts</h3>
          <ul className="mt-3 space-y-2 text-sm text-fd-muted-foreground">
            {recipe.expectedArtifacts.map((artifact) => (
              <li key={artifact.path}>
                <code className="text-fd-foreground">{artifact.path}</code>
                {' — '}{artifact.purpose} ({artifact.owner})
              </li>
            ))}
          </ul>
        </div>
        <div>
          <h3 className="font-semibold">Success receipt</h3>
          <ul className="mt-3 space-y-2 text-sm text-fd-muted-foreground">
            {recipe.successReceiptFields.map((field) => <li key={field}>{field}</li>)}
          </ul>
        </div>
      </div>

      <div className="rounded-xl border bg-fd-background/70 p-4 text-sm leading-6">
        <p><strong>Invoke:</strong> <code>{recipe.invocationLabel}</code></p>
        <p className="mt-2"><strong>Reload:</strong> {recipe.reloadGuidance}</p>
        <p className="mt-2"><strong>First prompt:</strong> {recipe.nextPrompts.first}</p>
      </div>

      <p className="text-sm text-fd-muted-foreground">
        <Link className="font-medium text-fd-primary underline underline-offset-4" href={recipe.integrationUrl}>
          Permanent {recipe.displayName} integration URL
        </Link>
      </p>
    </section>
  );
}
