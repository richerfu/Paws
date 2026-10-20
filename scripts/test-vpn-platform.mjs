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
  process.env.DEVECO_STUDIO_HOME && resolve(process.env.DEVECO_STUDIO_HOME, 'sdk/default/openharmony/ets/build-tools/ets-loader/node_modules/typescript'),
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
      if (name === '@kit.LiveViewKit') return { liveViewManager: { async isLiveViewEnabled() { return false; } } };
      if (name === '@kit.FormKit') return { formBindingData: { createFormBindingData(v) { return v; } }, formProvider: { async updateForm() {} } };
      if (name === '@kit.CoreFileKit') return { fileIo: { statSync() { throw new Error('missing fixture journal'); }, openSync() { throw new Error('fixture filesystem unavailable'); } } };
      if (name === '@kit.BasicServicesKit') return { commonEventManager: {} };
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
    '@kit.AbilityKit': {},
  });
  const plugin = new UrlPlugin();
  assert.equal(plugin.id, 'ohos.url');
  const opened = [];
  const context = { abilityContext: { async openLink(url) { opened.push(url); } }, isActive: () => true };
  const about = readFileSync(resolve(root, 'crates/paws_ui/src/view/pages/tools.rs'), 'utf8')
    .split('pub(crate) fn about_page')[1].split('pub(crate) fn privacy_page')[0];
  const links = [...about.matchAll(/\b(?:source_url|license_url|url): "(https:\/\/[^\"]+)"/g)].map(match => match[1]);
  assert.equal(links.length, 10, 'exercise every production source, license and attribution link');
  assert.ok(links.some(url => url.includes('chen08209/Clash.Meta')));
  assert.ok(links.some(url => url.includes('SagerNet/sing-box')));
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

test('ANCO container selection uses the device-wide compatibility route', () => {
  const api = configModule();
  const regular = new api.PawsVpnConfig({
    ...api.parseOptions(api.DEFAULT_OPTIONS_JSON),
    trustedApplications: ['com.example.browser'],
  });
  assert.deepEqual(Array.from(regular.trustedApplications), ['com.example.browser']);

  for (const bundleName of ['com.easy.hmos.abroad', 'com.huawei.shell_assistant']) {
    const anco = new api.PawsVpnConfig({
      ...api.parseOptions(api.DEFAULT_OPTIONS_JSON),
      trustedApplications: [bundleName],
    });
    assert.equal(anco.trustedApplications, undefined);
  }
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

test('VPN data transfer task publishes real progress without owning VPN lifetime', async () => {
  const events = [];
  let cancellationListener;
  let timerCleared = false;
  const backgroundTaskManager = {
    on(type, listener) { events.push(['on', type]); cancellationListener = listener; },
    off(type) { events.push(['off', type]); },
    async startBackgroundRunning(context, modes, agent) {
      events.push(['start', context, modes, agent]);
      return { notificationId: 17043, continuousTaskId: 23, slotType: 4, contentType: 5 };
    },
    async stopBackgroundRunning(context) { events.push(['stop', context]); },
  };
  const published = [];
  const { VpnContinuousTask } = loadArkts(
    'entry/src/main/ets/vpnability/VpnContinuousTask.ets', {
      globals: {
        setInterval() { return 71; },
        clearInterval(id) { if (id === 71) timerCleared = true; },
      },
      '@kit.AbilityKit': {
        common: {},
        wantAgent: {
          OperationType: { START_ABILITY: 1 },
          WantAgentFlags: { UPDATE_PRESENT_FLAG: 2 },
          async getWantAgent(info) { return { info }; },
        },
      },
      '@kit.BackgroundTasksKit': { backgroundTaskManager },
      '@kit.NotificationKit': {
        notificationManager: {
          SlotType: { LIVE_VIEW: 4 },
          ContentType: { NOTIFICATION_CONTENT_SYSTEM_LIVE_VIEW: 5 },
          async publish(request) { published.push(request); },
        },
      },
      '@kit.PerformanceAnalysisKit': { hilog: { info() {}, warn() {} } },
      'libpaws_ui.so': { default: { queryVpnSpeed() { return JSON.stringify({ traffic: {
        downloadBytes: 20 * 1024 * 1024,
        uploadBytes: 5 * 1024 * 1024,
        downloadSpeed: 2048,
        uploadSpeed: 1024,
      } }); } } },
      './VpnConfig': {
        currentPawsBundleName() { return 'com.example.paws'; },
        PAWS_ENTRY_ABILITY: 'EntryAbility',
        PAWS_MODULE_NAME: 'entry',
      },
    },
  );
  const task = new VpnContinuousTask();
  const cancellations = [];
  const context = { abilityInfo: { name: 'EntryAbility' } };
  await task.start(context, info => { cancellations.push(info); });
  assert.deepEqual(Array.from(events.find(event => event[0] === 'start')[2]), ['dataTransfer']);
  assert.equal(published.length, 1);
  assert.equal(published[0].id, 17043);
  assert.equal(published[0].notificationSlotType, 4);
  assert.equal(published[0].content.systemLiveView.typeCode, 8);
  assert.equal(published[0].template.name, 'downloadTemplate');
  assert.equal(published[0].template.data.progressValue, 25);
  cancellationListener({ id: 999, reason: 1 });
  assert.equal(cancellations.length, 0, 'another continuous task cannot affect this VPN');
  cancellationListener({ id: 23, reason: 4, detailedReason: 4 });
  assert.deepEqual(cancellations, [{ id: 23, reason: 4, detailedReason: 4 }]);
  assert.equal(timerCleared, true);

  const pluginSource = readFileSync(resolve(root, 'entry/src/main/ets/plugins/VpnPlugin.ets'), 'utf8');
  assert.match(pluginSource, /continuous task protection ended; VPN remains active/);
  assert.doesNotMatch(pluginSource, /system cancelled continuous VPN task; stopping VPN/);

  const manifest = readFileSync(resolve(root, 'entry/src/main/module.json5'), 'utf8');
  assert.match(manifest, /"backgroundModes"\s*:\s*\[\s*"dataTransfer"\s*\]/);
  assert.match(manifest, /"ohos\.permission\.KEEP_BACKGROUND_RUNNING"/);
});

test('first VPN authorization hands off descriptors while system start is pending', async () => {
  const systemStart = deferred();
  const events = [];
  const { startVpnExtensionWithHandoff } = loadArkts(
    'entry/src/main/ets/vpnability/VpnExtensionLauncher.ets', {
      '@kit.NetworkKit': { vpnExtension: {
        startVpnExtensionAbility(want) {
          events.push(['start', want.parameters.pawsStartAttemptId]);
          return systemStart.promise;
        },
      } },
      '@kit.PerformanceAnalysisKit': { hilog: { info() {}, warn() {} } },
      'libpaws_ui.so': { default: {
        preparePlatformVpnHandoff() { return '0123456789abcdef0123456789abcdef'; },
        async sendPlatformVpnHandoff(_ashmem, _notification, attemptId) {
          events.push(['handoff', attemptId]);
        },
      } },
    },
  );
  await startVpnExtensionWithHandoff('{}', 'first-auth', { ashmemFd: 21, notificationFd: 22 });
  assert.deepEqual(events, [['start', 'first-auth'], ['handoff', 'first-auth']]);
  systemStart.resolve();
});

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

function entryAbilityFixture(applicationDebug, physicalLookup = async () => ({netId:1})) {
  const automationCalls = [];
  const vpnCalls = [];
  const foregroundCalls = [];
  class NativeAbility {
    context = {
      applicationInfo: { debug: applicationDebug, name: 'com.example.paws' },
      config: {},
      filesDir: '/data/paws',
      resourceManager: {},
    };

    lifecycleTail = Promise.resolve();
    async onCreate() {}
    enqueueLifecycleOperation(label, operation) { this.lifecycleTail = this.lifecycleTail.then(operation); return this.lifecycleTail; }
    onNewWant() {}
    onForeground() {}
  }
  class LazyPlugin {
    constructor(factory) { this.factory = factory; }
  }
  class EmptyPlugin {}
  class InstalledVpnPlugin {
    constructor(installed) { this.installed = installed; }
    onInstall() { this.installed(); }
    onAbilityForeground() { foregroundCalls.push(this); }
    async requestStartVpn(options) { vpnCalls.push(options); }
  }
  const { default: EntryAbility } = loadArkts(
    'entry/src/main/ets/entryability/EntryAbility.ets',
    {
      '@kit.AbilityKit': {
        ConfigurationConstant: { ColorMode: { COLOR_MODE_NOT_SET: -1 } },
      },
      '@kit.LocalizationKit': { i18n: { System: { getSystemLanguage() { return 'en-US'; } } } },
      '@kit.PerformanceAnalysisKit': { hilog: { info() {}, error() {}, warn() {} } },
      '@ohos-rs/ability': { LazyPlugin, NativeAbility },
      '@ohos-rs/ability-plugin-files': { FilesPlugin: EmptyPlugin },
      '@ohos-rs/ability-plugin-url': { UrlPlugin: EmptyPlugin },
      '@ohos.window': {},
      'libpaws_ui.so': { default: {
        configureAppHome() {},
        setPhysicalNetwork() {},
        configureUiLocale() {},
        configureSystemColorMode() {},
        seedGeodataFromRawfiles() { return 0; },
      } },
      '../common/Errors': { describeError(error) { return String(error); } },
      '../vpnability/VpnConfig': { setPawsBundleName() {} },
      '../vpnability/UnderlyingNetwork': { getPhysicalNetwork: physicalLookup },
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
      '../plugins/VpnPlugin': { VpnPlugin: InstalledVpnPlugin },
    },
  );
  return { ability: new EntryAbility(), automationCalls, vpnCalls, foregroundCalls };
}

test('cold-start debug VPN waits for plugin installation after factory creation', async () => {
  const fixture = entryAbilityFixture(true);
  const plugin = fixture.ability.bridgePlugins[2].factory();
  const start = fixture.ability.requestStartVpn('{}');
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(fixture.vpnCalls, []);
  plugin.onInstall();
  await start;
  assert.deepEqual(fixture.vpnCalls, ['{}']);
});

test('foreground and new Want wakeups reach the current VPN plugin instance', async () => {
  const fixture = entryAbilityFixture(false);
  fixture.ability.onForeground();
  assert.deepEqual(fixture.foregroundCalls, []);

  const initial = fixture.ability.bridgePlugins[2].factory();
  initial.onInstall();
  fixture.ability.onForeground();
  fixture.ability.onNewWant({}, {});
  await fixture.ability.lifecycleTail;

  const replacement = fixture.ability.bridgePlugins[2].factory();
  replacement.onInstall();
  fixture.ability.onForeground();
  assert.deepEqual(fixture.foregroundCalls, [initial, initial, replacement]);
});

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

test('HTTP regression Want reaches native acceptance only in a debug application', async () => {
  let checks = 0;
  const api = automationFixture({ async verifyAppHttpRegression() { checks++; return '{"ipDetected":true}'; } });
  const want = { parameters: { pawsVerifyAppHttp: 'true' } };
  await api.runDebugAutomation(want, false, async () => {});
  assert.equal(checks, 0);
  await api.runDebugAutomation(want, true, async () => {});
  assert.equal(checks, 1);
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

function extensionFixture({ clock, engine = 'meow' } = {}) {
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
    setPhysicalNetwork(netId) { calls.push(["physical-network",netId]); },
    async persistEngineSession() {},
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
    prepareMihomoVpn(...args) { calls.push(['prepare-mihomo', ...args]); return JSON.stringify({ initParams: '{}', setupParams: '{}' }); },
    resolveVpnEngine() { return engine; },
    startExternalVpn(...args) { nativeRunning = true; calls.push(['external-start', ...args]); },
    stopSingBox() { calls.push(['sing-box-stop']); },
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
      if (terminalAttempts.has(activeAttempt)) return 'stopping';
      if (nativeRunning) return 'connected';
      return activeAttempt.length > 0 ? 'starting' : 'disconnected';
    },
    waitForPlatformChangeEvent() { return pendingEvents.promise; },
    cancelPlatformChangeWait() {},
    syncPlatformChanges() {},
    persistVpnTelemetry() {},
    updateSingBoxVpnTraffic() { calls.push(['sing-box-traffic']); },
    updateExternalVpnTraffic(frame) { calls.push(['external-traffic', JSON.parse(frame)]); },
    queryVpnSpeed() { calls.push(['speed-only']); return JSON.stringify({ traffic: { uploadSpeed: 1024, downloadSpeed: 2048 } }); },
    queryTrafficSpeed() { calls.push(['speed-snapshot']); return JSON.stringify({ uploadSpeed: 0, downloadSpeed: 0 }); },
    snapshot() { return JSON.stringify({ traffic: { uploadSpeed: 0, downloadSpeed: 0 }, vpnRunning: nativeRunning }); },
    snapshotJson() { return this.snapshot(); },
  };
  const logging = Object.fromEntries(['info', 'warn', 'error', 'debug'].map(level => [level, (...args) => calls.push([level, ...args])]));
  const overrides = {
    '@kit.AbilityKit': {
      bundleManager: { getBundleNameByUidSync() { return ''; } },
      wantAgent: { getWantAgent: async () => ({}), OperationType: { START_ABILITY: 1 }, WantAgentFlags: { UPDATE_PRESENT_FLAG: 1 } },
    },
    '@kit.NetworkKit': {
      connection: { getConnectOwnerUidSync() { return -1; }, async getDefaultNet() { return {netId:1}; }, async getNetCapabilities() { return {networkCap:[12,15]}; }, async getAllNets(){ return [{netId:1}]; }, NetCap:{NET_CAPABILITY_INTERNET:12,NET_CAPABILITY_NOT_VPN:15}, createNetConnection() { return { on() {}, register(cb) { if (cb) cb(); }, unregister(cb) { if (cb) cb(); } }; } },
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
            async protect() {},
            async destroy() { calls.push(['destroy', id]); return destroyed.promise; },
          };
          connections.push(connection);
          return connection;
        },
      },
    },
    '@kit.NotificationKit': {
      notificationManager: { SlotType: { SERVICE_INFORMATION: 1 }, ContentType: { NOTIFICATION_CONTENT_BASIC_TEXT: 1 }, publish: async (request) => { calls.push(['notification', request]); }, cancel: async () => {}, isNotificationEnabled: async () => false },
    },
    '@kit.PerformanceAnalysisKit': { hilog: logging },
    'libpaws_ui.so': { default: native },
    'libcore.so': {
      quickSetup(callback) { callback(''); },
      startTUN() { return true; },
      stopTun() {},
      forceGC() { calls.push(['force-gc']); },
      getTraffic() { return '{"up":0,"down":0}'; },
      getTotalTraffic() { return '{"up":0,"down":0}'; },
    },
    globals: { setInterval: () => 1, clearInterval() {}, setTimeout: () => 2, clearTimeout() {}, Date: class extends Date { static now() { return clock ? clock() : Date.now(); } } },
  };
  const Extension = loadArkts('entry/src/main/ets/vpnability/PawsVpnExtensionAbility.ets', overrides).default;
  const extension = new Extension();
  extension.context = { filesDir: '/test/paws', applicationInfo: { name: 'com.example.paws.test' } };
  const config = configModule();
  const want = (attempt, fd) => config.buildVpnWant(config.DEFAULT_OPTIONS_JSON, { ashmemFd: fd, notificationFd: fd + 1 }, attempt);
  return {
    extension, connections, calls, want, native,
    mihomo: overrides['libcore.so'],
    emitPlatformEvent() { pendingEvents.resolve(true); },
    setLatestAttempt(attempt) { latestAttempt = attempt; },
    markTerminal(attempt) { terminalAttempts.add(attempt); },
    state() { return { activeAttempt, nativeRunning, protectedNetwork }; },
  };
}

test('VPN extension telemetry never publishes a second system notification', () => {
  const fixture = extensionFixture();
  fixture.extension.persistTelemetry();
  assert.equal(fixture.calls.filter(c => c[0] === 'speed-only').length, 0);
  assert.equal(fixture.calls.filter(c => c[0] === 'notification').length, 0);
});

test('sing-box telemetry samples native counters on every tick', () => {
  const fixture = extensionFixture({ engine: 'sing-box' });
  fixture.extension.activeEngine = 'sing-box';
  for (let i = 0; i < 3; i++) fixture.extension.persistTelemetry();
  assert.equal(fixture.calls.filter(c => c[0] === 'sing-box-traffic').length, 3);
  assert.equal(fixture.calls.filter(c => c[0] === 'external-traffic').length, 0);
  fixture.native.updateSingBoxVpnTraffic = () => { throw new Error('counter unavailable'); };
  fixture.extension.persistTelemetry();
  assert.ok(fixture.calls.some(c => c[0] === 'warn' && c.some(value => String(value).includes('counter unavailable'))));
});

test('mihomo telemetry forwards native rates and totals, including idle samples', () => {
  const fixture = extensionFixture({ engine: 'mihomo' });
  fixture.extension.activeEngine = 'mihomo';
  fixture.mihomo.getTraffic = () => '{"up":1024,"down":4096}';
  fixture.mihomo.getTotalTraffic = () => '{"up":8192,"down":65536}';
  fixture.extension.persistTelemetry();
  fixture.mihomo.getTraffic = () => '{"up":0,"down":0}';
  fixture.extension.persistTelemetry();
  assert.deepEqual(fixture.calls.filter(c => c[0] === 'external-traffic').map(c => c[1]), [
    { uploadSpeed: 1024, downloadSpeed: 4096, uploadTotal: 8192, downloadTotal: 65536 },
    { uploadSpeed: 0, downloadSpeed: 0, uploadTotal: 8192, downloadTotal: 65536 },
  ]);
});

test('repeated background telemetry cannot call notification services', () => {
  const fixture = extensionFixture();
  for (let i = 0; i < 120; i++) fixture.extension.persistTelemetry();
  assert.equal(fixture.calls.filter(c => c[0] === 'speed-only').length, 0);
  assert.equal(fixture.calls.filter(c => c[0] === 'notification').length, 0);
});

function pluginFixture({
  startBarrier,
  deferStartNumber = 1,
  recoverableSession = '',
  recoverCleanup = false,
  startOutcome = 'connected',
  startCompletionBarrier,
  cooperativeStop = true,
  healthSequence = [],
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
  let pendingAutoRecovery = '';
  const native = {
    initializePlatformSharedMemory() { return '21,22'; },
    preparePlatformVpnHandoff() { return '0123456789abcdef0123456789abcdef'; },
    async sendPlatformVpnHandoff() {},
    currentPlatformVpnSessionId() { return currentSession; },
    claimPlatformVpnAutoRecovery() {
      const claimed = pendingAutoRecovery;
      pendingAutoRecovery = '';
      if (claimed.length > 0) calls.push(['claim-auto-recovery', claimed]);
      return claimed;
    },
    rearmPlatformVpnAutoRecovery(error) { calls.push(['rearm-auto-recovery', error]); },
    async validateEngineStartup(session) { calls.push(['health-probe',session]); return JSON.stringify(await (healthSequence.shift() ?? {success:true,actualEngine:'mihomo',rollbackToBaseline:false,stage:'return-traffic'})); },
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
    '../vpnability/VpnEmulatorCompatibility': {
      authorizeVpnForEmulatorDebug() {},
      canRecoverRejectedEmulatorStop() { return false; },
      isVpnEmulatorCompatibilityEnabled() { return false; },
    },
    '../vpnability/VpnContinuousTask': {
      vpnContinuousTask: {
        async start() { calls.push(['continuous-start']); },
        async stop() { calls.push(['continuous-stop']); },
      },
    },
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
    setAutoRecovery(value) { pendingAutoRecovery = value; },
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
  assert.equal(fixture.calls.filter(call => call[0] === 'start-extension').length, 1,
    'one connection intent sends one system VPN start');
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

for (const deferStartNumber of [1]) {
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

test('config preparation waits for TUN process protection before loading network resources', async () => {
  const fixture = extensionFixture();
  fixture.extension.onCreate(fixture.want('attempt-a', 21));
  await until(() => fixture.connections.length === 1, 'TUN creation');
  assert.equal(fixture.calls.some(call => call[0] === 'prepare'), false);
  const protection = deferred();
  fixture.connections[0].protectProcessNet = async () => protection.promise;
  fixture.connections[0].created.resolve(101);
  await until(() => fixture.extension.tunFd === 101, 'TUN descriptor');
  assert.equal(fixture.calls.some(call => call[0] === 'prepare'), false);
  protection.resolve();
  await until(() => fixture.calls.some(call => call[0] === 'native-start'), 'protected native start');
  const protectedAt = fixture.calls.findIndex(call => call[0] === 'protected' && call[2] === true);
  const prepareAt = fixture.calls.findIndex(call => call[0] === 'prepare');
  assert.ok(protectedAt >= 0 && prepareAt > protectedAt);
  assert.equal(fixture.calls.filter(call => call[0] === 'prepare').length, 1);
});

test('failed process protection never starts config preparation', async () => {
  const fixture = extensionFixture();
  fixture.extension.onCreate(fixture.want('attempt-a', 21));
  await until(() => fixture.connections.length === 1, 'TUN creation');
  fixture.connections[0].protectProcessNet = async () => { throw new Error('protection denied'); };
  fixture.connections[0].created.resolve(101);
  await until(() => fixture.calls.some(call => call[0] === 'failed'), 'protection failure');
  assert.equal(fixture.calls.some(call => call[0] === 'prepare'), false);
  assert.equal(fixture.calls.some(call => call[0] === 'native-start'), false);
  fixture.connections[0].destroyed.resolve();
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

test('per-app policy reaches the system VPN config and rejects conflicting lists', () => {
  const api = configModule();
  const opts = { ...JSON.parse(api.DEFAULT_OPTIONS_JSON), blockedApplications: ['com.example.bank'] };
  assert.deepEqual(Array.from(new api.PawsVpnConfig(opts).blockedApplications), ['com.example.bank']);
  assert.throws(() => new api.PawsVpnConfig({ ...opts, trustedApplications: ['com.example.browser'] }), /either/);
});

test('journal detects unclean sessions and caps retained event history', () => {
  const files = new Map([['/files/paws/lifecycle.json', JSON.stringify({ active: true, events: [] })]]);
  const io = {
    OpenMode: { WRITE_ONLY: 1, CREATE: 2, TRUNC: 4 },
    statSync(path) { return { size: files.get(path).length }; },
    readTextSync(path) { return files.get(path); },
    openSync(path) { return { fd: path }; },
    writeSync(fd, value) { files.set(fd, value); }, fsyncSync() {}, closeSync() {},
    renameSync(from, to) { files.set(to, files.get(from)); files.delete(from); },
  };
  const { LifecycleJournal } = loadArkts('entry/src/main/ets/vpnability/LifecycleJournal.ets', {
    '@kit.CoreFileKit': { fileIo: io }, '@kit.PerformanceAnalysisKit': { hilog: { warn() {} } },
  });
  const journal = new LifecycleJournal(); journal.open('/files');
  assert.match(files.get('/files/paws/lifecycle.json'), /cause-unknown/);
  for (let i = 0; i < 80; i++) journal.record('vpn-connected', true);
  assert.equal(JSON.parse(files.get('/files/paws/lifecycle.json')).events.length, 32);
  journal.record('vpn-cleanup-complete', false);
  assert.equal(JSON.parse(files.get('/files/paws/lifecycle.json')).active, false);
});

test('OS exit recorder preserves reported causes without exporting arbitrary event payloads', () => {
  const files = new Map(); let watcher;
  const io = {
    OpenMode: { WRITE_ONLY: 1, CREATE: 2, TRUNC: 4 },
    statSync(path) { if (!files.has(path)) throw Error('missing'); return { size: files.get(path).length }; },
    readTextSync(path) { return files.get(path); }, openSync(path) { return { fd: path }; },
    writeSync(fd, value) { files.set(fd, value); }, fsyncSync() {}, closeSync() {},
    renameSync(from, to) { files.set(to, files.get(from)); files.delete(from); },
  };
  const { SystemExitRecorder } = loadArkts('entry/src/main/ets/vpnability/SystemExitRecorder.ets', {
    globals: { setTimeout() { return 0; } },
    '@kit.AbilityKit': {}, '@kit.CoreFileKit': { fileIo: io },
    '@kit.PerformanceAnalysisKit': {
      hilog: { info() {}, warn() {} },
      hiAppEvent: { event: { APP_KILLED: 'APP_KILLED', APP_CRASH: 'APP_CRASH', APP_FREEZE: 'APP_FREEZE', RESOURCE_OVERLIMIT: 'RESOURCE_OVERLIMIT' },
        addWatcher(w) { watcher = w; return { setSize() {}, takeNext() { return null; } }; } },
    },
  });
  new SystemExitRecorder().start('/files', { lastExitReason: 7, lastExitMessage: 'Memory Pressure' });
  assert.equal(watcher.appEventFilters[0].domain, 'OS');
  watcher.onReceive('OS', [{ appEventInfos: [{ name: 'APP_KILLED', params: {
    reason: 'ResourceLeak(FDLeak)', time: 123, pid: 54723, secret: 'private-test-marker', external_log: '/private/log',
  } }] }]);
  const records = JSON.parse(files.get('/files/paws/system-exits.json'));
  assert.equal(records.at(-1).reason, 'ResourceLeak(FDLeak)');
  assert.equal(records.at(-1).pid, '54723');
  assert.doesNotMatch(JSON.stringify(records), /private-test-marker|external_log/);
  for (let i = 0; i < 100; i++) watcher.onReceive('OS', [{ appEventInfos: [{ name: 'APP_KILLED', params: { reason: 'test' } }] }]);
  assert.equal(JSON.parse(files.get('/files/paws/system-exits.json')).length, 64);
});

test('VPN recorder drains historical events into a separate capped file and deduplicates callbacks', () => {
  const files = new Map(); let watcher; let pending = true; let bound;
  const event = { name: 'APP_KILLED', params: { reason: 'Memory Pressure', time: 42, pid: 6632 } };
  const io = {
    OpenMode: { WRITE_ONLY: 1, CREATE: 2, TRUNC: 4 },
    statSync() { throw Error('missing'); }, readTextSync() { throw Error('missing'); },
    openSync(path) { return { fd: path }; }, writeSync(fd, value) { files.set(fd, value); }, fsyncSync() {}, closeSync() {},
    renameSync(from, to) { files.set(to, files.get(from)); files.delete(from); },
  };
  const { SystemExitRecorder } = loadArkts('entry/src/main/ets/vpnability/SystemExitRecorder.ets', {
    globals: { setTimeout() { return 0; } }, '@kit.AbilityKit': {}, '@kit.CoreFileKit': { fileIo: io },
    '@kit.PerformanceAnalysisKit': { hilog: { info() {}, warn() {} },
      hiAppEvent: { event: { APP_KILLED: 'APP_KILLED', APP_CRASH: 'APP_CRASH', APP_FREEZE: 'APP_FREEZE', RESOURCE_OVERLIMIT: 'RESOURCE_OVERLIMIT' },
        addWatcher(w) { watcher = w; return { setSize(n) { bound = n; }, takeNext() { if (!pending) return null; pending = false; return { appEventInfos: [event] }; } }; } },
    },
  });
  new SystemExitRecorder().start('/files', undefined, true);
  assert.equal(watcher.name, 'TinyPawsVpnExit'); assert.equal(bound, 65536);
  assert.ok(watcher.appEventFilters[0].names.includes('RESOURCE_OVERLIMIT'));
  assert.equal(files.has('/files/paws/system-exits.json'), false);
  watcher.onReceive('OS', [{ appEventInfos: [event] }]);
  const records = JSON.parse(files.get('/files/paws/system-exits-vpn.json'));
  assert.equal(records.length, 1); assert.equal(records[0].pid, '6632'); assert.equal(records[0].reason, 'Memory Pressure');
});


test('failed candidate health rolls back exactly once after cleaning its session', async () => {
  const fixture = pluginFixture({healthSequence:[{success:false,actualEngine:'sing-box',rollbackToBaseline:true,stage:'https'}]});
  fixture.cleanup.resolve();
  await fixture.plugin.requestStartVpn('{}');
  await until(() => fixture.calls.filter(c => c[0] === 'health-probe').length === 2, 'background baseline validation');
  assert.equal(fixture.calls.filter(c=>c[0]==='begin').length,2);
  assert.equal(fixture.calls.filter(c=>c[0]==='stop-extension').length,1);
  assert.equal(fixture.calls.filter(c=>c[0]==='health-probe').length,2);
  const cleanup=fixture.calls.findIndex(c=>c[0]==='complete-os-stop');
  const replacement=fixture.calls.findIndex(c=>c[0]==='begin'&&c[1]==='session-2');
  assert.ok(cleanup>=0&&replacement>cleanup);
});

test('user stop during candidate probe prevents the older probe from reconnecting', async () => {
  const health = deferred();
  const fixture = pluginFixture({healthSequence:[health.promise]});
  const started = fixture.plugin.requestStartVpn('{}');
  await until(()=>fixture.calls.some(c=>c[0]==='health-probe'),'candidate probe');
  fixture.cleanup.resolve();
  await fixture.plugin.requestStopVpn();
  health.resolve({success:false,actualEngine:'sing-box',rollbackToBaseline:true,stage:'https'});
  await started;
  assert.equal(fixture.calls.filter(c=>c[0]==='begin').length,1);
});

test('physical egress discovery never selects the VPN network', async()=>{
  const api=loadArkts('entry/src/main/ets/vpnability/UnderlyingNetwork.ets',{'@kit.NetworkKit':{connection:{NetCap:{NET_CAPABILITY_NOT_VPN:15,NET_CAPABILITY_INTERNET:12},async getDefaultNet(){return {netId:9}},async getAllNets(){return [{netId:9},{netId:4}]},async getNetCapabilities(net){return {networkCap:net.netId===9?[12]:[12,15]}}}}});
  assert.equal((await api.getPhysicalNetwork()).netId,4);
});

test('changing engine during an older health probe supersedes it without rolling back the new session', async () => {
  const health = deferred();
  const fixture = pluginFixture({healthSequence:[health.promise]});
  const original = fixture.plugin.requestStartVpn('{"engine":"singbox"}');
  await until(()=>fixture.calls.some(c=>c[0]==='health-probe'),'first health probe');
  fixture.cleanup.resolve();
  await fixture.plugin.requestStartVpn('{"engine":"mihomo"}');
  health.reject(new Error('old engine session replaced'));
  await original;
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.calls.filter(c=>c[0]==='begin').length,2);
  assert.equal(fixture.calls.filter(c=>c[0]==='health-probe').length,2);
});

test('confirmed VPN start settles its bridge receipt while HTTPS validation is still pending', async () => {
  const health = deferred();
  const fixture = pluginFixture({ healthSequence: [health.promise] });
  const receipt = await fixture.plugin.invokeAsync('start-vpn', {
    typeName: 'paws.VpnStartRequest', value: { requestId: 'start-before-health', optionsJson: '{}' },
  }, {});
  await until(() => fixture.calls.some(c => c[0] === 'health-probe'), 'pending health probe');
  const result = await fixture.waitOnce(receipt.value.operationId);
  assert.equal(result.value.status, 'succeeded', 'HTTPS must not keep an established tunnel in Starting');
  health.resolve({ success: true, actualEngine: 'mihomo', rollbackToBaseline: false, stage: 'return-traffic' });
});

test('user Stop during fallback cleanup prevents the older health task from restarting VPN', async () => {
  const fixture = pluginFixture({ healthSequence: [{ success: false, actualEngine: 'sing-box', rollbackToBaseline: true, stage: 'https' }] });
  await fixture.plugin.requestStartVpn('{}');
  await until(() => fixture.calls.some(c => c[0] === 'await-cleanup'), 'fallback cleanup');
  const userStop = fixture.plugin.requestStopVpn();
  fixture.cleanup.resolve();
  await userStop;
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.calls.filter(c => c[0] === 'begin').length, 1);
});

for (const engine of ['mihomo', 'sing-box']) {
  test(`${engine} Stop is handled before preparing or reloading its configuration`, async () => {
    const fixture = extensionFixture({ engine });
    fixture.native.bindPlatformVpnStart('attempt-a');
    fixture.native.startExternalVpn();
    fixture.markTerminal('attempt-a');
    fixture.extension.activeEngine = engine;
    fixture.extension.activeAttemptId = 'attempt-a';
    fixture.extension.tunFd = 31;
    fixture.extension.platformSubscriptionActive = true;
    fixture.extension.platformSubscriptionGeneration = 1;
    fixture.extension.reloadMihomoConfig = async () => { throw new Error('must not reload after Stop'); };
    fixture.extension.reloadSingBoxConfig = async () => { throw new Error('must not reload after Stop'); };
    const listening = fixture.extension.runPlatformSubscription(1);
    fixture.emitPlatformEvent();
    await listening;
    await fixture.extension.requestChain;
    assert.equal(fixture.calls.filter(c => c[0] === 'native-stop').length, 1);
    assert.equal(fixture.calls.filter(c => c[0] === 'cleanup-complete').length, 1);
    assert.equal(fixture.calls.some(c => c[0] === 'warn' && String(c).includes('config synchronization failed')), false);
  });
}

test('a Go configuration error leaves the subscription available for the next Stop event', async () => {
  const fixture = extensionFixture({ engine: 'mihomo' });
  fixture.native.bindPlatformVpnStart('attempt-a');
  fixture.native.startExternalVpn();
  fixture.extension.activeEngine = 'mihomo';
  fixture.extension.activeAttemptId = 'attempt-a';
  fixture.extension.tunFd = 31;
  fixture.extension.platformSubscriptionActive = true;
  fixture.extension.platformSubscriptionGeneration = 1;
  const update = deferred();
  const stop = deferred();
  let waits = 0;
  fixture.native.waitForPlatformChangeEvent = () => ++waits === 1 ? update.promise : stop.promise;
  fixture.extension.reloadMihomoConfig = async () => { throw new Error('configuration unavailable'); };
  const listening = fixture.extension.runPlatformSubscription(1);
  update.resolve(true);
  await until(() => waits === 2, 'subscription after config failure');
  assert.equal(fixture.extension.platformSubscriptionActive, true);
  fixture.markTerminal('attempt-a');
  stop.resolve(true);
  await listening;
  await fixture.extension.requestChain;
  assert.equal(fixture.calls.filter(c => c[0] === 'cleanup-complete').length, 1);
});


test('pending physical-network discovery never blocks the Ability lifecycle queue', async () => {
  const network = deferred();
  const fixture = entryAbilityFixture(true, () => network.promise);
  fixture.ability.onCreate({parameters:{pawsAutoStartVpn:true}},{});
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(fixture.automationCalls.length,1);
  network.resolve({netId:4});
});


test('diagnostic mode is validated and committed before VPN start; release ignores it', async () => {
  const calls = [];
  const api = automationFixture({
    async setMode(mode) { calls.push(mode); },
    querySnapshot() { calls.push('snapshot'); return JSON.stringify({vpnOptions:{}}); },
  });
  const want = {parameters:{pawsMode:'global',pawsAutoStartVpn:true,pawsWaitForVpnBeforeProbes:true}};
  await api.runDebugAutomation(want,false,async()=>calls.push('start'));
  assert.deepEqual(calls,[]);
  await api.runDebugAutomation(want,true,async()=>calls.push('start'));
  assert.deepEqual(calls,['global','snapshot','start']);
  await assert.rejects(api.runDebugAutomation({parameters:{pawsMode:'bad',pawsAutoStartVpn:true}},true,async()=>calls.push('bad-start')),/invalid diagnostic mode/);
  assert.equal(calls.includes('bad-start'),false);
});

test('diagnostic proxy selection awaits the persistent native API before reading options and starting VPN', async () => {
  const calls = [];
  const selection = deferred();
  const api = automationFixture({
    async importProfileFromContentAndActivate() { calls.push('import'); return 'local-profile'; },
    async setMode(mode) { calls.push(`mode:${mode}`); },
    async selectProxy(group, proxy) {
      assert.equal(group, 'GLOBAL');
      assert.equal(proxy, 'fixture-node');
      calls.push('select');
      await selection.promise;
    },
    querySnapshot() { calls.push('snapshot'); return JSON.stringify({vpnOptions:{}}); },
  });
  const want = {parameters:{pawsProfileContent:'rules: [MATCH,DIRECT]', pawsMode:'global',
    pawsSelectGroup:'GLOBAL', pawsSelectProxy:'fixture-node', pawsAutoStartVpn:true,
    pawsWaitForVpnBeforeProbes:true}};
  const pending = api.runDebugAutomation(want,true,async()=>calls.push('start'));
  await until(()=>calls.includes('select'),'persistent selection');
  assert.deepEqual(calls,['import','mode:global','select']);
  selection.resolve();
  await pending;
  assert.deepEqual(calls,['import','mode:global','select','snapshot','start']);
  calls.length = 0;
  await api.runDebugAutomation({parameters:{pawsSelectGroup:'GLOBAL',pawsSelectProxy:'fixture-node'}},
    true,async()=>calls.push('unexpected-start'));
  assert.deepEqual(calls,['select'],'selection-only Want must not be discarded or start VPN');
});

test('proxy selection Wants are inert in release and invalid or failed selections never start VPN', async () => {
  const calls = [];
  const api = automationFixture({
    async selectProxy() { calls.push('select'); throw new Error('fixture selection failed'); },
    querySnapshot() { calls.push('snapshot'); return JSON.stringify({vpnOptions:{}}); },
    async setMode() { calls.push('mode'); },
  });
  const start = async()=>calls.push('start');
  const valid = {pawsSelectGroup:'GLOBAL',pawsSelectProxy:'fixture-node',pawsAutoStartVpn:true};
  await api.runDebugAutomation({parameters:valid},false,start);
  assert.deepEqual(calls,[]);
  for (const parameters of [{pawsSelectGroup:'GLOBAL'}, {pawsSelectProxy:'fixture-node'},
    {pawsSelectGroup:'GLOBAL',pawsSelectProxy:''}, {pawsSelectGroup:' ',pawsSelectProxy:'fixture-node'},
    {pawsSelectGroup:true,pawsSelectProxy:true}]) {
    const want = {parameters:{...parameters,pawsMode:'global',pawsAutoStartVpn:true}};
    await api.runDebugAutomation(want,false,start);
    await assert.rejects(api.runDebugAutomation(want,true,start),/requires both pawsSelectGroup and pawsSelectProxy/);
    assert.deepEqual(calls,[],'invalid pair must fail before mode changes or native selection');
  }
  await assert.rejects(api.runDebugAutomation({parameters:valid},true,start),/fixture selection failed/);
  assert.deepEqual(calls,['select'],'a rejected selection must not read options or start VPN');
});

test('mihomo extension scavenges memory after startup and on system memory pressure', async () => {
  const fixture = extensionFixture({ engine: 'mihomo' });
  fixture.extension.onCreate(fixture.want('attempt-a', 21));
  await until(() => fixture.connections.length === 1, 'mihomo platform connection');
  fixture.connections[0].created.resolve(50);
  await until(() => fixture.calls.some(call => call[0] === 'external-start'), 'mihomo external start');
  assert.equal(fixture.calls.filter(call => call[0] === 'force-gc').length, 1);
  fixture.extension.onMemoryLevel(2);
  assert.equal(fixture.calls.filter(call => call[0] === 'force-gc').length, 2);
});

test('system wakeup claims pending auto-recovery and restarts VPN without dropping continuous task', async () => {
  const fixture = pluginFixture({ recoverCleanup: true, cooperativeStop: false });
  await fixture.plugin.requestStartVpn('{}');
  assert.equal(fixture.calls.filter(call => call[0] === 'begin').length, 1);
  fixture.setAutoRecovery('{"mtu":1500}');
  fixture.plugin.onAbilityForeground();
  await until(() => fixture.calls.filter(call => call[0] === 'begin').length === 2, 'auto-recovered VPN start');
  assert.equal(fixture.calls.filter(call => call[0] === 'stop-extension').length, 1);
  assert.equal(fixture.calls.filter(call => call[0] === 'recover-cleanup').length, 1);
  assert.equal(fixture.calls.filter(call => call[0] === 'continuous-stop').length, 0);
});


