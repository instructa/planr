---
description: Standards for SvelteKit Routing and Page Organization
globs: **/routes/**/*.{ts,js,svelte}
alwaysApply: false
---

SvelteKit Routing

## Route Organization
- Use the filesystem-based routing structure provided by SvelteKit
- Implement dynamic routes with [param] syntax in directory names
- Use route groups with (groupname) syntax for logical organization without affecting the URL
- Keep nested layouts minimal to avoid excessive nesting

## Page Files
- Use +page.svelte for the page component
- Implement +page.ts for client-side load functions
- Use +page.server.ts for server-only load functions and form actions
- Create +layout.svelte for layout components that wrap multiple pages

## API Routes
- Implement API endpoints with +server.ts files
- Use named exports for HTTP methods (GET, POST, PUT, DELETE)
- Return Response objects with appropriate status codes and headers
- Validate input data in API routes before processing 