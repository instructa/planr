import { cpSync, mkdirSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const destination = fileURLToPath(new URL('./dist/skills/planr/', import.meta.url));
mkdirSync(new URL('./dist/skills/', import.meta.url), { recursive: true });
rmSync(destination, { recursive: true, force: true });
cpSync(new URL('../skills/planr/', import.meta.url), destination, { recursive: true });
