import { createServer } from 'node:http';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

// A loopback-only preview of dist/ for local checks. Production is static hosting.
const dist = fileURLToPath(new URL('../dist/', import.meta.url));
const port = Number(process.env.PORT ?? 4321);
const types = {
  '.html': 'text/html; charset=utf-8', '.css': 'text/css', '.js': 'text/javascript',
  '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.txt': 'text/plain; charset=utf-8',
  '.xml': 'application/xml', '.png': 'image/png', '.webmanifest': 'application/manifest+json',
  '.md': 'text/markdown; charset=utf-8', '.jpg': 'image/jpeg', '.mp4': 'video/mp4', '.vtt': 'text/vtt',
};
createServer((req, res) => {
  let path = normalize(decodeURIComponent(new URL(req.url, 'http://x').pathname)).replace(/^(\.\.[/\\])+/, '');
  let file = join(dist, path);
  if (existsSync(file) && statSync(file).isDirectory()) {
    file = join(file, 'index.html');
  }
  const found = existsSync(file);
  res.writeHead(found ? 200 : 404, { 'Content-Type': types[extname(found ? file : '404.html')] ?? 'application/octet-stream' });
  res.end(readFileSync(found ? file : join(dist, '404.html')));
}).listen(port, '127.0.0.1', () => console.log(`preview: http://127.0.0.1:${port}`));
