// Actual optional ArkTS module with clock/trace effect ports; not an ArkTS or device test.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import test from 'node:test';

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const compiler = [process.env.PAWS_TYPESCRIPT_PATH,
  process.env.OHOS_NDK_HOME && resolve(process.env.OHOS_NDK_HOME, 'ets/build-tools/ets-loader/node_modules/typescript'),
  process.env.DEVECO_STUDIO_HOME && resolve(process.env.DEVECO_STUDIO_HOME, 'sdk/default/openharmony/ets/build-tools/ets-loader/node_modules/typescript'),
  '/Applications/DevEco-Studio.app/Contents/sdk/default/openharmony/ets/build-tools/ets-loader/node_modules/typescript',
].filter(Boolean).find(existsSync);
if (!compiler) throw new Error('Set PAWS_TYPESCRIPT_PATH to the installed OHOS TypeScript package');
const ts = require(compiler);
const nonce = '1234567890abcdef1234567890abcdef';
const messages = JSON.parse(readFileSync(resolve(here, 'templates/ui-errors.json'), 'utf8'));
function load(name, imports, globals = {}) {
  const source = readFileSync(resolve(here, 'templates', name), 'utf8').replace('__NONCE__', nonce)
    .replace('__COMMAND_LIST__', readFileSync(resolve(here, 'templates/commands.json'), 'utf8'))
    .replace('__MULTILINE__', JSON.stringify(messages.multiline)).replace('__UNBROKEN__', JSON.stringify(messages.unbroken))
    .replace('__FIXTURE_HOST__', '127.0.0.1').replace('__HTTP_PORT__', '18746').replace('__DNS_PORT__', '18753').replace('__UDP_PORT__', '18754');
  const compiled = ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 }, reportDiagnostics: true, fileName: name });
  assert.equal((compiled.diagnostics ?? []).filter(d => d.category === ts.DiagnosticCategory.Error).length, 0);
  const module = { exports: {} };
  vm.runInNewContext(compiled.outputText, { module, exports: module.exports, setTimeout, clearTimeout, ...globals,
    require(name) { assert.ok(Object.hasOwn(imports, name), name); return imports[name]; } });
  return module.exports;
}

function harness() {
  let now = 1000, active = true;
  const records = [], diagnostics = [];
  const api = load('EngineParityUiError.ets.in', {
    '@kit.PerformanceAnalysisKit': { hilog: { warn(...args) { diagnostics.push(args); } } },
    './EngineParityPaths': { ENGINE_PARITY_NONCE: nonce,
      assertEngineParityDebug(debug) { if (!debug) throw new Error('debug required'); } },
    './EngineParityTrace': { engineParityTraceActive() { return active; },
      traceParityProbe(...args) { records.push(args); } },
  }, { Date: { now: () => now } });
  const want = (mode = 'ui-error-multiline') => ({ parameters: {
    pawsEngineParityNonce: nonce, pawsEngineParityProbe: mode, pawsEngineParityCase: 'public_case' } });
  const query = { id: 435, command: 'state.query', payload: { domain: 'proxies' } };
  return { api, records, diagnostics, want, query, time: value => { now = value; }, trace: value => { active = value; } };
}

test('compiled public fixture is complete 2134-byte multiline or unbroken Unicode, never Want content', () => {
  assert.equal(Buffer.byteLength(messages.multiline), 2134);
  assert.equal(messages.multiline.split('\n').length, 58);
  assert.ok(!/[\r\n\s]/u.test(messages.unbroken));
  assert.ok(messages.unbroken.includes('👩‍💻é'));
  assert.ok(Buffer.byteLength(messages.unbroken) > 4096);
});

test('unarmed actual Gateway hook passes through every request without effects', () => {
  const h = harness();
  for (const request of [h.query, { id: 1, command: 'vpn.start', payload: {} }, { id: 2, command: 'state.query', payload: {} }]) {
    h.api.throwEngineParityUiError(request);
  }
  assert.equal(h.records.length, 0);
});

test('arm is numeric metadata only and does not query or mutate; actual hook throws once', async () => {
  const h = harness();
  await h.api.runEngineParityUiError(h.want(), true);
  assert.deepEqual(h.records[0].slice(0, 6), ['ui-error-armed', 'public_case', true, 1, 2134, 0]);
  h.time(1050);
  assert.throws(() => h.api.throwEngineParityUiError(h.query), error => error.message === messages.multiline);
  assert.equal(h.records[1][0], 'ui-error-injected');
  assert.equal(h.records[1][5], 50);
  h.api.throwEngineParityUiError(h.query); // Retry is not faulted a second time.
  assert.equal(h.records.length, 2);
  assert.ok(!JSON.stringify(h.records).includes('Public QA multiline'));
});

test('unbroken injected text and byte metadata include the complete Unicode fixture', async () => {
  const h = harness(); await h.api.runEngineParityUiError(h.want('ui-error-unbroken'), true);
  assert.throws(() => h.api.throwEngineParityUiError(h.query), error => error.message === messages.unbroken);
  for (const record of h.records) { assert.equal(record[3], 2); assert.equal(record[4], Buffer.byteLength(messages.unbroken)); }
});

test('non-query and other domains never consume the armed proxies error', async () => {
  const h = harness(); await h.api.runEngineParityUiError(h.want(), true);
  for (const request of [{ command: 'proxies.select', payload: { domain: 'proxies' } },
    { command: 'state.subscribe', payload: { domain: 'proxies' } },
    { command: 'state.query', payload: { domain: 'status' } },
    { command: 'state.query', payload: {} }, { command: 'state.query', payload: null }]) h.api.throwEngineParityUiError(request);
  assert.equal(h.records.length, 1);
  assert.throws(() => h.api.throwEngineParityUiError(h.query), /Public QA multiline/);
});

test('exact 10s TTL expires lazily and clock rollback cannot extend an armed error', async () => {
  for (const time of [11000, 11001, 999]) {
    const h = harness(); await h.api.runEngineParityUiError(h.want(), true); h.time(time);
    h.api.throwEngineParityUiError(h.query);
    assert.equal(h.records.length, 1);
  }
  const h = harness(); await h.api.runEngineParityUiError(h.want(), true); h.time(10999);
  assert.throws(() => h.api.throwEngineParityUiError(h.query), /Public QA multiline/);
});

test('debug, nonce, fixed actions and public case IDs fail closed before arming', async () => {
  const h = harness();
  await assert.rejects(h.api.runEngineParityUiError(h.want(), false), /debug required/);
  const wrongNonce = h.want(); wrongNonce.parameters.pawsEngineParityNonce = 'other';
  await assert.rejects(h.api.runEngineParityUiError(wrongNonce, true), /nonce mismatch/);
  for (const mode of ['http', 'ui-error-anything', '', 'http://example.test']) {
    await assert.rejects(h.api.runEngineParityUiError(h.want(mode), true), /Invalid public/);
  }
  for (const caseId of ['../path', 'x'.repeat(49), '', 'private/thing']) {
    const want = h.want(); want.parameters.pawsEngineParityCase = caseId;
    await assert.rejects(h.api.runEngineParityUiError(want, true), /Invalid public/);
  }
  h.api.throwEngineParityUiError(h.query); assert.equal(h.records.length, 0);
});

test('Want body, path, URL and configuration or any extra parameters are rejected', async () => {
  const h = harness();
  for (const key of ['text', 'body', 'path', 'url', 'pawsProfileUrl', 'pawsAutoStartVpn', 'pawsEngineParityError']) {
    const want = h.want(); want.parameters[key] = 'arbitrary-private-value';
    await assert.rejects(h.api.runEngineParityUiError(want, true), /no extra parameters/);
    const diagnostic = h.diagnostics.at(-1);
    assert.equal(diagnostic[1], 'PawsParityUiError');
    assert.match(diagnostic[2], /^reject=ui-error-keys /);
    assert.equal(diagnostic.at(-1), key);
    assert.ok(!JSON.stringify(diagnostic).includes('arbitrary-private-value'));
  }
  h.api.throwEngineParityUiError(h.query); assert.equal(h.records.length, 0);
  assert.equal(h.diagnostics.length, 7);
  const bounded = harness(), want = bounded.want();
  Object.defineProperty(want.parameters, 'os.public.metadata', { enumerable: true,
    get() { throw new Error('Diagnostic must not read unknown values'); } });
  want.parameters['bad\nPRIVATE-KEY'] = 'PRIVATE-VALUE';
  want.parameters['x'.repeat(81)] = 'PRIVATE-VALUE';
  for (let index = 0; index < 20; index++) want.parameters[`extra_${index}`] = 'PRIVATE-VALUE';
  await assert.rejects(bounded.api.runEngineParityUiError(want, true), /no extra parameters/);
  const diagnostic = bounded.diagnostics[0];
  assert.deepEqual(diagnostic.slice(3, 6), ['23', '15', '2']);
  assert.equal(diagnostic[6].split(',').length, 6);
  assert.ok(diagnostic[6].includes('os.public.metadata'));
  assert.ok(JSON.stringify(diagnostic).length < 900);
  assert.ok(!JSON.stringify(diagnostic).includes('PRIVATE'));
  assert.ok(!JSON.stringify(diagnostic).includes('x'.repeat(81)));
  const maximum = harness(), maximumKeys = {};
  for (let index = 0; index < 8; index++) maximumKeys[`key${index}`.padEnd(80, 'x')] = 'PRIVATE-VALUE';
  const maximumWant = { parameters: Object.assign(maximumKeys, maximum.want().parameters) };
  await assert.rejects(maximum.api.runEngineParityUiError(maximumWant, true), /no extra parameters/);
  assert.equal(maximum.diagnostics[0][6].length, 8 * 80 + 7);
  assert.ok(JSON.stringify(maximum.diagnostics[0]).length < 900);
  assert.equal(maximum.records.length, 0);
  bounded.api.throwEngineParityUiError(bounded.query); assert.equal(bounded.records.length, 0);
  for (const kind of ['release', 'nonce', 'inactive']) {
    const denied = harness(), request = denied.want(); request.parameters.extra = 'PRIVATE';
    if (kind === 'nonce') request.parameters.pawsEngineParityNonce = 'wrong';
    if (kind === 'inactive') denied.trace(false);
    await assert.rejects(denied.api.runEngineParityUiError(request, kind !== 'release'));
    assert.equal(denied.diagnostics.length, 0);
    assert.equal(denied.records.length, 0);
  }
});

test('actual error arm ignores framework metadata without reading it and still rejects all unknown business keys', async () => {
  const h = harness(), want = h.want();
  for (const key of ['isCallBySCB', 'isShellCall', 'moduleName', 'ohos.aafwk.param.callerAbilityName',
    'ohos.aafwk.param.callerBundleName', 'ohos.aafwk.param.callerNativeName', 'ohos.aafwk.param.callerPid',
    'ohos.aafwk.param.callerToken', 'ohos.aafwk.param.callerAppIdentifier',
    'ohos.extra.param.key.showMode', 'ohos.extra.param.key.shareUrl', 'ohos.dlp.params.sandbox',
    'ohos.param.callerAppCloneIndex', 'ohos.params.gamePrelaunch']) {
    Object.defineProperty(want.parameters, key, { enumerable: true,
      get() { throw new Error('Framework metadata value must not be read'); } });
  }
  await h.api.runEngineParityUiError(want, true);
  assert.equal(h.records.length, 1); assert.equal(h.records[0][0], 'ui-error-armed');
  assert.equal(h.diagnostics.length, 0);
  assert.throws(() => h.api.throwEngineParityUiError(h.query), error => error.message === messages.multiline);
  assert.equal(h.records[1][0], 'ui-error-injected');
  h.api.throwEngineParityUiError(h.query); assert.equal(h.records.length, 2);
  for (const key of ['text', 'url', 'size', 'method', 'verb', 'pawsProfileUrl', 'pawsAutoStartVpn',
    'pawsEngineParityRecord', 'pawsEngineParityError', 'moduleNames', 'isCallBySCBExtra',
    'ohos.aafwk.param', 'ohos.aafwk.param.', 'ohos.extra.param.', 'ohos.other.param.key',
    'ohos.dlp.params.other', 'ohos.param.callerAppCloneIndexExtra', 'ohos.params.gamePrelaunchExtra']) {
    const denied = harness(), input = denied.want();
    Object.defineProperty(input.parameters, key, { enumerable: true,
      get() { throw new Error('Rejected business value must not be read'); } });
    Object.defineProperty(input.parameters, 'ohos.extra.param.key.showMode', { enumerable: true,
      get() { throw new Error('Framework value must not be read during rejection'); } });
    await assert.rejects(denied.api.runEngineParityUiError(input, true), /no extra parameters/);
    assert.equal(denied.records.length, 0); assert.equal(denied.diagnostics[0][6], key);
    assert.deepEqual(denied.diagnostics[0].slice(3, 6), ['1', '0', '0']);
    denied.api.throwEngineParityUiError(denied.query); assert.equal(denied.records.length, 0);
  }
});

test('inactive trace rejects arm and trace closure disarms without a false injected event', async () => {
  const h = harness(); h.trace(false);
  await assert.rejects(h.api.runEngineParityUiError(h.want(), true), /not initialized/);
  h.trace(true); await h.api.runEngineParityUiError(h.want(), true); h.trace(false);
  h.api.throwEngineParityUiError(h.query); h.trace(true); h.api.throwEngineParityUiError(h.query);
  assert.equal(h.records.length, 1);
});

test('an existing arm cannot be overwritten but expiry and explicit disposal permit rearming', async () => {
  const h = harness(); await h.api.runEngineParityUiError(h.want(), true);
  await assert.rejects(h.api.runEngineParityUiError(h.want('ui-error-unbroken'), true), /already armed/);
  h.time(11000); await h.api.runEngineParityUiError(h.want('ui-error-unbroken'), true);
  h.api.cancelEngineParityUiError(); h.api.throwEngineParityUiError(h.query);
  await h.api.runEngineParityUiError(h.want(), true);
  assert.throws(() => h.api.throwEngineParityUiError(h.query), /Public QA multiline/);
  assert.equal(h.records.filter(record => record[0] === 'ui-error-injected').length, 1);
});
