---
description: Coding Standards & Rules for SvelteKit v2.16.0 and Svelte v5.0.0
globs: **/*.{ts,js,svelte}
alwaysApply: true
---

SvelteKit Base Rules

## Project Structure
- Follow SvelteKit's routing convention with routes defined in src/routes directory
- Use TypeScript for all script files
- Keep global styles in app.css
- Store shared components, utilities, and types in the src/lib directory
- Use +page.svelte for page components and +layout.svelte for layouts
- Keep API endpoints in +server.ts files within appropriate route directories

## TypeScript Usage
- Use strict TypeScript typing with proper interfaces and types
- Define types for component props using the $props syntax in Svelte 5
- Avoid using any or unknown types unless absolutely necessary
- Use type assertions sparingly and only when you're certain of the type

## SvelteKit Features
- Use SvelteKit's built-in data loading with load functions
- Leverage SvelteKit's server-side rendering capabilities when appropriate
- Use form actions for form submissions instead of client-side fetch
- Utilize $app/stores for accessing page data, navigation status, and other SvelteKit stores 