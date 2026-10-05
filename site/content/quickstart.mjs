// The quickstart's chooser (/docs/quickstart/). docs/quickstart.md marks its sections with
// `<!-- show: key=value -->`; the build checks that every mark names a question & answer here.
// The first answer of each question is the default. `agent` is shared with the landing's prompts.
// An answer's third entry is a small note beside its label.

export default {
  label: 'Your setup',
  note: 'Only your steps are shown',
  questions: [
    {
      key: 'agent',
      label: 'Your agent',
      options: [['claude-code', 'Claude Code'], ['codex', 'Codex'], ['bb', 'bb'], ['other', 'Other agent']],
    },
    {
      key: 'setup',
      label: 'Who sets it up',
      options: [['agent', 'My agent'], ['you', 'I do']],
    },
    {
      key: 'where',
      label: 'Where the plan lives',
      options: [['own', 'Its own repository', 'recommended'], ['in-repo', 'My code repository']],
    },
  ],
};
