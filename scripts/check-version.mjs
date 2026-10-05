import { checkVersions } from './versions.mjs';

try {
  if (process.argv.length > 3) {
    throw Error('usage: node scripts/check-version.mjs [v<version>]');
  }
  console.log(`planr: versions match ${checkVersions(process.argv[2])}`);
} catch (error) {
  console.error(`planr: ${error.message}`);
  process.exitCode = 1;
}
