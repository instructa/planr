Use this template to create a new cursor rules based on our techstack and package.json

<ruleset>
  <format>
  # Short heading
  - Describe rule
  - Describe rule
  - Describe rule
  </format>

  <examples>
    <example1>
    # Code Style
    - Use TypeScript consistently for type safety and maintainability.
    - Use kebab-case for directories and other non-component filenames.
    </example1>
</ruleset>

<prompt_layout>
Filename: add-{{INSERT_FILENAME}}.mdc
---
description: Coding Standards & Rules for {{framework+version}}
globs: {{add here file globs like "**/*.tsx,**/.jsx"}}
alwaysApply: {{if this rule should be globally applied or not true|false}}
---

{{NAME}} Requirements

Add the most important <ruleset /> rules here

</prompt_layout>

