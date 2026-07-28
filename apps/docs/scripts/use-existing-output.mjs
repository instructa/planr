import { access, stat } from 'node:fs/promises';
import path from 'node:path';

const outputRoot = path.resolve(import.meta.dirname, '..', 'out');
await access(path.join(outputRoot, 'index.html'));
const metadata = await stat(outputRoot);
if (!metadata.isDirectory()) throw new Error('reviewed docs output is not a directory');
console.log('docs_output=existing');
