import {
  ArrowRight,
  BookOpenText,
  Braces,
  GitBranch,
  ShieldCheck,
  Sparkles,
  TerminalSquare,
  UsersRound,
} from 'lucide-react';
import Image from 'next/image';
import Link from 'next/link';
import { HomeLayout } from 'fumadocs-ui/layouts/home';
import { CommandBlock } from '@/components/command-block';
import { AgentSetupPanel } from '@/components/agent-setup-panel';
import { PathCard } from '@/components/path-card';
import { agentRecipeList } from '@/lib/agent-recipes';
import { baseOptions } from '@/lib/layout.shared';

export default function HomePage() {
  return (
    <HomeLayout {...baseOptions()} className="planr-home">
      <div className="hero-grid" aria-hidden="true" />
      <section className="hero-shell" aria-labelledby="home-title">
        <div className="hero-copy">
          <div className="hero-kicker">
            <Sparkles aria-hidden="true" />
            Planr 1.9.0 with Pi runtime integration
          </div>
          <h1 id="home-title">
            Give every agent a plan.
            <span>Prove the work is done.</span>
          </h1>
          <p className="hero-lede">
            Planr turns product intent into a durable task graph that Codex, Claude Code, Cursor,
            Grok Build, Pi, MCP clients, and humans can share without losing ownership or evidence.
          </p>
          <div className="hero-actions">
            <Link prefetch={false} className="button-primary" href="#agent-setup">
              Set up with your agent
              <ArrowRight aria-hidden="true" />
            </Link>
            <Link prefetch={false} className="button-secondary" href="/docs/getting-started/installation">
              Install manually
            </Link>
            <Link prefetch={false} className="button-tertiary" href="/docs/getting-started/why-planr">See how it works</Link>
          </div>
          <div className="trust-row" aria-label="Product guarantees">
            <span><ShieldCheck aria-hidden="true" /> Local by default</span>
            <span><GitBranch aria-hidden="true" /> Graph-owned state</span>
            <span><UsersRound aria-hidden="true" /> 1.9.0 release</span>
          </div>
        </div>
        <div className="hero-terminal">
          <div className="hero-terminal__glow" aria-hidden="true" />
          <div className="hero-terminal__label">
            <span>Manual first run</span>
            <span>~ 2 minutes</span>
          </div>
          <CommandBlock command="brew install instructa/tap/planr" label="Install" />
          <ol className="lifecycle-preview" aria-label="Planr lifecycle preview">
            <li><span>01</span><strong>Plan</strong><small>Turn intent into a checked contract</small></li>
            <li><span>02</span><strong>Map</strong><small>Build the authoritative work graph</small></li>
            <li><span>03</span><strong>Work</strong><small>Lease one ready item at a time</small></li>
            <li><span>04</span><strong>Verify</strong><small>Close with replayable evidence</small></li>
          </ol>
        </div>
      </section>

      <AgentSetupPanel />

      <section className="agent-shell" aria-labelledby="agent-title">
        <div className="agent-heading">
          <p>Works with your coding agent</p>
          <h2 id="agent-title">Keep the tool you already trust.</h2>
          <span>Planr gives Codex, Claude Code, Cursor, Grok Build, Pi, and MCP clients the same durable plan, task graph, and evidence trail.</span>
        </div>
        <div className="agent-grid">
          {agentRecipeList.map((recipe) => (
            <Link key={recipe.id} prefetch={false} className="agent-card" href={recipe.integrationUrl}>
              <span className="agent-card__mark">
                <Image src={recipe.logoPath} width={80} height={80} alt={recipe.logoAlt} />
              </span>
              <span><strong>{recipe.displayName}</strong><small>{recipe.cardSummary}</small></span>
              <ArrowRight aria-hidden="true" />
            </Link>
          ))}
        </div>
      </section>

      <section className="path-shell" aria-labelledby="choose-path-title">
        <div className="section-heading">
          <p>Start with your outcome</p>
          <h2 id="choose-path-title">One source of truth. Several ways in.</h2>
        </div>
        <div className="path-grid">
          <PathCard
            href="/docs/getting-started/quickstart"
            eyebrow="New to Planr"
            title="Ship your first mapped task"
            description="Install the CLI, initialize a project, and move one item to evidence-backed completion."
            icon={<TerminalSquare />}
          />
          <PathCard
            href="/docs/guides"
            eyebrow="Running real work"
            title="Coordinate an existing project"
            description="Scope features, hand work across agents, and recover cleanly after interruptions."
            icon={<BookOpenText />}
          />
          <PathCard
            href="/docs/integrations"
            eyebrow="Connecting clients"
            title="Use your preferred coding agent"
            description="Set up Codex, Claude Code, Cursor, Grok Build, Pi, a generic MCP host, or a CLI-only workflow."
            icon={<Braces />}
          />
        </div>
      </section>

      <section className="principles-shell" aria-labelledby="principles-title">
        <div className="section-heading section-heading--compact">
          <p>Designed for work that outlives a chat</p>
          <h2 id="principles-title">Durable context without a control plane.</h2>
        </div>
        <div className="principle-list">
          <article><span>01</span><h3>Readable plans</h3><p>Product and build plans stay reviewable Markdown, close to the code they govern.</p></article>
          <article><span>02</span><h3>Atomic ownership</h3><p>Picks create explicit leases, so parallel workers know exactly who owns the next move.</p></article>
          <article><span>03</span><h3>Evidence first</h3><p>Logs, tests, browser proof, and reviews turn “done” into an auditable state transition.</p></article>
        </div>
      </section>

      <footer className="home-footer">
        <p>Planr is open source and local-first.</p>
        <nav aria-label="Footer navigation">
          <Link prefetch={false} href="/docs">Documentation</Link>
          <Link href="https://github.com/instructa/planr">GitHub</Link>
          <Link prefetch={false} href="/docs/contributing">Contributing</Link>
        </nav>
      </footer>
    </HomeLayout>
  );
}
