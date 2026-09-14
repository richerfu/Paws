// Execute the application ArkTS logic with controlled platform boundaries.
// This does not replace ArkTS compilation or device network verification.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import test from 'node:test';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);
const compilerCandidates = [
  process.env.PAWS_TYPESCRIPT_PATH,
  process.env.OHOS_NDK_HOME && resolve(process.env.OHOS_NDK_HOME, 'ets/build-tools/ets-loader/node_modules/typescript'),
  process.env.DEVECO_SDK_HOME && resolve(process.env.DEVECO_SDK_HOME, 'default/openharmony/ets/build-tools/ets-loader/node_modules/typescript'),
  '/Applications/DevEco-Studio.app/Contents/sdk/default/openharmony/ets/build-tools/ets-loader/node_modules/typescript',
].filter(Boolean);
const compilerPath = compilerCandidates.find(existsSync);
if (!compilerPath) {
  throw new Error('Set PAWS_TYPESCRIPT_PATH to the TypeScript package in your OpenHarmony SDK.');
}
const ts = require(compilerPath);

function loadArkts(relativePath, overrides = {}, cache = new Map()) {
  const filename = resolve(root, relativePath);
  if (cache.has(filename)) return cache.get(filename).exports;
  const source = readFileSync(filename, 'utf8');
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
    fileName: filename,
    reportDiagnostics: true,
  });
  const errors = (compiled.diagnostics ?? []).filter(d => d.category === ts.DiagnosticCategory.Error);
  assert.equal(errors.length, 0, errors.map(d => ts.flattenDiagnosticMessageText(d.messageText, '\n')).join('\n'));
  const module = { exports: {} };
  cache.set(filename, module);
  const scope = {
    module,
    exports: module.exports,
    console,
    setTimeout,
    clearTimeout,
    setInterval,
    clearInterval,
    ...overrides.globals,
    require(name) {
      if (Object.hasOwn(overrides, name)) return overrides[name];
      if (name.startsWith('.')) {
        return loadArkts(resolve(dirname(filename), `${name}.ets`), overrides, cache);
      }
      throw new Error(`Unmocked platform module ${name} in ${relativePath}`);
    },
  };
  vm.runInNewContext(compiled.outputText, scope, { filename });
  return module.exports;
}

function configModule() {
  return loadArkts('entry/src/main/ets/vpnability/VpnConfig.ets', { '@kit.AbilityKit': {} });
}

test('URL capability forwards About links through the actual packaged plugin', async () => {
  const { UrlPlugin } = loadArkts('entry/oh_modules/@ohos-rs/ability-plugin-url/src/main/ets/UrlPlugin.ets', {
    '@ohos-rs/ability': { AsyncPluginBase: class {} },
  });
  const plugin = new UrlPlugin();
  assert.equal(plugin.id, 'ohos.url');
  const opened = [];
  const context = { abilityContext: { async openLink(url) { opened.push(url); } }, isActive: () => true };
  const about = readFileSync(resolve(root, 'crates/paws_ui/src/view/pages/tools.rs'), 'utf8')
    .split('pub(crate) fn about_page')[1].split('pub(crate) fn privacy_page')[0];
  const links = [...about.matchAll(/open_external_url\("([^"]+)"/g)].map(match => match[1]);
  assert.equal(links.length, 2, 'exercise both production About repository links');
  for (const url of links) {
    const result = await plugin.invokeAsync('open-url', { typeName: 'ohos.url.OpenRequest', value: { url } }, context);
    assert.equal(result.typeName, 'ohos.url.OpenResponse');
    assert.equal(result.value.accepted, true);
    assert.equal(opened.at(-1), url);
  }
  await assert.rejects(plugin.invokeAsync('open-url', { typeName: 'ohos.url.OpenRequest', value: { url: 'not-a-url' } }, context), /absolute URL/);
  context.abilityContext.openLink = async () => { throw new Error('browser unavailable'); };
  await assert.rejects(plugin.invokeAsync('open-url', { typeName: 'ohos.url.OpenRequest', value: { url: 'https://example.com' } }, context), /browser unavailable/);
});

test('default application VPN options survive the ArkTS configuration boundary', () => {
  const api = configModule();
  const options = api.parseOptions(api.DEFAULT_OPTIONS_JSON);
  const config = new api.PawsVpnConfig(options);
  assert.equal(config.addresses[0].address.address, '172.19.0.1');
  assert.equal(config.routes[0].destination.prefixLength, 0);
  assert.equal(config.dnsAddresses[0], '172.19.0.2');
  assert.equal(config.mtu, 1500);
});

test('corrupted or non-object JSON cannot silently become a default-route VPN', () => {
  const api = configModule();
  for (const input of ['bad-json', 'null', '[]', '1', '"vpn"']) {
    assert.throws(() => new api.PawsVpnConfig(api.parseOptions(input)), undefined, input);
  }
});

test('explicit invalid routes and addresses cannot fall back to valid defaults', () => {
  const api = configModule();
  for (const value of ['10.0.0.1/99', '10.0.0.1/no', '10.0.0.1/24junk', '999.2.3.4/24', '::1/129']) {
    assert.throws(() => api.parseLinkAddress(value, 32), undefined, value);
  }
});

test('Want retains matching session identity and transferred descriptors', () => {
  const api = configModule();
  const want = api.buildVpnWant(api.DEFAULT_OPTIONS_JSON, { ashmemFd: 21, notificationFd: 22 }, 'session-a');
  assert.equal(api.readPlatformStartAttemptId(want), 'session-a');
  const descriptors = api.readPlatformSharedMemoryFds(want);
  assert.equal(descriptors.ashmemFd, 21);
  assert.equal(descriptors.notificationFd, 22);
});

test('color mode acknowledgement preserves platform failures and rejects unsupported modes', async () => {
  const { ColorModePlugin } = loadArkts('entry/src/main/ets/plugins/ColorModePlugin.ets', {
    '@ohos-rs/ability': { AsyncPluginBase: class {} },
    '@kit.AbilityKit': { ConfigurationConstant: { ColorMode: { COLOR_MODE_NOT_SET: -1, COLOR_MODE_DARK: 0, COLOR_MODE_LIGHT: 1 } } },
    '@kit.PerformanceAnalysisKit': { hilog: { info() {}, error() {} } },
  });
  const plugin = new ColorModePlugin();
  const context = { abilityContext: { async setColorMode() { throw new Error('platform refused color mode'); } } };
  await assert.rejects(plugin.invokeAsync('set-color-mode', { typeName: 'paws.ColorModeRequest', value: { mode: 0 } }, context), /platform refused/);
  await assert.rejects(plugin.invokeAsync('set-color-mode', { typeName: 'paws.ColorModeRequest', value: { mode: 9 } }, context), /unsupported mode/);
  await assert.rejects(plugin.invokeAsync('set-color-mode', { typeName: 'paws.ColorModeRequest', value: null }, context), /expected/);
});

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test('scan cancellation uses the SDK error code and never message substrings', async () => {
  let failure;
  const { ScanPlugin } = loadArkts('entry/src/main/ets/plugins/ScanPlugin.ets', {
    '@ohos-rs/ability': { AsyncPluginBase: class {} },
    '@kit.ScanKit': {
      scanCore: { ScanType: { QR_CODE: 1 } },
      scanBarcode: { async startScanForResult() {
        if (failure) throw failure;
        return { originalValue: ' https://example.test/subscription ' };
      } },
    },
    '@kit.PerformanceAnalysisKit': { hilog: { info() {}, error() {} } },
  });
  const plugin = new ScanPlugin();
  const request = { typeName: 'paws.ScanRequest', value: {} };
  const context = { abilityContext: {} };
  assert.equal((await plugin.invokeAsync('scan-qr', request, context)).value.content,
    'https://example.test/subscription');
  failure = { code: 1000500002, message: 'localized user dismissal' };
  assert.equal((await plugin.invokeAsync('scan-qr', request, context)).value.content, '');
  failure = { code: 1000500001, message: 'cancel token service failed' };
  await assert.rejects(plugin.invokeAsync('scan-qr', request, context), /cancel token service failed/);
});

function automationFixture(native) {
  return loadArkts('entry/src/main/ets/automation/DebugAutomation.ets', {
    '@kit.AbilityKit': {},
    '@kit.PerformanceAnalysisKit': { hilog: { info() {}, error() {} } },
    'libpaws_ui.so': { default: native },
  });
}

function entryAbilityFixture(applicationDebug) {
  const automationCalls = [];
  class NativeAbility {
    context = {
      applicationInfo: { debug: applicationDebug, name: 'com.example.paws' },
      config: {},
      filesDir: '/data/paws',
      resourceManager: {},
    };

    async onCreate() {}
    onNewWant() {}
  }
  class LazyPlugin {
    constructor(factory) { this.factory = factory; }
  }
  class EmptyPlugin {}
  const { default: EntryAbility } = loadArkts(
    'entry/src/main/ets/entryability/EntryAbility.ets',
    {
      '@kit.AbilityKit': {
        ConfigurationConstant: { ColorMode: { COLOR_MODE_NOT_SET: -1 } },
      },
      '@kit.LocalizationKit': { i18n: { System: { getSystemLanguage() { return 'en-US'; } } } },
      '@kit.PerformanceAnalysisKit': { hilog: { info() {}, error() {} } },
      '@ohos-rs/ability': { LazyPlugin, NativeAbility },
      '@ohos-rs/ability-plugin-files': { FilesPlugin: EmptyPlugin },
      '@ohos-rs/ability-plugin-url': { UrlPlugin: EmptyPlugin },
      '@ohos.window': {},
      'libpaws_ui.so': { default: {
        configureAppHome() {},
        configureUiLocale() {},
        configureSystemColorMode() {},
        seedGeodataFromRawfiles() { return 0; },
      } },
      '../common/Errors': { describeError(error) { return String(error); } },
      '../vpnability/VpnConfig': { setPawsBundleName() {} },
      '../automation/DebugAutomation': {
        runDebugAutomation(want, debug) {
          automationCalls.push({ want, debug });
          return Promise.resolve();
        },
      },
      '../plugins/ClipboardPlugin': { ClipboardPlugin: EmptyPlugin },
      '../plugins/ColorModePlugin': { ColorModePlugin: EmptyPlugin },
      '../plugins/ExportPlugin': { ExportPlugin: EmptyPlugin },
      '../plugins/SafeAreaPlugin': { SafeAreaPlugin: EmptyPlugin },
      '../plugins/ScanPlugin': { ScanPlugin: EmptyPlugin },
      '../plugins/VpnPlugin': { VpnPlugin: EmptyPlugin },
    },
  );
  return { ability: new EntryAbility(), automationCalls };
}

test('protocol negative acceptance cannot count an unexpected success as failure', async () => {
  const api = automationFixture({
    async testProxyDelay() { return 5; },
    async testProxyEcho(_proxy, _url, payload) { return payload; },
  });
  await assert.rejects(api.runDebugAutomation({ parameters: {
    pawsDelayProxy: 'proxy', pawsExpectDelayFailure: true,
  } }, true, async () => {}), /unexpectedly succeeded/);
  await assert.rejects(api.runDebugAutomation({ parameters: {
    pawsEchoProxy: 'proxy', pawsEchoUrl: 'tcp://example.test:1234', pawsExpectEchoFailure: true,
  } }, true, async () => {}), /unexpectedly succeeded/);
});

test('automation cannot replace an unreadable runtime configuration with VPN defaults', async () => {
  let starts = 0;
  const api = automationFixture({ querySnapshot() { return 'corrupt'; } });
  await assert.rejects(api.runDebugAutomation(
    { parameters: { pawsAutoStartVpn: true } },
    true,
    async () => { starts++; },
  ));
  assert.equal(starts, 0);
});

test('release application ignores automation Wants while debug application accepts them', async () => {
  const calls = [];
  const api = automationFixture({
    async importProfileFromContentAndActivate() { calls.push('import'); return 'profile-a'; },
    querySnapshot() { calls.push('snapshot'); return JSON.stringify({ vpnOptions: { mtu: 1500 } }); },
    async testProxyEcho(_proxy, _url, payload) { calls.push('echo'); return payload; },
  });
  const want = { parameters: {
    pawsProfileContent: 'mixed-port: 7890\nrules:\n  - MATCH,DIRECT\n',
    pawsProfileName: 'single-transaction',
    pawsAutoStartVpn: true,
    pawsEchoProxy: 'proxy',
    pawsEchoUrl: 'tcp://example.test:1234',
  } };

  await api.runDebugAutomation(want, false, async () => { calls.push('start'); });
  assert.deepEqual(calls, [], 'release ApplicationInfo.debug=false must make the exported Want inert');

  await api.runDebugAutomation(want, true, async () => { calls.push('start'); });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(calls, ['import', 'snapshot', 'start', 'echo']);
});

test('EntryAbility gates both initial and subsequent Wants with the SDK application debug flag', async () => {
  for (const applicationDebug of [false, true]) {
    const fixture = entryAbilityFixture(applicationDebug);
    const initialWant = { parameters: { pawsAutoStartVpn: true } };
    const subsequentWant = { parameters: { pawsEchoProxy: 'proxy' } };

    await fixture.ability.onCreate(initialWant, {});
    fixture.ability.onNewWant(subsequentWant, {});
    await new Promise(resolve => setImmediate(resolve));

    assert.deepEqual(
      fixture.automationCalls.map(call => call.debug),
      [applicationDebug, applicationDebug],
    );
    assert.equal(fixture.automationCalls[0].want, initialWant);
    assert.equal(fixture.automationCalls[1].want, subsequentWant);
  }
});

async function until(predicate, description) {
  for (let turn = 0; turn < 100; turn++) {
    if (predicate()) return;
    await new Promise(resolve => setImmediate(resolve));
  }
  assert.fail(`Async operation did not reach ${description}`);
}

function extensionFixture() {
  const calls = [];
  const connections = [];
  let latestAttempt = 'attempt-a';
  const terminalAttempts = new Set();
  let activeAttempt = '';
  let nativeRunning = false;
  let protectedNetwork = false;
  const pendingEvents = deferred();
  const native = {
    configureAppHome() {},
    attachPlatformSharedMemory(ashmem, notification) { calls.push(['attach', ashmem, notification]); },
    validatePlatformVpnStartRequest(_ashmem, _notification, attempt) {
      calls.push(['validate', attempt]);
      if (attempt !== latestAttempt || terminalAttempts.has(attempt)) throw new Error('stale Want');
    },
    acknowledgeTerminalPlatformVpnStartDelivery(ashmem, notification, attempt) {
      const acknowledged = terminalAttempts.has(attempt);
      calls.push(['ack-delivery', attempt, ashmem, notification, acknowledged]);
      return acknowledged;
    },
    bindPlatformVpnStart(attempt) {
      if (attempt !== latestAttempt) throw new Error('stale attempt');
      activeAttempt = attempt;
      calls.push(['bind', attempt]);
    },
    setPlatformVpnStarting(attempt, starting) { calls.push(['starting', attempt, starting]); return attempt === activeAttempt; },
    setPlatformVpnRunning(attempt, running) { if (attempt === activeAttempt) nativeRunning = running; calls.push(['running', attempt, running]); },
    setPlatformNetworkProtected(attempt, protectedValue) { if (attempt === activeAttempt) protectedNetwork = protectedValue; calls.push(['protected', attempt, protectedValue]); return attempt === activeAttempt; },
    setPlatformVpnFailed(attempt, error) { if (attempt === activeAttempt) nativeRunning = false; calls.push(['failed', attempt, error]); },
    async prepareVpn(...args) { calls.push(['prepare', ...args]); return true; },
    async startVpn(fd, _options, ...args) {
      nativeRunning = true;
      calls.push(['native-start', activeAttempt, fd, ...args]);
    },
    async stopVpn(attempt) {
      calls.push(['native-stop', attempt]);
      if (attempt && attempt !== activeAttempt) return false;
      nativeRunning = false;
      return true;
    },
    completePlatformVpnCleanup(attempt) { calls.push(['cleanup-complete', attempt]); return attempt === activeAttempt; },
    extensionTick() {
      if (nativeRunning) return 'connected';
      return activeAttempt.length > 0 ? 'starting' : 'disconnected';
    },
    waitForPlatformChangeEvent() { return pendingEvents.promise; },
    cancelPlatformChangeWait() {},
    syncPlatformChanges() {},
    persistVpnTelemetry() {},
    snapshot() { return JSON.stringify({ traffic: { uploadSpeed: 0, downloadSpeed: 0 }, vpnRunning: nativeRunning }); },
    snapshotJson() { return this.snapshot(); },
  };
  const logging = Object.fromEntries(['info', 'warn', 'error', 'debug'].map(level => [level, (...args) => calls.push([level, ...args])]));
  const overrides = {
    '@kit.AbilityKit': { wantAgent: { getWantAgent: async () => ({}) } },
    '@kit.NetworkKit': {
      VpnExtensionAbility: class { onCreate() {} },
      vpnExtension: {
        createVpnConnection() {
          const created = deferred();
          const destroyed = deferred();
          const id = connections.length;
          const connection = {
            created,
            destroyed,
            async create(config) { calls.push(['create', id, config]); return created.promise; },
            async protectProcessNet() { calls.push(['protect', id]); },
            async destroy() { calls.push(['destroy', id]); return destroyed.promise; },
          };
          connections.push(connection);
          return connection;
        },
      },
    },
    '@kit.NotificationKit': {
      notificationManager: { publish: async () => {}, cancel: async () => {}, isNotificationEnabled: async () => false },
    },
    '@kit.PerformanceAnalysisKit': { hilog: logging },
    'libpaws_ui.so': { default: native },
    globals: { setInterval: () => 1, clearInterval() {}, setTimeout: () => 2, clearTimeout() {} },
  };
  const Extension = loadArkts('entry/src/main/ets/vpnability/PawsVpnExtensionAbility.ets', overrides).default;
  const extension = new Extension();
  extension.context = { filesDir: '/test/paws', applicationInfo: { name: 'com.example.paws.test' } };
  const config = configModule();
  const want = (attempt, fd) => config.buildVpnWant(config.DEFAULT_OPTIONS_JSON, { ashmemFd: fd, notificationFd: fd + 1 }, attempt);
  return {
    extension, connections, calls, want,
    setLatestAttempt(attempt) { latestAttempt = attempt; },
    markTerminal(attempt) { terminalAttempts.add(attempt); },
    state() { return { activeAttempt, nativeRunning, protectedNetwork }; },
  };
}

function pluginFixture({
  startBarrier,
  deferStartNumber = 1,
  recoverableSession = '',
  recoverCleanup = false,
  startOutcome = 'connected',
  startCompletionBarrier,
  cooperativeStop = true,
} = {}) {
  const calls = [];
  let currentSession = '';
  let recoverable = recoverableSession;
  let revision = '1';
  let intentEpoch = 0;
  let stopFenceEpoch = '';
  let stopFenceSession = '';
  let stopInFlight = false;
  let nextId = 0;
  let nextRequestId = 0;
  const cleanup = deferred();
  const cleanedSessions = new Set();
  let dispatchCount = 0;
  const native = {
    initializePlatformSharedMemory() { return '21,22'; },
    currentPlatformVpnSessionId() { return currentSession; },
    advancePlatformVpnIntent() { intentEpoch += 1; calls.push(['intent', `${intentEpoch}`]); return `${intentEpoch}`; },
    isPlatformVpnIntentCurrent(expected) { return expected === `${intentEpoch}`; },
    claimCurrentPlatformVpnStop(expected) {
      if (expected !== `${intentEpoch}`) throw new Error('VPN intent superseded');
      if (stopInFlight) throw new Error('VPN OS stop already in flight');
      const retained = stopFenceEpoch.length > 0;
      const claimed = retained ? stopFenceSession : (recoverable || currentSession);
      if (claimed.length > 0 || retained) {
        stopFenceEpoch = expected;
        stopFenceSession = claimed;
      }
      calls.push(['claim-stop', claimed]);
      return claimed;
    },
    isPlatformVpnStopCurrent(expected, session) {
      return expected === `${intentEpoch}` && stopFenceEpoch === expected &&
        stopFenceSession === session && !stopInFlight;
    },
    beginPlatformVpnOsStop(expected, session) {
      if (expected !== `${intentEpoch}` || stopInFlight) return false;
      if (stopFenceEpoch.length === 0) {
        if (session.length > 0) return false;
        stopFenceEpoch = expected;
        stopFenceSession = session;
      } else if (stopFenceEpoch !== expected || stopFenceSession !== session) {
        return false;
      }
      stopInFlight = true;
      calls.push(['begin-os-stop', expected, session]);
      return true;
    },
    completePlatformVpnOsStop(expected, session) {
      if (stopFenceEpoch !== expected || stopFenceSession !== session || !stopInFlight) return false;
      stopFenceEpoch = '';
      stopFenceSession = '';
      stopInFlight = false;
      calls.push(['complete-os-stop', expected, session]);
      return true;
    },
    failPlatformVpnOsStop(expected, session) {
      if (stopFenceEpoch !== expected || stopFenceSession !== session || !stopInFlight) return false;
      stopInFlight = false;
      calls.push(['fail-os-stop', expected, session]);
      return true;
    },
    async recoverPlatformVpnCleanupAfterConfirmedStop(session) {
      calls.push(['recover-cleanup', session]);
      const recovered = recoverCleanup && (recoverable.length === 0 || session === recoverable);
      if (cleanedSessions.has(session)) return true;
      if (recovered && session === recoverable) recoverable = '';
      if (recovered && session === currentSession) currentSession = '';
      return recovered;
    },
    isPlatformVpnSessionCurrent(session, expectedRevision) { return session === currentSession && (!expectedRevision || revision === expectedRevision); },
    isRuntimeConfigRevisionCurrent(expectedRevision) { return revision === expectedRevision; },
    beginPlatformVpnStartForIntent(expected) {
      if (expected !== `${intentEpoch}`) throw new Error('VPN start intent superseded');
      if (stopFenceEpoch.length > 0) throw new Error('VPN OS stop remains pending');
      currentSession = `session-${++nextId}`;
      calls.push(['begin', currentSession]);
      return currentSession;
    },
    async awaitPlatformVpnStart() {
      if (startCompletionBarrier) await startCompletionBarrier.promise;
      if (startOutcome instanceof Error) throw startOutcome;
      return startOutcome;
    },
    cancelPlatformVpnStart(session) { calls.push(['cancel', session]); },
    requestPlatformVpnStop(session) {
      calls.push(['request-stop', session]);
      return cooperativeStop && session === currentSession;
    },
    failPlatformVpnStart() {},
    failUnattachedPlatformVpnStart() {},
    async awaitPlatformVpnStop(session) {
      calls.push(['await-cleanup', session]);
      await cleanup.promise;
      cleanedSessions.add(session);
      if (currentSession === session) currentSession = '';
      return true;
    },
  };
  const { VpnPlugin } = loadArkts('entry/src/main/ets/plugins/VpnPlugin.ets', {
    '@ohos-rs/ability': { AsyncPluginBase: class { getContext() { return { abilityContext: {} }; } } },
    '@kit.AbilityKit': {},
    '@kit.NetworkKit': { vpnExtension: {
      async startVpnExtensionAbility(want) { calls.push(['start-extension', want]); if (++dispatchCount === deferStartNumber && startBarrier) await startBarrier.promise; },
      async stopVpnExtensionAbility() { calls.push(['stop-extension']); },
    } },
    '@kit.NotificationKit': { notificationManager: { async isNotificationEnabled() { return true; } } },
    '@kit.PerformanceAnalysisKit': { hilog: { info() {}, warn() {}, error() {} } },
    'libpaws_ui.so': { default: native },
  });
  const createPlugin = () => {
    const instance = new VpnPlugin();
    instance.onInstall({});
    return instance;
  };
  const plugin = createPlugin();
  const waitOnce = (operationId, waitMs = 1) => plugin.invokeAsync('await-vpn-operation', {
    typeName: 'paws.VpnOperationWaitRequest', value: { operationId, waitMs },
  }, {});
  const lookup = (requestId) => plugin.invokeAsync('lookup-vpn-operation', {
    typeName: 'paws.VpnOperationLookupRequest', value: { requestId },
  }, {});
  const waitTerminal = async (submission, resultKey) => {
    const receipt = await submission;
    assert.equal(typeof receipt.value.operationId, 'string');
    for (let turn = 0; turn < 1000; turn++) {
      const response = await waitOnce(receipt.value.operationId);
      if (response.value.status === 'pending') continue;
      if (response.value.status === 'failed') throw new Error(response.value.error);
      assert.equal(response.value.status, 'succeeded');
      return { value: { [resultKey]: response.value.result } };
    }
    assert.fail('VPN operation did not reach a terminal result');
  };
  const restart = (session = 'session-1') => waitTerminal(plugin.invokeAsync('restart-vpn', {
    typeName: 'paws.VpnRestartRequest',
    value: {
      requestId: `request-${++nextRequestId}`,
      expectedSessionId: session,
      expectedConfigRevision: '1',
      optionsJson: '{}',
    },
  }, {}), 'restarted');
  const stopOwned = (session = 'session-1', expectedRevision = '1') =>
    waitTerminal(plugin.invokeAsync('stop-vpn-if-current', {
      typeName: 'paws.VpnOwnedStopRequest',
      value: {
        requestId: `request-${++nextRequestId}`,
        expectedSessionId: session,
        expectedConfigRevision: expectedRevision,
      },
    }, {}), 'stopped');
  return {
    plugin,
    calls,
    cleanup,
    restart,
    stopOwned,
    waitOnce,
    lookup,
    createPlugin,
    advanceIntent() { return native.advancePlatformVpnIntent(); },
    setRevision(value) { revision = value; },
  };
}

test('bridge submits a VPN operation once and observes typed pending without retrying it', async () => {
  const startBarrier = deferred();
  const startCompletionBarrier = deferred();
  const fixture = pluginFixture({ startBarrier, startCompletionBarrier });
  const receipt = await fixture.plugin.invokeAsync('start-vpn', {
    typeName: 'paws.VpnStartRequest', value: { requestId: 'bridge-pending', optionsJson: '{}' },
  }, {});
  assert.equal(typeof receipt.value.operationId, 'string');
  const duplicate = await fixture.plugin.invokeAsync('start-vpn', {
    typeName: 'paws.VpnStartRequest', value: { requestId: 'bridge-pending', optionsJson: '{}' },
  }, {});
  assert.equal(duplicate.value.operationId, receipt.value.operationId,
    'same requestId and payload must return the original receipt');
  await assert.rejects(
    fixture.plugin.invokeAsync('start-vpn', {
      typeName: 'paws.VpnStartRequest',
      value: { requestId: 'bridge-pending', optionsJson: '{"different":true}' },
    }, {}),
    /reused with a different request/,
  );
  await until(() => fixture.calls.filter(call => call[0] === 'start-extension').length === 1,
    'submitted extension start');
  const pending = await fixture.waitOnce(receipt.value.operationId);
  assert.equal(pending.value.status, 'pending');
  const pendingLookup = await fixture.lookup('bridge-pending');
  assert.equal(pendingLookup.value.status, 'found');
  assert.equal(pendingLookup.value.operationId, receipt.value.operationId);
  assert.equal(pendingLookup.value.operationStatus, 'pending');
  assert.equal(fixture.calls.filter(call => call[0] === 'start-extension').length, 1,
    'observing pending must not resubmit the platform request');

  const foreign = pluginFixture();
  const unavailable = await foreign.waitOnce(receipt.value.operationId);
  assert.equal(unavailable.value.status, 'unavailable',
    'a replacement Ability session must not claim an old operation id');
  const unknownLookup = await foreign.lookup('bridge-pending');
  assert.equal(unknownLookup.value.status, 'unknown-session');

  startBarrier.resolve();
  startCompletionBarrier.resolve();
  let terminal;
  do {
    terminal = await fixture.waitOnce(receipt.value.operationId);
  } while (terminal.value.status === 'pending');
  assert.equal(terminal.value.status, 'succeeded');
  const repeatedTerminal = await fixture.waitOnce(receipt.value.operationId);
  assert.deepEqual(repeatedTerminal.value, terminal.value,
    'terminal receipt must remain repeat-readable until capacity reclamation');
  const terminalLookup = await fixture.lookup('bridge-pending');
  assert.equal(terminalLookup.value.operationStatus, 'succeeded');
  assert.equal(terminalLookup.value.result, true);
  assert.equal(fixture.calls.filter(call => call[0] === 'start-extension').length, 2,
    'only the intentional authorization redispatch may invoke start twice');
});

test('bridge reports platform failure as typed terminal state', async () => {
  const fixture = pluginFixture({ startOutcome: new Error('native terminal failure') });
  const receipt = await fixture.plugin.invokeAsync('start-vpn', {
    typeName: 'paws.VpnStartRequest', value: { requestId: 'bridge-failure', optionsJson: '{}' },
  }, {});
  let terminal;
  do {
    terminal = await fixture.waitOnce(receipt.value.operationId);
  } while (terminal.value.status === 'pending');
  assert.equal(terminal.value.status, 'failed');
  assert.match(terminal.value.error, /native terminal failure/);
});

test('owned stop requires both the session and configuration revision', async () => {
  const fixture = pluginFixture();
  await fixture.plugin.requestStartVpn('{}');
  assert.equal((await fixture.stopOwned('session-old')).value.stopped, false);
  assert.equal((await fixture.stopOwned('session-1', '0')).value.stopped, false);
  assert.equal(fixture.calls.filter(call => call[0] === 'stop-extension').length, 0);
  const stop = fixture.stopOwned();
  await until(() => fixture.calls.some(call => call[0] === 'await-cleanup'), 'owned cleanup');
  fixture.cleanup.resolve();
  assert.equal((await stop).value.stopped, true, JSON.stringify(fixture.calls));
});

test('a newer intent supersedes an owned stop before its queue head', async () => {
  const fixture = pluginFixture();
  await fixture.plugin.requestStartVpn('{}');
  const queueBarrier = deferred();
  fixture.plugin.operationChain = queueBarrier.promise;
  const stopped = fixture.stopOwned();
  fixture.advanceIntent();
  queueBarrier.resolve();
  assert.equal((await stopped).value.stopped, false);
  assert.equal(fixture.calls.filter(call => call[0] === 'stop-extension').length, 0);
});

test('watchdog orphan recovery confirms the OS stop before replacement start', async () => {
  const fixture = pluginFixture({ recoverableSession: 'dead-session', recoverCleanup: true });
  await fixture.plugin.requestStartVpn('{}');
  const stop = fixture.calls.findIndex(call => call[0] === 'stop-extension');
  const recovery = fixture.calls.findIndex(call => call[0] === 'recover-cleanup');
  const begin = fixture.calls.findIndex(call => call[0] === 'begin');
  assert.ok(stop >= 0);
  assert.ok(stop < recovery);
  assert.ok(recovery < begin);
  assert.equal(fixture.calls.some(call => call[0] === 'await-cleanup'), false);
});

for (const deferStartNumber of [1, 2]) {
  test(`user stop ignores unresolved platform start dispatch ${deferStartNumber}`, async () => {
    const startBarrier = deferred();
    const fixture = pluginFixture({ startBarrier, deferStartNumber });
    await fixture.plugin.requestStartVpn('{}');
    await until(() => fixture.calls.filter(call => call[0] === 'start-extension').length === deferStartNumber, 'deferred dispatch');
    const stopped = fixture.plugin.requestStopVpn();
    await until(() => fixture.calls.some(call => call[0] === 'await-cleanup'), 'cooperative stop request');
    fixture.cleanup.resolve();
    await stopped;
    assert.equal(fixture.calls.filter(call => call[0] === 'stop-extension').length, 1);
    startBarrier.resolve();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(fixture.calls.filter(call => call[0] === 'start-extension').length, deferStartNumber);
  });
}

test('attached VPN can stop when HarmonyOS leaves its start Promise pending', async () => {
  const neverSettledStart = deferred();
  const fixture = pluginFixture({ startBarrier: neverSettledStart });
  await fixture.plugin.requestStartVpn('{}');
  const stopped = fixture.plugin.requestStopVpn();
  await until(() => fixture.calls.some(call => call[0] === 'await-cleanup'),
    'cooperative cleanup while Extension is alive');
  fixture.cleanup.resolve();
  await stopped;
  assert.equal(fixture.calls.some(call => call[0] === 'await-attach'), false);
  assert.equal(fixture.calls.some(call => call[0] === 'stop-extension'), true);
  assert.equal(fixture.calls.filter(call => call[0] === 'start-extension').length, 1,
    'stop must not wait for or resubmit the unresolved system start');
});

test('delivered terminal Want can stop and recover cleanup with start Promise pending', async () => {
  const neverSettledStart = deferred();
  const fixture = pluginFixture({
    startBarrier: neverSettledStart,
    recoverCleanup: true,
    cooperativeStop: false,
  });
  await fixture.plugin.requestStartVpn('{}');
  const stopped = fixture.plugin.requestStopVpn();
  await until(() => fixture.calls.some(call => call[0] === 'recover-cleanup'),
    'confirmed-stop cleanup recovery');
  await stopped;

  assert.equal(fixture.calls.filter(call => call[0] === 'stop-extension').length, 1);
  assert.equal(fixture.calls.some(call => call[0] === 'await-cleanup'), false,
    'delivered owner is recovered only after the OS stop resolves');
});

test('terminal unattached attempt is fenced without waiting for platform dispatch', async () => {
  const startBarrier = deferred();
  const fixture = pluginFixture({
    startBarrier,
    recoverCleanup: true,
    cooperativeStop: false,
  });
  await fixture.plugin.requestStartVpn('{}');
  const stopped = fixture.plugin.requestStopVpn();
  await stopped;
  assert.equal(fixture.calls.filter(call => call[0] === 'stop-extension').length, 1,
    'journal fence makes an unresolved start Promise irrelevant to stop');
  startBarrier.resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.calls.filter(call => call[0] === 'start-extension').length, 1,
    'late system acknowledgement cannot redispatch a superseded intent');
});

test('scoped stop cannot cross a newer global VPN intent', async () => {
  const fixture = pluginFixture();
  await fixture.plugin.requestStartVpn('{}');
  const queueBarrier = deferred();
  fixture.plugin.operationChain = queueBarrier.promise;
  const stop = fixture.stopOwned();
  fixture.advanceIntent();
  queueBarrier.resolve();
  assert.equal((await stop).value.stopped, false);
  assert.equal(fixture.calls.filter(call => call[0] === 'stop-extension').length, 0);
});

test('a queued Stop from an old Plugin cannot OS-stop a newer Plugin session', async () => {
  const fixture = pluginFixture({ cooperativeStop: false, recoverCleanup: true });
  await fixture.plugin.requestStartVpn('{}');
  const queueBarrier = deferred();
  fixture.plugin.operationChain = queueBarrier.promise;
  const stopReceipt = await fixture.plugin.invokeAsync('stop-vpn', {
    typeName: 'paws.VpnStopRequest',
    value: { requestId: 'stale-queued-stop' },
  }, {});
  const replacement = fixture.createPlugin().requestStartVpn('{}');
  queueBarrier.resolve();
  await replacement;

  const terminal = await fixture.waitOnce(stopReceipt.value.operationId);
  assert.equal(terminal.value.status, 'succeeded');
  assert.equal(terminal.value.result, false,
    'the stale Stop receipt must not be reported as authoritative success');
  assert.equal(fixture.calls.filter(call => call[0] === 'stop-extension').length, 1,
    'only the replacement intent may dispatch the OS stop');
  assert.deepEqual(
    fixture.calls.find(call => call[0] === 'begin-os-stop'),
    ['begin-os-stop', '3', 'session-1'],
  );
  const stopIndex = fixture.calls.findIndex(call => call[0] === 'stop-extension');
  const replacementBegin = fixture.calls.findIndex(call => call[0] === 'begin' && call[1] === 'session-2');
  assert.ok(stopIndex >= 0 && stopIndex < replacementBegin,
    'the retained stop fence must be confirmed before replacement begin');
});

test('a newer Plugin takes over Stop while the old Plugin awaits cooperative cleanup', async () => {
  const fixture = pluginFixture({ recoverCleanup: true });
  await fixture.plugin.requestStartVpn('{}');
  const stopReceipt = await fixture.plugin.invokeAsync('stop-vpn', {
    typeName: 'paws.VpnStopRequest',
    value: { requestId: 'stale-cooperative-stop' },
  }, {});
  await until(() => fixture.calls.some(call => call[0] === 'await-cleanup'),
    'old Plugin cooperative cleanup wait');
  const replacement = fixture.createPlugin().requestStartVpn('{}');
  fixture.cleanup.resolve();
  await replacement;

  const terminal = await fixture.waitOnce(stopReceipt.value.operationId);
  assert.equal(terminal.value.status, 'succeeded');
  assert.equal(terminal.value.result, false,
    'superseded cooperative Stop must remain a false receipt');
  assert.equal(fixture.calls.filter(call => call[0] === 'stop-extension').length, 1,
    'only the newer Plugin may dispatch after taking over the exact fence');
  assert.deepEqual(
    fixture.calls.find(call => call[0] === 'begin-os-stop'),
    ['begin-os-stop', '3', 'session-1'],
  );
  const completedFence = fixture.calls.findIndex(call => call[0] === 'complete-os-stop');
  const replacementBegin = fixture.calls.findIndex(call => call[0] === 'begin' && call[1] === 'session-2');
  assert.ok(completedFence >= 0 && completedFence < replacementBegin,
    'cleanup ACK alone must not release the fence before OS stop confirmation');
});

test('restart waits for platform cleanup and a user stop wins over saved settings', async () => {
  const fixture = pluginFixture();
  await fixture.plugin.requestStartVpn('{}');
  const restart = fixture.restart();
  await until(() => fixture.calls.some(call => call[0] === 'await-cleanup'), 'restart cleanup barrier');
  assert.equal(fixture.calls.filter(call => call[0] === 'begin').length, 1);
  const stop = fixture.plugin.requestStopVpn();
  fixture.cleanup.resolve();
  const response = await restart;
  await stop;
  assert.equal(response.value.restarted, false);
  assert.equal(fixture.calls.filter(call => call[0] === 'begin').length, 1);
});

test('queued restart cannot apply a superseded configuration revision', async () => {
  const fixture = pluginFixture();
  await fixture.plugin.requestStartVpn('{}');
  const restart = fixture.restart();
  await until(() => fixture.calls.some(call => call[0] === 'await-cleanup'), 'restart cleanup barrier');
  fixture.setRevision('2');
  fixture.cleanup.resolve();
  assert.equal((await restart).value.restarted, false);
  assert.equal(fixture.calls.filter(call => call[0] === 'begin').length, 1);
});

test('stale session restart does not touch the active platform connection', async () => {
  const fixture = pluginFixture();
  await fixture.plugin.requestStartVpn('{}');
  assert.equal((await fixture.restart('session-old')).value.restarted, false);
  assert.equal(fixture.calls.filter(call => call[0] === 'stop-extension').length, 0);
});

test('a stale Want is rejected before replacing the current IPC binding', async () => {
  const fixture = extensionFixture();
  fixture.extension.onCreate(fixture.want('attempt-a', 21));
  await until(() => fixture.calls.some(call => call[0] === 'create'), 'first create');
  const attachedBefore = fixture.calls.filter(call => call[0] === 'attach').length;
  fixture.extension.onRequest(fixture.want('attempt-stale', 91), 2);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.calls.filter(call => call[0] === 'attach').length, attachedBefore);
  assert.equal(fixture.calls.filter(call => call[0] === 'destroy').length, 0);
  assert.equal(fixture.state().activeAttempt, 'attempt-a');
});

test('a bound pending owner proceeds to platform create while native lifecycle is starting', async () => {
  const fixture = extensionFixture();
  fixture.extension.onCreate(fixture.want('attempt-a', 21));
  await until(() => fixture.connections.length === 1, 'pending owner platform create');

  assert.equal(fixture.calls.filter(call => call[0] === 'bind').length, 1);
  assert.equal(fixture.calls.filter(call => call[0] === 'create').length, 1);
  assert.equal(fixture.calls.filter(call => call[0] === 'destroy').length, 0);
  assert.equal(fixture.state().activeAttempt, 'attempt-a');
});

test('a late terminal Want acknowledges exact delivery without attaching or reviving it', async () => {
  const fixture = extensionFixture();
  fixture.markTerminal('attempt-a');
  fixture.extension.onCreate(fixture.want('attempt-a', 21));
  await new Promise(resolve => setImmediate(resolve));

  assert.deepEqual(
    fixture.calls.find(call => call[0] === 'ack-delivery'),
    ['ack-delivery', 'attempt-a', 21, 22, true],
  );
  assert.equal(fixture.calls.some(call => call[0] === 'attach'), false);
  assert.equal(fixture.calls.some(call => call[0] === 'bind'), false);
  assert.equal(fixture.connections.length, 0);
  assert.equal(fixture.state().nativeRunning, false);
});

test('delivery of an old terminal Want cannot replace a newer Extension owner', async () => {
  const fixture = extensionFixture();
  fixture.setLatestAttempt('attempt-b');
  fixture.extension.onCreate(fixture.want('attempt-b', 31));
  await until(() => fixture.connections.length === 1, 'new owner platform connection');
  fixture.markTerminal('attempt-a');
  fixture.extension.onRequest(fixture.want('attempt-a', 21), 2);
  await new Promise(resolve => setImmediate(resolve));

  assert.deepEqual(
    fixture.calls.find(call => call[0] === 'ack-delivery' && call[1] === 'attempt-a'),
    ['ack-delivery', 'attempt-a', 21, 22, true],
  );
  assert.equal(fixture.calls.filter(call => call[0] === 'attach').length, 1);
  assert.equal(fixture.calls.filter(call => call[0] === 'bind').length, 1);
  assert.equal(fixture.calls.filter(call => call[0] === 'destroy').length, 0);
  assert.equal(fixture.state().activeAttempt, 'attempt-b');
});

test('replacement waits for old create and destroy and never starts the stale native session', async () => {
  const fixture = extensionFixture();
  fixture.extension.onCreate(fixture.want('attempt-a', 21));
  await until(() => fixture.connections.length === 1, 'first platform connection');
  fixture.setLatestAttempt('attempt-b');
  fixture.extension.onRequest(fixture.want('attempt-b', 31), 2);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.connections.length, 1, 'new create must wait for unresolved old create');

  fixture.connections[0].created.resolve(50);
  await until(() => fixture.calls.some(call => call[0] === 'destroy' && call[1] === 0), 'old connection cleanup');
  assert.equal(fixture.connections.length, 1, 'new create must wait for old destroy');
  assert.equal(fixture.calls.filter(call => call[0] === 'native-start' && call[1] === 'attempt-a').length, 0);

  fixture.connections[0].destroyed.resolve();
  await until(() => fixture.connections.length === 2, 'replacement connection');
  fixture.connections[1].created.resolve(60);
  await until(() => fixture.calls.some(call => call[0] === 'native-start' && call[1] === 'attempt-b'), 'new native session');
  assert.equal(fixture.state().activeAttempt, 'attempt-b');
  assert.equal(fixture.state().nativeRunning, true);
  assert.equal(fixture.calls.filter(call => call[0] === 'destroy' && call[1] === 1).length, 0);
});

test('authorization redispatch of the same attempt does not cancel pending startup', async () => {
  const fixture = extensionFixture();
  fixture.extension.onCreate(fixture.want('attempt-a', 21));
  await until(() => fixture.connections.length === 1, 'first platform connection');
  fixture.extension.onRequest(fixture.want('attempt-a', 21), 2);
  fixture.connections[0].created.resolve(50);
  await until(() => fixture.calls.some(call => call[0] === 'native-start'), 'same-attempt native startup');
  assert.equal(fixture.connections.length, 1);
  assert.equal(fixture.calls.filter(call => call[0] === 'destroy').length, 0);
  assert.equal(fixture.state().nativeRunning, true);
});

test('destroy during a pending create cannot resurrect the native VPN', async () => {
  const fixture = extensionFixture();
  fixture.extension.onCreate(fixture.want('attempt-a', 21));
  await until(() => fixture.connections.length === 1, 'pending create');
  fixture.extension.onDestroy();
  fixture.connections[0].created.resolve(50);
  await until(() => fixture.calls.some(call => call[0] === 'destroy'), 'late-created resource cleanup');
  fixture.connections[0].destroyed.resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.calls.filter(call => call[0] === 'native-start').length, 0);
  assert.equal(fixture.state().nativeRunning, false);
});

test('a failed platform destroy prevents replacement from creating another tunnel', async () => {
  const fixture = extensionFixture();
  fixture.extension.onCreate(fixture.want('attempt-a', 21));
  await until(() => fixture.connections.length === 1, 'first connection');
  fixture.connections[0].created.resolve(50);
  await until(() => fixture.calls.some(call => call[0] === 'native-start'), 'first native session');
  fixture.setLatestAttempt('attempt-b');
  fixture.extension.onRequest(fixture.want('attempt-b', 31), 2);
  await until(() => fixture.calls.some(call => call[0] === 'destroy'), 'replacement cleanup');
  fixture.connections[0].destroyed.reject(new Error('platform destroy rejected'));
  await new Promise(resolve => setImmediate(resolve));
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.connections.length, 1, 'failed cleanup must retain the replacement barrier');
  assert.equal(fixture.calls.filter(call => call[0] === 'native-start' && call[1] === 'attempt-b').length, 0);
});
