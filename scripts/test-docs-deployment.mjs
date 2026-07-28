import assert from 'node:assert/strict';
import { deployDocs, deploymentCommands } from './deploy-docs.mjs';
import { LIVE_DOCS_ORACLE, verifyLiveDeployment } from '../apps/docs/scripts/verify-live-deployment.mjs';

const options = {
  receipt: '.planr/receipts/docs.json',
  input: '.planr/ci/selection.json',
  head: '0123456789abcdef0123456789abcdef01234567',
  url: 'https://planr.so',
};
const commands = deploymentCommands(options);
assert.equal(commands.filter(({ args }) => args.includes('deploy')).length, 1, 'promotion performs exactly one deployment');
assert.equal(commands.filter(({ args }) => args.includes('build')).length, 0, 'promotion never starts another build');
assert.equal(
  commands[0].args[commands[0].args.indexOf('--gates') + 1],
  'docs-content,docs-typecheck,docs-lint,docs-build,docs-artifact',
  'promotion verifies the same job-scoped docs receipt produced by CI',
);
assert.equal(commands.find(({ args }) => args.includes('deploy')).env.PLANR_DOCS_RECEIPT_VALIDATED, '1');
assert.deepEqual(commands.map(({ label }) => label), ['reviewed receipt', 'Alchemy production deployment', 'bounded live oracle']);

const calls = [];
assert.deepEqual(deployDocs(options, (executable, args) => {
  calls.push([executable, ...args]);
  return { status: 0 };
}), commands.map(({ label }) => label));
assert.equal(calls.length, 3);
assert.throws(
  () => deployDocs(options, (_executable, args) => ({ status: args.includes('verify') ? 1 : 0 })),
  /reviewed receipt failed/,
  'a stale or invalid receipt stops promotion before deploy',
);

const requested = [];
const observations = await verifyLiveDeployment('https://planr.so', {
  fetchImpl: async (url) => {
    requested.push(url.pathname);
    const route = LIVE_DOCS_ORACLE.find(({ path }) => path === url.pathname);
    return new Response(route.markers.join('\n'), { status: 200, headers: { 'content-type': `${route.type}; charset=utf-8` } });
  },
});
assert.deepEqual(requested, LIVE_DOCS_ORACLE.map(({ path }) => path));
assert.equal(observations.length, LIVE_DOCS_ORACLE.length);
assert.ok(LIVE_DOCS_ORACLE.length <= 5, 'live promotion oracle stays intentionally bounded');

console.log(JSON.stringify({
  verdict: 'pass',
  production_builds: 0,
  alchemy_deploys: 1,
  live_routes: observations.length,
  receipt_failures_stop_before_deploy: true,
}, null, 2));
