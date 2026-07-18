import { CommandBlock } from '@/components/command-block';
import { agentPrompts } from '@/lib/agent-recipes';

export type AgentPromptName = keyof typeof agentPrompts;

export function PromptBlock({ prompt, label }: { prompt: AgentPromptName; label: string }) {
  return <CommandBlock command={agentPrompts[prompt]} label={label} />;
}
