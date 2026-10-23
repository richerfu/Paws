// Execute the actual stage-only ArkTS helpers with filesystem/OS effect ports.
// This is not an ArkTS compiler or real-device isolation test.
import assert from 'node:assert/strict';
import { closeSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync,
  rmSync, statSync, writeFileSync, writeSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
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
function load(name, imports, globals = {}) {
  const source = readFileSync(resolve(here, 'templates', name), 'utf8').replace('__NONCE__', nonce)
    .replace('__COMMAND_LIST__', readFileSync(resolve(here, 'templates/commands.json'), 'utf8'))
    .replace('__FIXTURE_HOST__', '127.0.0.1').replace('__HTTP_PORT__', '18746').replace('__DNS_PORT__', '18753').replace('__UDP_PORT__', '18754');
  const compiled = ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 }, reportDiagnostics: true, fileName: name });
  assert.equal((compiled.diagnostics ?? []).filter(d => d.category === ts.DiagnosticCategory.Error).length, 0);
  const module = { exports: {} };
  vm.runInNewContext(compiled.outputText, { module, exports: module.exports, setTimeout, clearTimeout, ...globals,
    require(name) { assert.ok(Object.hasOwn(imports, name), name); return imports[name]; } });
  return module.exports;
}

function filesystemPort(t, { race = false } = {}) {
  const root = mkdtempSync(resolve(tmpdir(), 'paws-engine-qa-paths-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const filesDir = '/data/files';
  const calls = [];
  let lastMkdirError, peerCreates = 0;
  const physical = path => {
    assert.ok(path === filesDir || path.startsWith(`${filesDir}/`));
    return path === filesDir ? root : resolve(root, path.slice(filesDir.length + 1));
  };
  const fileIo = {
    OpenMode: { CREATE: 1, WRITE_ONLY: 2, APPEND: 4 },
    mkdirSync(path, recursion) {
      calls.push(['mkdir', path, recursion]);
      assert.equal(recursion, true);
      const target = physical(path);
      try {
        mkdirSync(dirname(target), { recursive: true });
        if (race && peerCreates === 0) {
          // A real second process creates the leaf between this process's
          // recursive parent creation and final mkdir. Never delete it to pass.
          const peer = spawnSync(process.execPath, ['-e',
            'const fs=require("node:fs"),p=require("node:path");fs.mkdirSync(process.argv[1]);'
            + 'fs.writeFileSync(p.join(process.argv[1],"peer-sentinel"),"peer");', target],
          { encoding: 'utf8', timeout: 5000 });
          assert.equal(peer.error, undefined);
          assert.equal(peer.status, 0, peer.stderr);
          peerCreates++;
        }
        // Unlike Node recursive mkdir, OHOS reports EEXIST on an existing leaf.
        // Use the real filesystem's non-recursive final mkdir to reproduce it.
        mkdirSync(target);
      } catch (error) { lastMkdirError = error; throw error; }
    },
    statSync(path) { calls.push(['stat', path]); return statSync(physical(path)); },
    openSync(path, flags) { assert.equal(flags, 7); return { fd: openSync(physical(path), 'a') }; },
    writeSync(fd, content) { return writeSync(fd, content); },
    closeSync(file) { closeSync(typeof file === 'number' ? file : file.fd); },
  };
  const loadPaths = () => load('EngineParityPaths.ets.in', { '@kit.CoreFileKit': { fileIo } });
  return { filesDir, fileIo, calls, physical, loadPaths,
    lastMkdirError: () => lastMkdirError, peerCreates: () => peerCreates };
}

test('actual paths helper creates only the compile-time isolated home and accepts real EEXIST', t => {
  const h = filesystemPort(t);
  const api = h.loadPaths();
  const filesDir = h.filesDir;
  assert.equal(api.engineParityHome(filesDir), `${filesDir}/engine-parity/${nonce}/paws`);
  assert.equal(api.engineParityBase(filesDir), `${filesDir}/engine-parity/${nonce}`);
  assert.deepEqual(h.calls, [['mkdir', `${filesDir}/engine-parity/${nonce}/paws`, true],
    ['mkdir', `${filesDir}/engine-parity/${nonce}/paws`, true],
    ['stat', `${filesDir}/engine-parity/${nonce}/paws`]]);
  assert.equal(h.lastMkdirError().code, 'EEXIST');
  assert.ok(statSync(h.physical(`${filesDir}/engine-parity/${nonce}/paws`)).isDirectory());
  assert.equal(existsSync(h.physical(`${filesDir}/paws`)), false);
});

test('actual Entry trace then configure-home and separate Extension repeated calls share the same directory', t => {
  const h = filesystemPort(t);
  const entry = h.loadPaths();
  const trace = load('EngineParityTrace.ets', {
    '@kit.CoreFileKit': { fileIo: h.fileIo },
    '@kit.PerformanceAnalysisKit': { hilog: { error() { assert.fail('trace must remain writable'); } } },
    './EngineParityPaths': entry,
  });
  trace.startEngineParityTrace(h.filesDir);
  const expected = `${h.filesDir}/engine-parity/${nonce}/paws`;
  assert.equal(entry.engineParityHome(h.filesDir), expected);
  const extension = h.loadPaths(); // independent module state, shared filesystem
  assert.equal(extension.engineParityHome(h.filesDir), expected);
  assert.equal(extension.engineParityBase(h.filesDir), `${h.filesDir}/engine-parity/${nonce}`);
  assert.equal(extension.engineParityHome(h.filesDir), expected);
  assert.equal(trace.engineParityTraceActive(), true);
  trace.closeEngineParityTrace();
  const records = readFileSync(h.physical(`${expected}/gateway-trace.jsonl`), 'utf8')
    .trim().split('\n').map(JSON.parse);
  assert.deepEqual(records.map(record => record.event), ['trace-start', 'trace-close']);
  assert.equal(h.calls.filter(call => call[0] === 'mkdir').length, 5);
});

test('actual paths helper accepts a competing process directory without deleting its content', t => {
  const h = filesystemPort(t, { race: true });
  const expected = `${h.filesDir}/engine-parity/${nonce}/paws`;
  assert.equal(h.loadPaths().engineParityHome(h.filesDir), expected);
  assert.equal(h.peerCreates(), 1);
  assert.equal(h.lastMkdirError().code, 'EEXIST');
  assert.equal(readFileSync(h.physical(`${expected}/peer-sentinel`), 'utf8'), 'peer');
  assert.equal(h.loadPaths().engineParityHome(h.filesDir), expected);
  assert.equal(readFileSync(h.physical(`${expected}/peer-sentinel`), 'utf8'), 'peer');
});

test('a file occupying QA base or home fails and is preserved', t => {
  for (const suffix of ['', '/paws']) {
    const h = filesystemPort(t);
    const occupied = `${h.filesDir}/engine-parity/${nonce}${suffix}`;
    mkdirSync(dirname(h.physical(occupied)), { recursive: true });
    writeFileSync(h.physical(occupied), 'existing-file-must-not-be-removed');
    assert.throws(() => h.loadPaths().engineParityHome(h.filesDir),
      error => error === h.lastMkdirError() && ['EEXIST', 'ENOTDIR'].includes(error.code));
    assert.equal(readFileSync(h.physical(occupied), 'utf8'), 'existing-file-must-not-be-removed');
  }
});

test('invalid directory fails before any filesystem effect', () => {
  let writes = 0;
  const api = load('EngineParityPaths.ets.in', { '@kit.CoreFileKit': { fileIo: {
    mkdirSync() { writes++; } } } });
  for (const path of ['', 'relative/files', '/data/../personal']) assert.throws(() => api.engineParityHome(path));
  assert.equal(writes, 0);
});

test('debug gate does not permit release initialization', () => {
  const api = load('EngineParityPaths.ets.in', { '@kit.CoreFileKit': { fileIo: {} } });
  assert.throws(() => api.assertEngineParityDebug(false), /requires a debug/);
  api.assertEngineParityDebug(true);
});

test('filesystem failure propagates instead of falling back to personal home', () => {
  const denied = new Error('denied');
  const api = load('EngineParityPaths.ets.in', { '@kit.CoreFileKit': { fileIo: {
    mkdirSync() { throw denied; }, statSync() { throw new Error('cannot confirm directory'); } } } });
  assert.throws(() => api.engineParityHome('/data/files'), error => error === denied);
});

test('widget producer does not access file IDs or update personal widgets', () => {
  const api = load('TinyPawsFormUpdate.ets', {});
  api.updateTinyPawsForms('/data/storage/el2/base/files', 'QA connected');
});

test('form callbacks refuse creation and have no update/remove effects', () => {
  const api = load('TinyPawsFormAbility.ets', { '@kit.FormKit': { FormExtensionAbility: class {} } });
  const form = new api.default();
  assert.throws(() => form.onAddForm({}), /does not create or replace widgets/);
  form.onUpdateForm('existing-personal-widget');
  form.onRemoveForm('existing-personal-widget');
});

function traceHarness() {
  let content = '', closed = 0;
  const fs = { OpenMode: { CREATE: 1, WRITE_ONLY: 2, APPEND: 4 },
    statSync() { return { size: Buffer.byteLength(content) }; },
    openSync(path) { assert.equal(path, `/data/files/engine-parity/${nonce}/paws/gateway-trace.jsonl`); return { fd: 3 }; },
    writeSync(fd, text) { assert.equal(fd, 3); content += text; }, closeSync() { closed++; } };
  const api = load('EngineParityTrace.ets', {
    '@kit.CoreFileKit': { fileIo: fs }, '@kit.PerformanceAnalysisKit': { hilog: { error() {} } },
    './EngineParityPaths': { engineParityHome(files) { assert.equal(files, '/data/files'); return `${files}/engine-parity/${nonce}/paws`; } },
  });
  return { api, records: () => content.trim().split('\n').filter(Boolean).map(JSON.parse), closed: () => closed };
}

test('actual trace records only whitelisted metadata and never serializes query payload/data/error', () => {
  const h = traceHarness();
  h.api.startEngineParityTrace('/data/files');
  const secret = 'PRIVATE-URL-SECRET-AND-LOG-BODY';
  h.api.traceParityReceived({ id: 7, command: 'state.query', payload: { secret } });
  h.api.traceParityResult({ id: 7, command: 'state.query' }, { id: 7, ok: true, error: secret,
    data: { domain: 'status', revision: 4, configRevision: 2, generation: 1,
      observation: { engine: 'mihomo', session: 'qa-owner-1', issues: [{ field: 'memory', reason: secret }] },
      data: { activeEngine: 'mihomo', vpnSessionId: 'qa-owner-1', vpnRunning: true, secret,
        get controllerAddr() { throw new Error('Must not access private controller address'); } } },
    barrier: { generation: 1, required: [{ domain: 'status', revision: 4, groupKey: secret }] } }, 350);
  h.api.traceParityResult({ id: 8, command: 'unknown.secret' }, { id: 8, ok: true, data: secret }, 1);
  h.api.closeEngineParityTrace();
  const records = h.records();
  assert.equal(records.length, 4);
  assert.ok(!JSON.stringify(records).includes(secret));
  assert.equal(records[2].activeEngine, 'mihomo');
  assert.equal(records[2].owner, 'qa-owner-1');
  assert.equal(records[2].observedEngine, 'mihomo');
  assert.equal(records[2].observedOwner, 'qa-owner-1');
  assert.deepEqual(records[2].issueFields, ['memory']);
  assert.equal(records[2].errorPresent, true);
  assert.equal(records[2].barrier.required[0].groupKey, undefined);
  assert.equal(h.closed(), records.length);
  assert.equal(h.api.engineParityTraceActive(), false);
});

test('actual trace has a hard event cap and marks invalid barriers instead of fabricating generation zero', () => {
  const h = traceHarness();
  h.api.startEngineParityTrace('/data/files');
  h.api.traceParityResult({ id: 1, command: 'state.query' }, { id: 1, ok: true,
    barrier: { generation: NaN, required: [] } }, 10);
  assert.equal(h.records()[1].barrierValid, false);
  assert.equal(h.records()[1].barrier, undefined);
  for (let i = 0; i < 2100; i++) h.api.traceParityReceived({ id: i, command: 'state.query' });
  assert.equal(h.records().length, 2049);
  assert.equal(h.records().at(-1).event, 'trace-truncated');
  assert.equal(h.api.engineParityTraceActive(), false);
});

function networkHarness(options = {}) {
  const calls = [], records = [], diagnostics = [], handlers = {};
  const want = mode => ({ parameters: { pawsEngineParityNonce: nonce, pawsEngineParityProbe: mode,
    pawsEngineParityCase: 'public_case', pawsEngineParityRecord: 'A' } });
  const udp = {
    on(name, fn) { handlers[name] = fn; }, off(name, fn) { assert.equal(fn, handlers[name]); delete handlers[name]; calls.push('off'); },
    async bind(address) { calls.push(['bind', address]); if (options.bindPending) await new Promise(() => {}); },
    async send(value) {
      calls.push(['send', value.address]);
      const query = new Uint8Array(value.data);
      let reply;
      if (value.address.port === 18754) {
        if (options.foreignEcho) handlers.message({ message: new Uint8Array([0]).buffer,
          remoteInfo: { address: '192.0.2.20', port: 9999 } });
        reply = options.wrongEcho ? new Uint8Array([1, 2, 3]) : query.slice();
      }
      else if (options.dnsReply) reply = options.dnsReply(query);
      else {
        reply = new Uint8Array(query.length + 16);
        reply.set(query); reply[2] = 0x81; reply[3] = 0x80; reply[7] = 1;
        reply.set([0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 2, 0, 4, 203, 0, 113, 11], query.length);
      }
      handlers.message({ message: reply.buffer, remoteInfo: { address: value.address.address, port: value.address.port } });
    }, async close() { calls.push('udp-close'); },
  };
  const api = load('EngineParityNetwork.ets.in', {
    '@kit.PerformanceAnalysisKit': { hilog: { warn(...args) { diagnostics.push(args); } } },
    '@kit.NetworkKit': { socket: { constructUDPSocketInstance() { calls.push('udp-created'); return udp; } },
      http: { RequestMethod: { GET: 'GET' }, HttpDataType: { STRING: 'STRING' }, createHttp() {
        calls.push('http-created'); return { async request(url, settings) {
          calls.push(['http', url, settings]);
          if (options.httpPending) await new Promise(() => {});
          const expected = url.includes('/stream?') ? `paws-engine-parity:${nonce}\n`.repeat(32768).slice(0, 1048576) :
            `paws-engine-parity:${nonce}\ncase=public_case\n`;
          return { responseCode: 200, result: options.wrongHttp ? 'arbitrary-secret-body' :
            options.truncatedHttp ? expected.slice(0, -1) : expected };
        }, destroy() { calls.push('http-destroy'); } }; } } },
    './EngineParityPaths': { ENGINE_PARITY_NONCE: nonce, assertEngineParityDebug(debug) { assert.equal(debug, true); } },
    './EngineParityTrace': { engineParityTraceActive() { return options.traceActive !== false; },
      traceParityProbe(...args) { records.push(args); } },
  }, options.fastClock ? { setTimeout(fn, ms) { return setTimeout(fn, Math.min(ms, 10)); } } : {});
  return { api, want, calls, records, diagnostics };
}

test('network Want gate rejects wrong nonce/path/URL actions before any socket construction', async () => {
  const h = networkHarness();
  const bad = h.want('http'); bad.parameters.pawsEngineParityNonce = 'wrong';
  await assert.rejects(h.api.runEngineParityNetwork(bad, true), /nonce mismatch/);
  await assert.rejects(h.api.runEngineParityNetwork(h.want('http://private.example'), true), /Invalid bounded/);
  const badCase = h.want('http'); badCase.parameters.pawsEngineParityCase = '../personal';
  await assert.rejects(h.api.runEngineParityNetwork(badCase, true), /Invalid bounded/);
  assert.equal(h.calls.length, 0);
  const inactive = networkHarness({ traceActive: false });
  await assert.rejects(inactive.api.runEngineParityNetwork(inactive.want('http'), true), /QA home/);
  assert.equal(inactive.calls.length, 0);
});

test('actual NetworkKit HTTP probe uses only compiled fixture endpoint and exact synthetic body then destroys request', async () => {
  const h = networkHarness();
  const want = h.want('http'); want.parameters.url = 'http://private.example';
  await h.api.runEngineParityNetwork(want, true);
  const request = h.calls.find(c => Array.isArray(c) && c[0] === 'http');
  assert.equal(request[1], `http://127.0.0.1:18746/echo?nonce=${nonce}&case=public_case`);
  assert.equal(request[2].usingProxy, false);
  assert.equal(h.records[0][2], true);
  assert.equal(h.calls.at(-1), 'http-destroy');
  const mismatch = networkHarness({ wrongHttp: true });
  await mismatch.api.runEngineParityNetwork(mismatch.want('http'), true);
  assert.equal(mismatch.records[0][2], false);
  assert.ok(!JSON.stringify(mismatch.records).includes('arbitrary-secret-body'));
});

test('actual HTTP stream requires the complete fixed 1MiB body and records its distinct event', async () => {
  for (const truncatedHttp of [false, true]) {
    const h = networkHarness({ truncatedHttp });
    await h.api.runEngineParityNetwork(h.want('http-stream'), true);
    const request = h.calls.find(c => Array.isArray(c) && c[0] === 'http');
    assert.equal(request[1], `http://127.0.0.1:18746/stream?nonce=${nonce}`);
    assert.equal(request[2].usingCache, false);
    assert.equal(request[2].usingProxy, false);
    assert.equal(request[2].maxLimit, 1048576);
    assert.equal(h.records[0][0], 'network-http-stream');
    assert.equal(h.records[0][2], !truncatedHttp);
    assert.equal(h.records[0][4], truncatedHttp ? 1048575 : 1048576);
    assert.equal(h.calls.at(-1), 'http-destroy');
  }
});

test('actual HTTP stream rejects Want URLs and size overrides before creating a request', async () => {
  for (const key of ['url', 'size', 'pawsEngineParitySize', 'pawsProfileUrl']) {
    const h = networkHarness();
    const want = h.want('http-stream'); want.parameters[key] = 'http://not-the-fixture/';
    await assert.rejects(h.api.runEngineParityNetwork(want, true), /no URL or size/);
    assert.equal(h.calls.length, 0);
    assert.equal(h.records.length, 0);
    assert.equal(h.diagnostics.length, 1);
    assert.equal(h.diagnostics[0][1], 'PawsParityNetwork');
    assert.match(h.diagnostics[0][2], /^reject=http-stream-keys /);
    assert.equal(h.diagnostics[0].at(-1), key);
    assert.ok(!JSON.stringify(h.diagnostics).includes('http://not-the-fixture/'));
  }
  const h = networkHarness();
  const want = h.want('http-stream');
  const unreadValue = () => { throw new Error('Diagnostic must not read unknown values'); };
  Object.defineProperty(want.parameters, 'os.public.metadata', { enumerable: true, get: unreadValue });
  want.parameters['bad\nPRIVATE-KEY'] = 'PRIVATE-VALUE';
  want.parameters['x'.repeat(81)] = 'PRIVATE-VALUE';
  for (let index = 0; index < 20; index++) want.parameters[`extra_${index}`] = 'PRIVATE-VALUE';
  await assert.rejects(h.api.runEngineParityNetwork(want, true), /no URL or size/);
  assert.equal(h.calls.length, 0);
  const diagnostic = h.diagnostics[0];
  assert.deepEqual(diagnostic.slice(3, 6), ['23', '15', '2']);
  assert.equal(diagnostic[6].split(',').length, 6);
  assert.ok(diagnostic[6].includes('os.public.metadata'));
  assert.ok(JSON.stringify(diagnostic).length < 900);
  assert.ok(!JSON.stringify(diagnostic).includes('PRIVATE'));
  assert.ok(!JSON.stringify(diagnostic).includes('x'.repeat(81)));
  const maximum = networkHarness(), maximumKeys = {};
  for (let index = 0; index < 8; index++) maximumKeys[`key${index}`.padEnd(80, 'x')] = 'PRIVATE-VALUE';
  const maximumWant = { parameters: Object.assign(maximumKeys, maximum.want('http-stream').parameters) };
  await assert.rejects(maximum.api.runEngineParityNetwork(maximumWant, true), /no URL or size/);
  assert.equal(maximum.diagnostics[0][6].length, 8 * 80 + 7);
  assert.ok(JSON.stringify(maximum.diagnostics[0]).length < 900);
  assert.equal(maximum.calls.length, 0);
  for (const options of [{ traceActive: false }, {}]) {
    const denied = networkHarness(options), request = denied.want('http-stream');
    request.parameters.extra = 'PRIVATE';
    const debug = options.traceActive === false;
    await assert.rejects(denied.api.runEngineParityNetwork(request, debug));
    assert.equal(denied.diagnostics.length, 0);
    assert.equal(denied.calls.length, 0);
  }
  const wrong = networkHarness(), request = wrong.want('http-stream');
  request.parameters.extra = 'PRIVATE'; request.parameters.pawsEngineParityNonce = 'wrong';
  await assert.rejects(wrong.api.runEngineParityNetwork(request, true), /nonce mismatch/);
  assert.equal(wrong.diagnostics.length, 0);
});

test('actual stream ignores only framework metadata values and keeps the fixed request and business gates', async () => {
  const h = networkHarness(), want = h.want('http-stream');
  const metadata = ['isCallBySCB', 'isShellCall', 'moduleName', 'ohos.aafwk.param.callerAbilityName',
    'ohos.aafwk.param.callerBundleName', 'ohos.aafwk.param.callerNativeName', 'ohos.aafwk.param.callerPid',
    'ohos.aafwk.param.callerToken', 'ohos.aafwk.param.callerUid', 'ohos.aafwk.param.callerAppIdentifier',
    'ohos.extra.param.key.showMode', 'ohos.extra.param.key.shareUrl', 'ohos.dlp.params.sandbox',
    'ohos.param.callerAppCloneIndex', 'ohos.params.gamePrelaunch'];
  for (const key of metadata) Object.defineProperty(want.parameters, key, { enumerable: true,
    get() { throw new Error('Framework metadata value must not be read'); } });
  await h.api.runEngineParityNetwork(want, true);
  const request = h.calls.find(c => Array.isArray(c) && c[0] === 'http');
  assert.equal(request[1], `http://127.0.0.1:18746/stream?nonce=${nonce}`);
  assert.equal(request[2].method, 'GET');
  assert.equal(request[2].maxLimit, 1048576);
  assert.equal(request[2].usingCache, false);
  assert.equal(request[2].usingProxy, false);
  assert.equal(h.records[0][0], 'network-http-stream');
  assert.equal(h.records[0][2], true); assert.equal(h.records[0][4], 1048576);
  assert.equal(h.diagnostics.length, 0);
  for (const key of ['url', 'size', 'method', 'verb', 'pawsProfileUrl', 'pawsEngineParityUrl',
    'pawsEngineParitySize', 'isShellCallExtra', 'moduleNames', 'ohos.aafwk.param',
    'ohos.aafwk.param.', 'ohos.extra.param.', 'ohos.extra.parameter.key', 'ohos.other.param.key',
    'ohos.dlp.params.other', 'ohos.param.callerAppCloneIndexExtra', 'ohos.params.gamePrelaunchExtra']) {
    const denied = networkHarness(), input = denied.want('http-stream');
    Object.defineProperty(input.parameters, key, { enumerable: true,
      get() { throw new Error('Rejected business value must not be read'); } });
    Object.defineProperty(input.parameters, 'ohos.aafwk.param.callerToken', { enumerable: true,
      get() { throw new Error('Framework value must not be read during rejection'); } });
    await assert.rejects(denied.api.runEngineParityNetwork(input, true), /no URL or size/);
    assert.equal(denied.calls.length, 0); assert.equal(denied.records.length, 0);
    assert.equal(denied.diagnostics[0][6], key);
    assert.deepEqual(denied.diagnostics[0].slice(3, 6), ['1', '0', '0']);
  }
});

test('stalled actual HTTP stream reaches its deadline and destroys the request without claiming bytes', async () => {
  const h = networkHarness({ httpPending: true, fastClock: true });
  await h.api.runEngineParityNetwork(h.want('http-stream'), true);
  assert.equal(h.records[0][0], 'network-http-stream');
  assert.equal(h.records[0][2], false);
  assert.equal(h.records[0][4], 0);
  assert.equal(h.calls.at(-1), 'http-destroy');
});

test('actual trace identifies failed query domain and grouping without retaining the group or error body', () => {
  const h = traceHarness(); h.api.startEngineParityTrace('/data/files');
  const request = { id: 435, command: 'state.query', payload: { domain: 'proxies', groupKey: 'PRIVATE-GROUP' } };
  const message = 'Error: core error: native management configuration epoch was replaced\nPRIVATE-STACK-👩‍💻';
  h.api.traceParityReceived(request);
  h.api.traceParityResult(request, { id: 435, ok: false, error: message }, 0);
  const received = h.records()[1], result = h.records()[2];
  assert.equal(received.domain, 'proxies'); assert.equal(received.grouped, true);
  assert.equal(result.domain, 'proxies'); assert.equal(result.grouped, true);
  assert.equal(result.errorUtf8Bytes, Buffer.byteLength(message));
  assert.equal(result.firstLineBytes, Buffer.byteLength(message.split('\n')[0]));
  assert.equal(result.hasNewline, true);
  assert.equal(result.knownStaticErrorCategory, 'epoch-obsolete');
  assert.ok(!JSON.stringify(h.records()).includes('PRIVATE-'));
});

test('actual trace errors use only fixed categories and accurate UTF8 lengths including CRLF and surrogate pairs', () => {
  const cases = [
    ['Error: core error: runtime domain query subscription was closed or superseded', 'subscription-superseded'],
    ['Observation owner was closed', 'subscription-superseded'],
    ['A query for this observation domain is already in flight', 'subscription-superseded'],
    ['Error: stale configuration revision: expected 17, current 18', 'revision-stale'],
    ['engine observation timed out after 8 seconds', 'deadline'],
    ['runtime domain query exceeded its 8 second total budget', 'deadline'],
    ['engine observation exceeded the domain query deadline', 'deadline'],
    ['Error: core error: SingBox runtime observation failed: controller HTTP 401: opaque private details', 'native-controller'],
    ['Mihomo proxy groups failed: response missing proxies', 'native-controller'],
    ['proxy group failed: group identity mismatched', 'group-identity'],
    ['私人👩‍💻é\r\nsecond\ud800', 'unclassified'],
    ['stale configuration revision: expected secret, current value', 'unclassified'],
    ['controller HTTP 401', 'unclassified'],
  ];
  const h = traceHarness(); h.api.startEngineParityTrace('/data/files');
  for (const [error, expected] of cases) {
    const request = { id: 1, command: 'state.query', payload: { domain: 'NOT-WHITELISTED', groupKey: '' } };
    h.api.traceParityResult(request, { id: 1, ok: false, error }, 0);
    const actual = h.records().at(-1);
    assert.equal(actual.domain, undefined); assert.equal(actual.grouped, false);
    assert.equal(actual.errorUtf8Bytes, Buffer.byteLength(error));
    assert.equal(actual.firstLineBytes, Buffer.byteLength(error.split(/[\r\n]/)[0]));
    assert.equal(actual.hasNewline, /[\r\n]/.test(error));
    assert.equal(actual.knownStaticErrorCategory, expected, error);
  }
  assert.ok(!JSON.stringify(h.records()).includes('opaque private details'));
});

test('actual trace captures state refresh failure lengths without converting an already committed write to a failure', () => {
  const h = traceHarness(); h.api.startEngineParityTrace('/data/files');
  h.api.traceParityResult({ id: 9, command: 'settings.vpn' }, { id: 9, ok: true,
    stateRefreshError: 'Operation completed, but state refresh failed: private cause\nprivate stack' }, 0);
  const value = h.records().at(-1);
  assert.equal(value.ok, true); assert.equal(value.errorPresent, false); assert.equal(value.stateRefreshError, true);
  assert.equal(value.knownStaticErrorCategory, 'unclassified'); assert.equal(value.hasNewline, true);
  assert.ok(!JSON.stringify(value).includes('private'));
});

test('actual UDP probe validates synthetic DNS answer and unregisters/closes, with distinct TUN destination', async () => {
  for (const mode of ['dns-upstream', 'dns-tun']) {
    const h = networkHarness();
    await h.api.runEngineParityNetwork(h.want(mode), true);
    const target = h.calls.find(c => Array.isArray(c) && c[0] === 'send')[1];
    assert.equal(target.address, mode === 'dns-tun' ? '172.19.0.2' : '127.0.0.1');
    assert.equal(target.port, mode === 'dns-tun' ? 53 : 18753);
    assert.equal(h.records[0][2], true);
    assert.equal(h.records[0][7], 11);
    assert.equal(h.records[0][8], 1);
    assert.ok(h.calls.includes('off') && h.calls.includes('udp-close'));
  }
});

test('stalled UDP bind has a bounded deadline and still unregisters/closes the socket', async () => {
  const h = networkHarness({ bindPending: true, fastClock: true });
  await h.api.runEngineParityNetwork(h.want('dns-tun'), true);
  assert.equal(h.records[0][2], false);
  assert.ok(h.calls.includes('off') && h.calls.includes('udp-close'));
  assert.ok(!h.calls.some(c => Array.isArray(c) && c[0] === 'send'));
});

function reencodedDns(query, { type = 1, additional = false, empty = false, loop = false, wrong = false } = {}) {
  const name = [...query.slice(12, -4)];
  const address = type === 1 ? [203, 0, 113, wrong ? 99 : 11] : [0x20, 1, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x11];
  const owner = loop ? [0xc0 | (query.length >> 8), query.length & 255] : name;
  const rr = empty ? [] : [...owner, 0, type, 0, 1, 0, 0, 0, 2, 0, address.length, ...address];
  const opt = additional ? [0, 0, 41, 4, 208, 0, 0, 0, 0, 0, 0] : [];
  const reply = new Uint8Array([...query, ...rr, ...opt]);
  reply[2] = 0x81; reply[3] = 0x80; reply[7] = empty ? 0 : 1; reply[11] = additional ? 1 : 0;
  return reply;
}

test('bounded actual DNS reader accepts uncompressed owner plus standard EDNS additional', async () => {
  const h = networkHarness({ dnsReply: query => reencodedDns(query, { additional: true }) });
  await h.api.runEngineParityNetwork(h.want('dns-tun'), true);
  assert.equal(h.records[0][2], true);
  assert.equal(h.records[0][8], 1);
});

test('bounded actual DNS reader matches real fixture AAAA address', async () => {
  const h = networkHarness({ dnsReply: query => reencodedDns(query, { type: 28 }) });
  const want = h.want('dns-tun'); want.parameters.pawsEngineParityRecord = 'AAAA';
  await h.api.runEngineParityNetwork(want, true);
  assert.equal(h.records[0][2], true);
  assert.equal(h.records[0][6], 28);
  assert.equal(h.records[0][7], 0x11);
});

test('valid DNS no-data is explicit rather than classified as a protocol error', async () => {
  const h = networkHarness({ dnsReply: query => reencodedDns(query, { empty: true }) });
  await h.api.runEngineParityNetwork(h.want('dns-tun'), true);
  assert.equal(h.records[0][0], 'network-dns-tun-no-data');
  assert.equal(h.records[0][2], false);
  assert.equal(h.records[0][3], 0);
  assert.equal(h.records[0][8], 0);
});

test('bounded actual DNS reader rejects compression loops and non-fixture A addresses', async () => {
  for (const option of [{ loop: true }, { wrong: true }]) {
    const h = networkHarness({ dnsReply: query => reencodedDns(query, option) });
    await h.api.runEngineParityNetwork(h.want('dns-tun'), true);
    assert.equal(h.records[0][0], 'network-dns-tun-invalid');
    assert.equal(h.records[0][2], false);
  }
});

test('ordinary UDP echo uses fixed compiled endpoint and full exact nonce/case body then confirms close', async () => {
  const h = networkHarness({ foreignEcho: true });
  const want = h.want('udp-echo'); want.parameters.url = 'http://personal.invalid';
  await h.api.runEngineParityNetwork(want, true);
  const target = h.calls.find(call => Array.isArray(call) && call[0] === 'send')[1];
  assert.equal(target.address, '127.0.0.1'); assert.equal(target.family, 1); assert.equal(target.port, 18754);
  assert.equal(h.records[0][0], 'network-udp-echo');
  assert.equal(h.records[0][2], true);
  assert.equal(h.records[0][4], `paws-engine-parity-udp:${nonce}\ncase=public_case\n`.length);
  assert.equal(h.records[1][0], 'network-udp-echo-close');
  assert.equal(h.records[1][2], true);
  assert.ok(h.calls.includes('off') && h.calls.includes('udp-close'));
});

test('ordinary UDP mismatched datagram never becomes a successful synthetic echo', async () => {
  const h = networkHarness({ wrongEcho: true });
  await h.api.runEngineParityNetwork(h.want('udp-echo'), true);
  assert.equal(h.records[0][2], false);
  assert.equal(h.records[0][4], 3);
  assert.equal(h.records[1][2], true);
});

test('ordinary UDP bind timeout is bounded and never sends a late packet', async () => {
  const h = networkHarness({ bindPending: true, fastClock: true });
  await h.api.runEngineParityNetwork(h.want('udp-echo'), true);
  assert.equal(h.records[0][0], 'network-udp-echo');
  assert.equal(h.records[0][2], false);
  assert.equal(h.records[1][2], true);
  assert.ok(!h.calls.some(call => Array.isArray(call) && call[0] === 'send'));
});
