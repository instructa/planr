---
description: Coding Standards for Svelte Components
globs: **/*.svelte
alwaysApply: false
---

Svelte Components

## Component Structure
- Use the script, template, and style tags in that order
- Separate component logic with script, markup, and styles in appropriate sections
- Keep components small and focused on a single responsibility
- Use $lib imports for shared components and utilities

## Component Logic
- Use Svelte's reactive declarations ($: syntax) for derived state
- Prefer state management with runes ($state, $derived) over traditional reactive statements
- Use event modifiers (e.g., on:click|preventDefault) instead of event handler functions when possible
- Implement lifecycle with onMount, onDestroy, or beforeNavigate hooks when needed

## Component Styling
- Use TailwindCSS classes for styling directly in the markup
- Scope component styles with the :global() selector only when necessary
- Avoid inline styles in favor of utility classes or component styles 