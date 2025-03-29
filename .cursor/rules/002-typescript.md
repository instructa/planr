---
description: Coding Standards for TypeScript in SvelteKit
globs: **/*.{ts,js}
alwaysApply: false
---

TypeScript Standards

## Type Definitions
- Define interfaces and types in separate files when reused across multiple components
- Use TypeScript generics for reusable components and functions
- Avoid type casting when possible, prefer type guards instead
- Define function parameter and return types explicitly

## Module Organization
- Export types, interfaces, and functions at the bottom of the file
- Use named exports instead of default exports
- Group imports by source (standard library, external packages, internal modules)
- Use barrel files (index.ts) to export multiple related items from a directory

## SvelteKit Types
- Use PageData and PageLoad types for load functions
- Define RequestEvent parameters for API endpoints and form actions
- Use PageServerLoad for server-side load functions
- Leverage the App namespace for global type augmentation when necessary 