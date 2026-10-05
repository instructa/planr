import { createHash } from 'node:crypto';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { semver } from './versions.mjs';

try {
  const [value, output = 'Formula/planr.rb', source, ...extra] = process.argv.slice(2);
  if (extra.length || !value) {
    throw Error('usage: node scripts/homebrew-formula.mjs <version> [output] [tarball URL or path]');
  }
  semver(value);
  if (value.includes('-')) {
    throw Error('Homebrew requires a stable version');
  }
  const registry = `https://registry.npmjs.org/planr/-/planr-${value}.tgz`;
  const url = source
    ? (/^(https:|file:)/.test(source) ? source : pathToFileURL(resolve(source)).href)
    : registry;
  let tarball;
  if (url.startsWith('file:')) {
    tarball = readFileSync(fileURLToPath(url));
  } else {
    const response = await fetch(url, { signal: AbortSignal.timeout(30000) });
    if (!response.ok) {
      throw Error(`tarball download failed: HTTP ${response.status}`);
    }
    tarball = Buffer.from(await response.arrayBuffer());
  }
  const sha256 = createHash('sha256').update(tarball).digest('hex');
  const formula = `class Planr < Formula
  desc "Planning CLI for Markdown task graphs"
  homepage "https://planr.so"
  url ${JSON.stringify(url)}
  sha256 "${sha256}"
  license "MIT"

  depends_on "node"

  def install
    system "npm", "install", *std_npm_args
    bin.install_symlink libexec/"lib/node_modules/planr/skills/planr/scripts/planr.mjs" => "planr"
  end

  def caveats
    <<~EOS
      planr 2.0 is a rebuild and does not read 1.x data.
      Version 1.x remains available: brew install instructa/tap/planr@1
    EOS
  end

  test do
    assert_equal "planr ${value}\\n", shell_output("#{bin}/planr --version")
  end
end
`;
  mkdirSync(dirname(resolve(output)), { recursive: true });
  writeFileSync(output, formula);
  console.log(`planr: wrote ${output} (sha256 ${sha256})`);
} catch (error) {
  console.error(`planr: ${error.message}`);
  process.exitCode = 1;
}
