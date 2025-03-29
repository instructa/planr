---
description: Standards for TailwindCSS Usage in SvelteKit
globs: **/*.{svelte,html,css}
alwaysApply: false
---

TailwindCSS Standards

## Class Organization
- Group related utility classes together (layout, typography, colors, etc.)
- Use consistent ordering of utility classes across components
- Extract common patterns to components rather than repeating long class strings
- Leverage @apply in component styles only for highly reused patterns

## Responsive Design
- Use Tailwind's responsive prefixes (sm:, md:, lg:, xl:) consistently
- Design for mobile-first, then add responsive classes for larger screens
- Avoid contradictory responsive classes that could cause layout issues

## Customization
- Extend the Tailwind theme in tailwind.config.js rather than using arbitrary values
- Define custom colors, spacing, and other values in the theme configuration
- Use meaningful color names that reflect their purpose rather than their appearance 