---
description: 
globs: .planr/stories/**/*.md
alwaysApply: false
---

Use this template to create a new story for tracking in the `docs/stories` directory. 

# User Story: {{ID}} - {{TITLE}}

## Status: {{STATUS}}  
*(Valid values: TODO, IN PROGRESS, DONE)*

## Description:

As a {{USER_TYPE}}, I want {{FEATURE}} so that {{REASON}}.

## Acceptance Criteria:

- [ ] {{CRITERION_1}}
- [ ] {{CRITERION_2}}
- [ ] ...

## Task Groups

1. - [ ] {Major Task Group 1}
   1. - [ ] {Test Subtasks (as needed)}
   2. - [ ] {Subtask}

## ECS Implementation Tasks:

- [ ] Traits: Define marker trait (IsFeature) and data traits with appropriate defaults
- [ ] Systems: Create system file with proper entity queries and delta time handling
- [ ] Actions: Add spawn function to actions.ts with all required traits
- [ ] Renderer: Create component with useTraitEffect for proper entity visualization
- [ ] Integration: Add system to frameloop.ts in correct order
- [ ] Startup: Store entity references and implement cleanup in useEffect
- [ ] Testing: Verify system behavior and ensure proper cleanup 

Note:
- Use - [x] for completed items
- Use ~~skipped/cancelled items~~

## Estimation: {{ESTIMATION}} story points

Story Points: {Story Points (1 SP = 1 day of Human Development = 10 minutes of AI development)}

## Developer Notes:

- Implementation commentary
- Important considerations
- Technical decisions

## Chat Command Log:

- User commands
- Agent Reponse and questions