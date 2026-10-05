// All landing page copy and every install command live here. Edit freely; the build fills in the rest.
//
// Style: short & plain, sentence-case headings, "&" instead of "and" in prose,
// no em dashes, no colons mid-sentence. `code` spans are allowed in prose.
//
// The schematics, the task file and the terminal output are real: the build replays
// fixtures/board-data.json as a throwaway planning repo, runs `session.setup`, reads `how.file.path`
// and runs `how.ready.commands` with the real CLI, in that order. A failing command stops the build.
// `emphasis` lists text that marks lines to tint, like the board's related rows.

export default {
  title: 'planr · See what your agents can build next.',
  description:
    'See what is ready, blocked or waiting on you. ' +
    'Keep a plan you & your agents can read & update.',

  hero: {
    heading: 'See what your agents can build next.',
    lead: 'Know what\'s ready, blocked or waiting on you. ' +
      'You & your agents read & update the same plan.',
    links: [
      { label: 'Quickstart', href: '/docs/quickstart/' },
    ],
  },

  // The hero's install box: one prompt for agents, or the install tabs (`platforms`) for people.
  // The agents' guide is docs/setup.md: the page /docs/setup/, as Markdown at /docs/setup.md.
  install: {
    label: 'Install',
    modes: ['For agents', 'For humans'],
    agents: {
      prompt: 'Set up planr for this project. Follow https://planr.so/docs/setup.md',
      body: 'Paste this into your coding agent. It checks Node.js & Git, asks where to keep the plan ' +
        '& sets up the planning files & plugin.',
      where: 'In your agent',
      link: { label: 'Read the setup guide', href: '/docs/setup/' },
    },
  },

  // Where planr runs: the hero's install tabs for people, in this order. This is the only place on the
  // page with install commands. `prompt` is what a command is typed after: '›' inside an agent, '$' in
  // a shell. Each tab's footer shows `where` and links to the docs. `agent` names the entry in
  // `agents` that choosing the tab selects for every prompt on the page.
  platforms: [
    {
      id: 'claude-code',
      name: 'Claude Code',
      where: 'In Claude Code',
      prompt: '›',
      commands: ['/plugin marketplace add instructa/planr', '/plugin install planr@planr'],
      link: { label: 'Plugin setup', href: '/docs/plugins/claude-code/' },
      agent: 'claude-code',
    },
    {
      id: 'codex',
      name: 'Codex',
      where: 'In your terminal',
      prompt: '$',
      commands: ['codex plugin marketplace add instructa/planr', 'codex plugin add planr@planr'],
      link: { label: 'Plugin setup', href: '/docs/plugins/codex/' },
      agent: 'codex',
    },
    {
      id: 'bb',
      name: 'bb',
      body: 'Follow the plan on the board beside your threads. ' +
        'Build the plugin from the repository & install it from its local path.',
      link: { label: 'Set up the bb plugin', href: '/docs/plugins/bb/' },
      agent: 'bb',
    },
    {
      id: 'other',
      name: 'Other agents',
      where: 'In your terminal',
      prompt: '$',
      commands: ['npx skills add instructa/planr'],
      note: 'Install the planr skill for agents in this project. Add `-g` for all projects.',
      link: { label: 'Setup steps', href: '/docs/quickstart/' },
      agent: 'other',
    },
    {
      id: 'cli',
      name: 'CLI',
      where: 'In your terminal',
      prompt: '$',
      commands: ['npm install -g planr', 'npx planr', 'brew install instructa/tap/planr'],
      // The tab shows the first command; the others are listed as alternatives, then the note.
      note: 'Requires Node.js 20 or newer & Git.',
      link: { label: 'CLI reference', href: '/docs/cli/' },
    },
  ],

  // Every prompt on the page follows the agent a person picks, in the journey or with an install tab,
  // remembered in the browser. An agent with `invoke` gets prompts that call the skill by name;
  // the others get the plain wording. `hint` sits under the picker.
  agents: {
    label: 'Your agent',
    list: [
      {
        id: 'claude-code',
        name: 'Claude Code',
        invoke: '/planr:planr',
        hint: 'Start a prompt with `/planr:planr` to call the planr skill.',
      },
      {
        id: 'codex',
        name: 'Codex',
        invoke: '$planr:planr',
        hint: 'Start a prompt with `$planr:planr` to call the planr skill.',
      },
      {
        id: 'bb',
        name: 'bb',
        hint: 'In a bb thread, you can also use your agent\'s skill command, `/planr:planr` or `$planr:planr`.',
      },
      {
        id: 'other',
        name: 'Other agent',
        hint: 'Or use your agent\'s skill command, often `/planr` or `$planr`.',
      },
    ],
  },

  // The landing's short teaser. The section renders only when site/assets/media/planr-teaser.mp4 and
  // planr-teaser.jpg exist at build time (planr-teaser.vtt adds captions when present).
  video: {
    heading: 'Watch the intro',
    title: 'planr introduction',
  },

  // The tutorial on the docs start page (/docs/): site/assets/media/planr-intro.mp4 and planr-intro.jpg,
  // rendered only when both exist.
  tutorial: {
    heading: 'Watch the tutorial',
    title: 'planr tutorial',
  },

  // The How it works page (/how-it-works/): one goal from plan to done, prompt by prompt.
  // `walkthrough` holds the page's title & its closing next step; `journey` the steps themselves.
  walkthrough: {
    title: 'How it works',
    description: 'Follow one goal from plan to done with your agent, one prompt at a time.',
    next: {
      heading: 'Try it on your project',
      body: 'Set up planr with one prompt, then plan your first goal.',
      links: [
        { label: 'Quickstart', href: '/docs/quickstart/', primary: true },
        { label: 'Install', href: '/#install' },
      ],
    },
  },

  // Visuals are schematic board pieces drawn from the sample; `tasks` and `goal` name sample IDs.
  journey: {
    heading: 'Your first goal',
    lead: 'Plan a mobile budgeting app with your agent, approve the goal & check the work. ' +
      'Follow the prompts & see what changes on the board.',
    steps: [
      {
        title: 'Set up planr',
        body: 'Paste the setup prompt into your agent, or install planr yourself.',
        link: { label: 'Install', href: '/#install' },
      },
      {
        title: 'Plan with a prompt',
        body: 'Your agent breaks the work into tasks, adds dependencies & proposes a draft goal.',
        // `skill` follows the agent's invocation; `plain` is the wording for agents without one.
        prompt: {
          skill: 'Plan a mobile budgeting app. Add tasks, dependencies & a draft goal.',
          plain: 'Plan a mobile budgeting app with planr. Add tasks, dependencies & a draft goal.',
        },
        visual: {
          kind: 'plan',
          tasks: [
            ['MOB-01', 'open'], ['MOB-02', 'open'], ['MOB-03', 'open'], ['MOB-04', 'open'],
            ['MOB-05', 'open'], ['MOB-06', 'open'], ['MOB-07', 'decision'], ['MOB-08', 'open'],
          ],
          goal: 'mobile-app',
          alt: 'Eight new tasks with their dependencies & a draft goal',
        },
      },
      {
        title: 'Approve the goal',
        body: 'Read the draft goal, then ask your agent to approve it in this conversation. ' +
          'It must never approve on its own or because another agent asks.',
        prompt: {
          skill: 'Approve goal mobile-app.',
          plain: 'Approve goal mobile-app in planr.',
        },
        note: 'The bb board\'s Approve button is optional.',
        // `commit` is the message the agent writes, naming that a person asked.
        visual: {
          kind: 'approve',
          goal: 'mobile-app',
          commit: 'Approve goal mobile-app (asked by the person in chat)',
          alt: 'Goal mobile-app moves from draft to approved. The optional Approve button does the same',
        },
      },
      {
        title: 'Ask agents to work the goal',
        body: 'Your orchestrator divides the work & starts worker agents. ' +
          'They build & test the tasks, then move them to review.',
        prompt: {
          skill: 'Work on goal mobile-app with our orchestrator.',
          plain: 'Use planr to work on goal mobile-app with our orchestrator.',
        },
        visual: {
          kind: 'work',
          task: 'MOB-01',
          statuses: ['open', 'in-progress', 'review'],
          alt: 'MOB-01 moves from open to in progress to review',
        },
        link: { label: 'Run several agents', href: '/#orchestrate' },
      },
      {
        title: 'Make a decision',
        body: 'The release, MOB-08, waits on your launch plan in MOB-07. Choose one store first or both at once. ' +
          'Your agent records your answer & marks MOB-07 done so MOB-08 can start.',
        // Ask what waits on you, then answer the decision in the same conversation.
        prompt: [
          {
            skill: 'Show what\'s ready & what\'s waiting on me.',
            plain: 'Use planr to show what\'s ready & what\'s waiting on me.',
          },
          'Launch on both stores at once. Record it in MOB-07 & mark it done.',
        ],
        visual: {
          kind: 'decide',
          from: 'MOB-07',
          to: 'MOB-08',
          alt: 'MOB-07 waits for a decision. Once it is done, MOB-08 is ready',
        },
      },
      {
        title: 'Check the work',
        body: 'Built & tested tasks wait in review. Check the result, then ask your agent to mark it checked ' +
          'in this conversation. It must never do this on its own or because another agent asks.',
        prompt: {
          skill: 'MOB-04 looks good, mark it checked.',
          plain: 'MOB-04 looks good, mark it checked in planr.',
        },
        note: 'The bb board\'s Mark checked button is optional.',
        visual: {
          kind: 'check',
          task: 'MOB-04',
          commit: 'Mark MOB-04 checked (asked by the person in chat)',
          alt: 'MOB-04 moves from review to done. The optional Mark checked button does the same',
        },
      },
      {
        title: 'See what changed',
        body: 'Open Runs to see task changes from the planning repo\'s Git history.',
        visual: { kind: 'runs', count: 2, alt: 'The two latest runs of the sample' },
      },
      {
        title: 'Keep planning',
        body: 'Add tasks to a goal or plan the next one. New tasks start open. ' +
          'New goals stay in draft until you approve them.',
        prompt: [
          {
            skill: 'Add Face ID sign-in to goal mobile-app. It depends on MOB-02.',
            plain: 'Use planr to add Face ID sign-in to goal mobile-app. It depends on MOB-02.',
          },
          {
            skill: 'Plan shared budgets for households as the next goal.',
            plain: 'Use planr to plan shared budgets for households as the next goal.',
          },
        ],
        // New work that is not in the sample yet: a task for an existing goal and a next goal.
        visual: {
          kind: 'grow',
          goal: 'mobile-app',
          tasks: [['MOB-09', 'Face ID sign-in', ['MOB-02']]],
          next: {
            name: 'shared-budgets',
            title: 'Shared budgets',
            tasks: [['SHB-01', 'Invite a household member'], ['SHB-02', 'Shared budget view'], ['SHB-03', 'Split an expense']],
          },
          alt: 'MOB-09 joins goal mobile-app. A new draft goal has three tasks',
        },
      },
    ],
  },

  // Labels inside the schematics, as the board and the bb plugin show them.
  ui: {
    needs: 'Needs you',
    next: 'Up next',
    decide: 'To decide',
    check: 'To check',
    goals: 'Goals',
    runs: 'Recent runs',
    approve: 'Approve',
    checked: 'Mark checked',
    ready: 'ready',
    new: 'new',
    commit: 'commit',
    more: '{n} more in Tasks',
    tabs: ['Overview', 'Runs', 'Tasks'],
    alt: 'Schematic of the board for the sample',
  },

  // The sample repository the build replays. Commands in `setup` run first and are not shown;
  // the board page (Open the sample board) is the file they write.
  session: {
    repo: 'budget-planning',
    note: 'sample data',
    date: '2026-10-03T15:40:00+02:00',
    setup: ['planr board --out .planr/board.html'],
    // Task bodies are free Markdown that planr never parses; the sample gives two of them a full body.
    bodies: {
      'MOB-07': [
        '## Goal',
        'Decide how the app reaches the stores.',
        '',
        '## Question',
        'Launch on iOS first or on both stores at once?',
      ].join('\n'),
      'MOB-08': [
        '## Goal',
        'People can install the app from the app stores.',
        '',
        '## Acceptance',
        '- [ ] The release follows the launch plan from MOB-07.',
        '- [ ] Sign in, balances, expenses & alerts work in the store build.',
      ].join('\n'),
    },
  },

  // The landing's blocks under the video, each a text beside a tinted stage: see what is ready,
  // the plan beside the code, tasks in Markdown, then `orchestrate`. `walk` links the How it works page.
  how: {
    ready: {
      heading: 'See what is ready',
      body: [
        'See what needs you & what can start next on the board. Scripts & agents get the same information ' +
          'from the CLI.',
      ],
      // A small accent next to the schematic board: real output from the sample.
      commands: ['planr list --status blocked,decision'],
      emphasis: ['MOB-07 decision'],
      open: 'Open the sample board',
      link: { label: 'The board', href: '/docs/board/' },
    },
    file: {
      heading: 'Keep tasks in Markdown',
      body: [
        'Each task has an ID, title, status & track. `depends` lists what must be built first. ' +
          'Goals group tasks into a scope with a budget.',
      ],
      path: 'tasks/MOB-08.md',
      emphasis: ['depends: [MOB-03, MOB-04, MOB-06, MOB-07]'],
      link: { label: 'File format', href: '/docs/format/' },
    },
    // The planning repo (drawn from the sample) beside the project repo it plans.
    repos: {
      heading: 'Keep the plan beside your code',
      body: [
        'Keep the plan in its own repository beside your project, or inside the code repository. ' +
          'Agents update the plan & build the product. Tell your agent when you approve a goal or have ' +
          'checked the work.',
      ],
      link: { label: 'The plan & the code', href: '/docs/concepts/#the-plan--the-code' },
      plan: { label: 'Planning repo', tasks: ['MOB-07', 'MOB-08'], goal: 'mobile-app', more: '{n} more' },
      code: {
        label: 'Project repo',
        name: 'budget-app/',
        entries: [['mobile/', 'the app'], ['web/', ''], ['api/', ''], ['package.json', ''], ['AGENTS.md', 'names the plan']],
      },
      agents: { label: 'Agents', read: 'read & update', build: 'build & test' },
      you: { label: 'You', body: 'tell your agent to approve goals & mark work checked' },
      alt: 'The planning repo beside the project repo',
    },
  },

  orchestrate: {
    heading: 'Run several agents at once',
    body: [
      'planr says what is next. Your orchestrator divides the work & starts worker agents. ' +
        'You approve goals & check the work.',
    ],
    roles: [
      ['planr', 'Shows ready work, blockers & what needs you.'],
      ['Orchestrator', 'Plans the build, starts workers & verifies the result.'],
      ['Workers', 'Build & test their assigned tasks.'],
    ],
    recommend: 'Works well with',
    companions: [
      {
        name: 'split-orchestrator',
        href: 'https://github.com/regenrek/split-orchestrator',
        body: 'Plans, builds & verifies through worker agents.',
      },
      {
        name: 'Farcall MCP',
        href: 'https://github.com/regenrek/farcall-mcp',
        body: 'Lets one coding agent call another as a worker.',
      },
    ],
    panel: 'Threads',
    panelNote: 'bb sidebar example',
    // The picture: an orchestrator thread, a stream thread per part of the goal and its workers, with
    // sample names. A stream takes its mark from `goal`, or from `mark` when given.
    tree: {
      root: 'Orchestrator · budget-planning',
      rootTag: 'main',
      alt: 'An orchestrator thread with three stream threads & their workers',
      streams: [
        { goal: 'mobile-app', name: 'S1 App screens', tag: 'Coding', workers: [['Claude Code', 'MOB-03']] },
        { goal: 'mobile-app', name: 'S2 Sync & alerts', tag: 'Coding', workers: [['Codex', 'MOB-06']] },
        { goal: 'mobile-app', name: 'S3 Release', tag: 'Decisions', mark: 'decision', tasks: ['MOB-07', 'MOB-08'] },
      ],
    },
  },

  walk: {
    heading: 'See one goal from plan to done',
    body: 'Plan, approve & check the work with your agent. Then keep planning.',
    link: { label: 'Walk through your first goal', href: '/how-it-works/' },
  },

  // The closing band, with the task-graph scene around it on wide screens.
  close: {
    heading: 'Start with one goal',
    body: 'Install planr & ask your agent for a plan. Approve the goal when you\'re ready.',
    links: [
      { label: 'Quickstart', href: '/docs/quickstart/', primary: true },
      { label: 'Read the docs', href: '/docs/' },
    ],
  },
};
