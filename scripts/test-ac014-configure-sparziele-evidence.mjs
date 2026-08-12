import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { chmodSync, cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'

const repo = path.dirname(path.dirname(new URL(import.meta.url).pathname))
const baseline = '/Users/kregenrek/projects/planr-dogfood/outcome-batching-ac014-alpha4-baseline-final'
const script = path.join(repo, 'scripts/ac014-configure-sparziele-evidence.mjs')
const root = mkdtempSync(path.join(tmpdir(), 'planr-ac014-configure-'))
process.on('exit', () => rmSync(root, { recursive: true, force: true }))

const fixture = path.join(root, 'fixture')
mkdirSync(path.join(fixture, '.planr/evidence'), { recursive: true })
cpSync(path.join(baseline, '.planr/evidence/schemas'), path.join(fixture, '.planr/evidence/schemas'), { recursive: true })
cpSync(path.join(baseline, '.planr/evidence/adapters'), path.join(fixture, '.planr/evidence/adapters'), { recursive: true })

const binary = path.join(root, 'reviewed-planr')
writeFileSync(binary, `#!${process.execPath}
const manifests=['verifier-sparziele-build-v1','verifier-sparziele-browser-cdp-v1'];
const instances=manifests.map((manifest_id,index)=>({id:'cap-'+index,capability:{manifest_id,availability:{status:'available'}}}));
process.stdout.write(JSON.stringify({object:{registry:{probes:manifests.map((manifest_id,index)=>({manifest_id,instance_id:'cap-'+index,availability_status:'available'}))},instances}}));
`)
chmodSync(binary, 0o555)
const digest = `sha256:${createHash('sha256').update(readFileSync(binary)).digest('hex')}`
const hostile = path.join(root, 'hostile')
mkdirSync(hostile)
writeFileSync(path.join(hostile, 'planr'), '#!/bin/sh\nexit 91\n')
chmodSync(path.join(hostile, 'planr'), 0o755)

const run = (args, cwd = fixture) => spawnSync(process.execPath, [script, ...args], {
  cwd, encoding: 'utf8', env: { ...process.env, PATH: hostile },
})
const positive = run(['pln-test', binary, digest])
assert.equal(positive.status, 0, `absolute reviewed binary must succeed offline with hostile PATH: ${positive.stderr}`)
assert.match(readFileSync(path.join(fixture, '.planr/evidence/obligations/sparziele.migration.json'), 'utf8'), /pln-test/)
assert.notEqual(run(['pln-test', 'planr', digest]).status, 0, 'relative/missing binary must fail')
assert.notEqual(run(['pln-test', path.join(root, 'missing'), digest]).status, 0, 'missing binary must fail')
assert.notEqual(run(['pln-test', binary, `sha256:${'0'.repeat(64)}`]).status, 0, 'wrong digest must fail')
chmodSync(binary, 0o755)
assert.notEqual(run(['pln-test', binary, digest]).status, 0, 'mutable binary must fail')
writeFileSync(binary, `${readFileSync(binary, 'utf8')}\n`)
chmodSync(binary, 0o555)
assert.notEqual(run(['pln-test', binary, digest]).status, 0, 'digest drift must fail')

const barePlanr = spawnSync('rg', ['-n', "spawnSync\\(['\\\"]planr", script], { encoding: 'utf8' })
assert.equal(barePlanr.status, 1)
console.log('AC-014 exact Planr binary configurator tests passed')
