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
  const { ABOUT_LINKS } = loadArkts('ui/nsh/src/about.ts');
  const links = ABOUT_LINKS.map(item => item.url);
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
  const lifecycleCalls = [];
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
    async onDestroy() { lifecycleCalls.push('base-destroy'); }
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
      '@nsh/ohos': { nativeShellPlugins: () => [new LazyPlugin(() => ({ id: 'nsh.arkui' }))] },
      '@ohos-rs/ability-plugin-files': { FilesPlugin: EmptyPlugin },
      '@ohos-rs/ability-plugin-url': { UrlPlugin: EmptyPlugin },
      ...Object.fromEntries(['resource', 'accessibility', 'arkui', 'clipboard', 'window', 'webview', 'video'].map(name => [
        '@ohos-rs/ability-plugin-' + name,
        { [name === 'arkui' ? 'ArkuiPlugin' : name === 'webview' ? 'WebviewPlugin' : name[0].toUpperCase() + name.slice(1) + 'Plugin']: EmptyPlugin },
      ])),
      'libnsh_platform_ohos.so': { initLogging() {} },
      '../components/PawsCoreGateway': {
        installPawsHost() {
          const vpn = new InstalledVpnPlugin();
          return { vpn, dispose() {} };
        },
      },
      '@ohos.window': { default: { AvoidAreaType: { TYPE_SYSTEM: 0, TYPE_CUTOUT: 1, TYPE_NAVIGATION_INDICATOR: 4 } } },
      'libpaws_ui.so': { default: {
        configureAppHome() {},
        setPhysicalNetwork() {},
        configureUiLocale() {},
        configureSystemColorMode() {},
        seedGeodataFromRawfiles() { return 0; },
      } },
      '../common/Errors': loadArkts('entry/src/main/ets/common/Errors.ets', { globals: { Error } }),
      '../vpnability/VpnConfig': { setPawsBundleName() {} },
      '../vpnability/UnderlyingNetwork': { getPhysicalNetwork: physicalLookup },
      '../automation/DebugAutomation': {
        runDebugAutomation(want, debug) {
          automationCalls.push({ want, debug });
          return Promise.resolve();
        },
      },
      '../plugins/VpnPlugin': { VpnPlugin: InstalledVpnPlugin },
    },
  );
  return { ability: new EntryAbility(), automationCalls, vpnCalls, foregroundCalls, lifecycleCalls };
}

function frameMetricsFixture() {
  const logs = [];
  const noTimers = () => assert.fail('frame diagnostics must not schedule timers');
  const { WindowFrameMetrics } = loadArkts('entry/src/main/ets/diagnostics/WindowFrameMetrics.ets', {
    '@kit.PerformanceAnalysisKit': { hilog: {
      info(...args) { logs.push({ level: 'info', args }); },
      warn(...args) { logs.push({ level: 'warn', args }); },
    } },
    globals: { setTimeout: noTimers, setInterval: noTimers,
      JSON: { stringify() { assert.fail('frame diagnostics must not serialize frame JSON'); } } },
  });
  return { metrics: new WindowFrameMetrics(), logs };
}

test('frame diagnostics count normal frames silently and log only first or threshold durations', () => {
  const { metrics, logs } = frameMetricsFixture();
  let callback;
  const detached = [];
  const win = {
    on(event, cb) { assert.equal(event, 'frameMetricsMeasured'); callback = cb; },
    off(event, cb) { detached.push({ event, cb }); },
  };
  metrics.start(win, false);
  assert.equal(callback, undefined, 'release applications do not subscribe');
  assert.equal(logs.length, 0);
  metrics.start(win, true);
  metrics.start(win, true);
  callback({ firstDrawFrame: true, vsyncTimestamp: 123456789000, layoutMeasureDuration: 1_000_000, inputHandlingDuration: 2_000_000 });
  assert.deepEqual(logs[0].args.slice(3), [1, 0, 1, '123456789000', '1.000', '2.000']);
  for (let i = 0; i < 1000; i++) {
    callback({ firstDrawFrame: false, vsyncTimestamp: 123456789000 + i * 10_000_000_000,
      layoutMeasureDuration: 16_699_999, inputHandlingDuration: 16_699_999 });
  }
  assert.equal(logs.length, 1, 'neither normal durations nor large timestamp gaps are reported as jank');
  callback({ firstDrawFrame: false, vsyncTimestamp: 9, layoutMeasureDuration: 16_700_000, inputHandlingDuration: 0 });
  assert.deepEqual(logs[1].args.slice(3), [1002, 1, 0, '9', '16.700', '0.000']);
  callback({ firstDrawFrame: false, vsyncTimestamp: 10, layoutMeasureDuration: 0, inputHandlingDuration: 20_000_000 });
  assert.deepEqual(logs[2].args.slice(3), [1003, 2, 0, '10', '0.000', '20.000']);
  metrics.dispose();
  assert.equal(detached.length, 1);
  assert.equal(detached[0].event, 'frameMetricsMeasured');
  assert.equal(detached[0].cb, callback, 'unsubscribe only this owner\'s exact callback');
  assert.deepEqual(logs[3].args.slice(3), [1003, 2]);
  const count = logs.length;
  callback({ firstDrawFrame: false, vsyncTimestamp: 11, layoutMeasureDuration: 100_000_000, inputHandlingDuration: 0 });
  metrics.dispose();
  metrics.start(win, true);
  assert.equal(logs.length, count, 'queued callbacks are inert after disposal');
  assert.equal(detached.length, 1);
});

test('frame diagnostic platform failures stay optional and never log exception content', () => {
  const { metrics, logs } = frameMetricsFixture();
  let failedCallback;
  metrics.start({ on(_event, cb) { failedCallback = cb; throw { code: 801, message: 'PRIVATE_CONTENT' }; },
    off() { assert.fail('a failed registration owns no subscription'); } }, true);
  assert.equal(logs.length, 1);
  assert.match(logs[0].args[2], /capability unavailable/);
  assert.deepEqual(logs[0].args.slice(3), ['register', 801]);
  failedCallback({ firstDrawFrame: true, vsyncTimestamp: 1, layoutMeasureDuration: 0, inputHandlingDuration: 0 });
  assert.equal(logs.length, 1);
  let activeCallback;
  metrics.start({ on(_event, cb) { activeCallback = cb; },
    off() { throw { code: 1300002, message: 'PRIVATE_CONTENT' }; } }, true);
  assert.doesNotThrow(() => metrics.dispose());
  const count = logs.length;
  activeCallback({ firstDrawFrame: true, vsyncTimestamp: 1, layoutMeasureDuration: 0, inputHandlingDuration: 0 });
  assert.equal(logs.length, count);
  assert.ok(!JSON.stringify(logs).includes('PRIVATE_CONTENT'));
});

function frameWindowFixture(content, calls) {
  return {
    async setWindowLayoutFullScreen() {},
    getWindowAvoidArea() { return {}; },
    setUIContent() { calls.push('set-ui-content'); return content.promise; },
    on(event, callback) { calls.push({ event, callback }); },
    off(event, callback) { calls.push({ off: event, callback }); },
  };
}

test('EntryAbility registers numeric frames only after successful content and stops before base destruction', async () => {
  for (const debug of [false, true]) {
    const fixture = entryAbilityFixture(debug);
    const calls = fixture.lifecycleCalls;
    const content = deferred();
    const win = frameWindowFixture(content, calls);
    const loading = fixture.ability.loadWindowStageContent({ async getMainWindow() { return win; } });
    await until(() => calls.includes('set-ui-content'), 'content loading');
    assert.equal(calls.length, 1, 'no subscription before successful setUIContent');
    content.resolve();
    await loading;
    assert.equal(calls.length, debug ? 2 : 1);
    const registered = calls.find(call => call.event === 'frameMetricsMeasured');
    await fixture.ability.onDestroy();
    assert.equal(calls.at(-1), 'base-destroy');
    const removed = calls.find(call => call.off === 'frameMetricsMeasured');
    assert.equal(!!registered, debug);
    assert.equal(!!removed, debug);
    if (debug) {
      assert.equal(removed.callback, registered.callback);
      assert.ok(calls.indexOf(removed) < calls.indexOf('base-destroy'));
    }
  }
});

test('failed or late content loading cannot leave a frame observer on a destroyed Ability', async () => {
  const failed = entryAbilityFixture(true);
  const failure = deferred();
  const failedWindow = frameWindowFixture(failure, failed.lifecycleCalls);
  const loadingFailure = failed.ability.loadWindowStageContent({ async getMainWindow() { return failedWindow; } });
  await until(() => failed.lifecycleCalls.includes('set-ui-content'), 'failing content load');
  failure.reject(new Error('content failed'));
  await assert.rejects(loadingFailure, /content failed/);
  assert.equal(failed.lifecycleCalls.length, 1);

  const late = entryAbilityFixture(true);
  const completion = deferred();
  const lateWindow = frameWindowFixture(completion, late.lifecycleCalls);
  const loadingLate = late.ability.loadWindowStageContent({ async getMainWindow() { return lateWindow; } });
  await until(() => late.lifecycleCalls.includes('set-ui-content'), 'late content load');
  await late.ability.onDestroy();
  completion.resolve();
  await loadingLate;
  assert.deepEqual(late.lifecycleCalls, ['set-ui-content', 'base-destroy']);
});

test('cold-start debug VPN uses the Ability-owned host without bridge installation', async () => {
  const fixture = entryAbilityFixture(true);
  fixture.ability.onCreate({}, {});
  await fixture.ability.requestStartVpn('{}');
  assert.deepEqual(fixture.vpnCalls, ['{}']);
  assert.equal(fixture.ability.moduleName, 'nsh_platform_ohos');
  assert.ok(fixture.ability.bridgePlugins.some(plugin => plugin.factory().id === 'nsh.arkui'),
    'register the matching NSH SDK editor/view plugin alongside NativeAbility');
});

test('foreground and new Want wakeups reach the Ability-owned VPN service', async () => {
  const fixture = entryAbilityFixture(false);
  fixture.ability.onForeground();
  assert.deepEqual(fixture.foregroundCalls, []);
  fixture.ability.onCreate({}, {});
  const initial = fixture.ability.vpnPlugin;
  fixture.ability.onForeground();
  fixture.ability.onNewWant({}, {});
  await fixture.ability.lifecycleTail;
  assert.deepEqual(fixture.foregroundCalls, [initial, initial]);
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
  const api = automationFixture({ queryActiveVpnOptions() { return 'corrupt'; } });
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
    queryActiveVpnOptions() { calls.push('options'); return JSON.stringify({ mtu: 1500 }); },
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
  assert.deepEqual(calls, ['import', 'options', 'start', 'echo']);
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
    // Host effect port only: Core's real engine/renderer MTU contract is tested
    // in paws_core::engine::tests, not replaced by this boundary fixture.
    engineTunMtu(optionsJson) {
      const requestedMtu = JSON.parse(optionsJson).mtu;
      const platformMtu = requestedMtu;
      calls.push(['engine-mtu', optionsJson, platformMtu]);
      return platformMtu;
    },
    registerEngineProcessResolver() {},
    clearEngineProcessResolver() {},
    async prepareEngine(options, attempt) {
      if (engine === 'mihomo') return this.prepareMihomoVpn(options, attempt);
      await this.prepareVpn(attempt);
      return '{}';
    },
    async startEngine(fd, _netId, options, attempt) {
      if (engine === 'meow') return this.startVpn(fd, options, attempt);
      this.startExternalVpn(options, attempt);
    },
    async stopEngine(attempt) { return this.stopVpn(attempt); },
    async reloadEngine() { calls.push(['engine-reload']); },
    async pollEngineTraffic() { calls.push(['engine-traffic']); },
    async pollEngineObservations() { calls.push(['engine-observations']); },
    trimEngineMemory() { calls.push(['force-gc']); },
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
    globals: { setInterval: () => 1, clearInterval() {}, setTimeout: () => 2, clearTimeout() {}, Date: class extends Date { static now() { return clock ? clock() : Date.now(); } } },
  };
  const Extension = loadArkts('entry/src/main/ets/vpnability/PawsVpnExtensionAbility.ets', overrides).default;
  const extension = new Extension();
  extension.context = { filesDir: '/test/paws', applicationInfo: { name: 'com.example.paws.test' } };
  const config = configModule();
  const want = (attempt, fd) => config.buildVpnWant(config.DEFAULT_OPTIONS_JSON, { ashmemFd: fd, notificationFd: fd + 1 }, attempt);
  return {
    extension, connections, calls, want, native,
    emitPlatformEvent() { pendingEvents.resolve(true); },
    setLatestAttempt(attempt) { latestAttempt = attempt; },
    markTerminal(attempt) { terminalAttempts.add(attempt); },
    state() { return { activeAttempt, nativeRunning, protectedNetwork }; },
  };
}

for (const engine of ['meow', 'mihomo', 'sing-box']) {
  test(`${engine} host boundary forwards the native-computed MTU to actual OS create(config)`, async () => {
    const api = configModule();
    for (const mtu of [1280, 1500, 2816, 9000]) {
      const fixture = extensionFixture({ engine });
      const optionsJson = JSON.stringify({ ...JSON.parse(api.DEFAULT_OPTIONS_JSON), engine, mtu });
      const want = api.buildVpnWant(optionsJson, { ashmemFd: 21, notificationFd: 22 }, 'attempt-a');
      fixture.extension.onCreate(want);
      await until(() => fixture.calls.some(call => call[0] === 'create'), 'system VPN create with resolved MTU');
      const nativeMtuCalls = fixture.calls.filter(call => call[0] === 'engine-mtu');
      assert.equal(nativeMtuCalls.length, 1);
      assert.equal(nativeMtuCalls[0][1], optionsJson, 'native MTU query must receive the same complete startup options');
      const expectedMtu = mtu;
      assert.equal(nativeMtuCalls[0][2], expectedMtu);
      const createCalls = fixture.calls.filter(call => call[0] === 'create');
      assert.equal(createCalls.length, 1);
      assert.equal(createCalls[0][2].mtu, nativeMtuCalls[0][2], 'actual Extension must forward the native result to VpnConnection.create');
      assert.equal(createCalls[0][2].mtu, expectedMtu, `${engine} requested MTU ${mtu}`);
    }
  });
}

test('VPN extension telemetry never publishes a second system notification', () => {
  const fixture = extensionFixture();
  fixture.extension.activeAttemptId = 'telemetry-a';
  fixture.extension.persistTelemetry(fixture.extension.telemetryGeneration, fixture.extension.activeAttemptId);
  assert.equal(fixture.calls.filter(c => c[0] === 'speed-only').length, 0);
  assert.equal(fixture.calls.filter(c => c[0] === 'notification').length, 0);
});

test('telemetry delegates to the common engine and reports failures', async () => {
  const fixture = extensionFixture();
  fixture.extension.activeAttemptId = 'telemetry-a';
  fixture.extension.persistTelemetry(fixture.extension.telemetryGeneration, fixture.extension.activeAttemptId);
  await new Promise(resolve => setImmediate(resolve));
  fixture.extension.persistTelemetry(fixture.extension.telemetryGeneration, fixture.extension.activeAttemptId);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.calls.filter(c => c[0] === 'engine-traffic').length, 2);
  fixture.native.pollEngineTraffic = async () => { throw new Error('counter unavailable'); };
  fixture.extension.persistTelemetry(fixture.extension.telemetryGeneration, fixture.extension.activeAttemptId);
  await new Promise(resolve => setImmediate(resolve));
  assert.ok(fixture.calls.some(c => c[0] === 'warn' && String(c).includes('counter unavailable')));
});

test('pending background telemetry is coalesced and never invokes notifications', async () => {
  const fixture = extensionFixture();
  fixture.extension.activeAttemptId = 'telemetry-a';
  const sample = deferred();
  fixture.native.pollEngineTraffic = () => { fixture.calls.push(['engine-traffic']); return sample.promise; };
  for (let i = 0; i < 120; i++) fixture.extension.persistTelemetry(fixture.extension.telemetryGeneration, fixture.extension.activeAttemptId);
  assert.equal(fixture.calls.filter(c => c[0] === 'engine-traffic').length, 1);
  assert.equal(fixture.calls.filter(c => c[0] === 'notification').length, 0);
  sample.resolve();
  await new Promise(resolve => setImmediate(resolve));
  fixture.extension.persistTelemetry(fixture.extension.telemetryGeneration, fixture.extension.activeAttemptId);
  assert.equal(fixture.calls.filter(c => c[0] === 'engine-traffic').length, 2);
});


test('held observation work does not coalesce or delay the separate traffic lane', async () => {
  const fixture = extensionFixture();
  fixture.extension.activeAttemptId = 'telemetry-a';
  const observation = deferred();
  fixture.native.pollEngineObservations = () => { fixture.calls.push(['engine-observations']); return observation.promise; };
  for (let i=0;i<120;i++) fixture.extension.persistObservations(0,'telemetry-a');
  fixture.extension.persistTelemetry(0,'telemetry-a');
  await new Promise(resolve => setImmediate(resolve));
  fixture.extension.persistTelemetry(0,'telemetry-a');
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.calls.filter(c=>c[0]==='engine-traffic').length,2);
  assert.equal(fixture.calls.filter(c=>c[0]==='engine-observations').length,1);
  assert.equal(fixture.extension.observationsPending,true);
  observation.resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.extension.observationsPending,false);
});

test('held traffic and observation work each own one independent pending receipt', async () => {
  const fixture=extensionFixture(); fixture.extension.activeAttemptId='telemetry-a';
  const traffic=deferred();
  fixture.native.pollEngineTraffic=()=>{fixture.calls.push(['engine-traffic']);return traffic.promise;};
  for(let i=0;i<120;i++) fixture.extension.persistTelemetry(0,'telemetry-a');
  fixture.extension.persistObservations(0,'telemetry-a');
  await new Promise(resolve=>setImmediate(resolve));
  fixture.extension.persistObservations(0,'telemetry-a');
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(fixture.calls.filter(c=>c[0]==='engine-traffic').length,1);
  assert.equal(fixture.calls.filter(c=>c[0]==='engine-observations').length,2);
  traffic.resolve(); await new Promise(resolve=>setImmediate(resolve));
});

test('old attempt finally cannot clear the pending work of a restarted telemetry generation', async () => {
  const fixture=extensionFixture(); fixture.extension.activeAttemptId='old-attempt';
  const oldTraffic=deferred(),oldObservation=deferred();
  fixture.native.pollEngineTraffic=()=>{fixture.calls.push(['old-traffic']);return oldTraffic.promise;};
  fixture.native.pollEngineObservations=()=>{fixture.calls.push(['old-observations']);return oldObservation.promise;};
  fixture.extension.persistTelemetry(0,'old-attempt'); fixture.extension.persistObservations(0,'old-attempt');
  fixture.extension.stopTelemetry(); fixture.extension.activeAttemptId='new-attempt';
  const generation=fixture.extension.telemetryGeneration;
  const newTraffic=deferred(),newObservation=deferred();
  fixture.native.pollEngineTraffic=()=>{fixture.calls.push(['new-traffic']);return newTraffic.promise;};
  fixture.native.pollEngineObservations=()=>{fixture.calls.push(['new-observations']);return newObservation.promise;};
  fixture.extension.persistTelemetry(generation,'new-attempt');fixture.extension.persistObservations(generation,'new-attempt');
  oldTraffic.resolve();oldObservation.reject(new Error('stale observation error'));
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(fixture.extension.telemetryPending,true);assert.equal(fixture.extension.observationsPending,true);
  fixture.extension.persistTelemetry(0,'old-attempt');fixture.extension.persistObservations(0,'old-attempt');
  assert.equal(fixture.calls.filter(c=>c[0]==='new-traffic').length,1);assert.equal(fixture.calls.filter(c=>c[0]==='new-observations').length,1);
  assert.ok(!fixture.calls.some(c=>c[0]==='warn' && String(c).includes('stale observation error')));
  newTraffic.resolve();newObservation.resolve();await new Promise(resolve=>setImmediate(resolve));
  assert.equal(fixture.extension.telemetryPending,false);assert.equal(fixture.extension.observationsPending,false);
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

test('debug crash recorder keeps only error class and bounded source frames from object reasons', () => {
  const files = new Map(); const logs = []; let watcher;
  const io = {
    OpenMode: { WRITE_ONLY: 1, CREATE: 2, TRUNC: 4 },
    statSync() { throw Error('missing'); }, readTextSync() { throw Error('missing'); },
    openSync(path) { return { fd: path }; }, writeSync(fd, value) { files.set(fd, value); }, fsyncSync() {}, closeSync() {},
    renameSync(from, to) { files.set(to, files.get(from)); files.delete(from); },
  };
  const { SystemExitRecorder } = loadArkts('entry/src/main/ets/vpnability/SystemExitRecorder.ets', {
    globals: { setTimeout() { return 0; } }, '@kit.AbilityKit': {}, '@kit.CoreFileKit': { fileIo: io },
    '@kit.PerformanceAnalysisKit': { hilog: { info() {}, warn() {}, error(...args) { logs.push(args); } },
      hiAppEvent: { event: { APP_KILLED: 'APP_KILLED', APP_CRASH: 'APP_CRASH', APP_FREEZE: 'APP_FREEZE', RESOURCE_OVERLIMIT: 'RESOURCE_OVERLIMIT' },
        addWatcher(w) { watcher = w; return { setSize() {}, takeNext() { return null; } }; } },
    },
  });
  const reason = { name: 'TypeError', message: 'private-input-marker',
    stack: 'TypeError: private-input-marker\n' + Array.from({ length: 30 }, (_, i) => `    at change (entry/NativeEditors.ets:${300 + i}:12)`).join('\n'),
    arbitrary: 'private-payload-marker' };
  const recorder = new SystemExitRecorder(); recorder.start('/files', undefined, false, true);
  watcher.onReceive('OS', [{ appEventInfos: [{ name: 'APP_CRASH', params: { reason, time: 1, external_log: '/private/log' } }] }]);
  const record = JSON.parse(files.get('/files/paws/system-exits.json')).at(-1);
  assert.equal(record.errorName, 'TypeError'); assert.equal(record.stackFrames.length, 20);
  assert.equal(record.stackFrames[0], 'at change (entry/NativeEditors.ets:300:12)');
  assert.doesNotMatch(JSON.stringify({ record, logs }), /private-input-marker|private-payload-marker|external_log/);
  assert.equal(logs.filter(args => args[2] === 'debug fault frame %{public}s').length, 20);
  recorder.start('/files', undefined, false); // Normal packages do not collect fault objects.
  logs.length = 0;
  watcher.onReceive('OS', [{ appEventInfos: [{ name: 'APP_CRASH', params: { reason, time: 2 } }] }]);
  const releaseRecord = JSON.parse(files.get('/files/paws/system-exits.json')).at(-1);
  assert.equal(releaseRecord.errorName, undefined); assert.equal(releaseRecord.stackFrames, undefined); assert.equal(logs.length, 0);
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
    fixture.extension.reloadActiveEngine = async () => { throw new Error('must not reload after Stop'); };
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
  fixture.extension.reloadActiveEngine = async () => { throw new Error('configuration unavailable'); };
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
    queryActiveVpnOptions() { calls.push('options'); return JSON.stringify({}); },
  });
  const want = {parameters:{pawsMode:'global',pawsAutoStartVpn:true,pawsWaitForVpnBeforeProbes:true}};
  await api.runDebugAutomation(want,false,async()=>calls.push('start'));
  assert.deepEqual(calls,[]);
  await api.runDebugAutomation(want,true,async()=>calls.push('start'));
  assert.deepEqual(calls,['global','options','start']);
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
    queryActiveVpnOptions() { calls.push('options'); return JSON.stringify({}); },
  });
  const want = {parameters:{pawsProfileContent:'rules: [MATCH,DIRECT]', pawsMode:'global',
    pawsSelectGroup:'GLOBAL', pawsSelectProxy:'fixture-node', pawsAutoStartVpn:true,
    pawsWaitForVpnBeforeProbes:true}};
  const pending = api.runDebugAutomation(want,true,async()=>calls.push('start'));
  await until(()=>calls.includes('select'),'persistent selection');
  assert.deepEqual(calls,['import','mode:global','select']);
  selection.resolve();
  await pending;
  assert.deepEqual(calls,['import','mode:global','select','options','start']);
  calls.length = 0;
  await api.runDebugAutomation({parameters:{pawsSelectGroup:'GLOBAL',pawsSelectProxy:'fixture-node'}},
    true,async()=>calls.push('unexpected-start'));
  assert.deepEqual(calls,['select'],'selection-only Want must not be discarded or start VPN');
});

test('proxy selection Wants are inert in release and invalid or failed selections never start VPN', async () => {
  const calls = [];
  const api = automationFixture({
    async selectProxy() { calls.push('select'); throw new Error('fixture selection failed'); },
    queryActiveVpnOptions() { calls.push('options'); return JSON.stringify({}); },
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



function gatewayFixture({ dispatch, scan, selected = [], fileSize = 3, readError, packingError } = {}) {
  const calls = [];
  const events = [];
  const waits = [];
  const timers = [];
  let disposed = false;
  const modes = { COLOR_MODE_DARK: 0, COLOR_MODE_LIGHT: 1, COLOR_MODE_NOT_SET: -1 };
  const receiptOwner = pluginFixture().plugin;
  class Vpn {
    attachHost(context) { calls.push(['attach-host', context]); }
    detachHost() { calls.push(['detach-host']); }
    async requestStartVpn(options) { calls.push(['start', options]); }
    async requestStopVpn() { calls.push(['stop']); return true; }
    async requestRestartVpn(...args) { calls.push(['restart', ...args]); return false; }
    async requestStopVpnIfCurrent(...args) { calls.push(['stop-owned', ...args]); return false; }
    submitHostOperation(...args) { return receiptOwner.submitHostOperation(...args); }
    lookupHostOperation(...args) { return receiptOwner.lookupHostOperation(...args); }
    finishHostOperation(...args) { return receiptOwner.finishHostOperation(...args); }
  }
  let generation = 0;
  let domainVersions = [];
  let dirty = [];
  const native = {
    revision: 7,
    querySnapshot() { throw new Error('aggregate snapshots are not part of the UI protocol'); },
    subscribeState(text) {
      const request = JSON.parse(text);
      if (request.generation < generation) throw new Error('superseded observation');
      generation = request.generation;
      domainVersions = [...request.domains];
      for (const domain of ['config', 'status']) if (!domainVersions.some(key => key.domain === domain)) domainVersions.push({ domain });
      domainVersions = domainVersions.map(key => ({ ...key, revision: this.revision }));
      calls.push(['subscribe-state', request]);
      return JSON.stringify({ generation, revision: this.revision, changes: domainVersions });
    },
    observeState(critical) {
      calls.push(['observe-state', critical]);
      const change = { generation, revision: this.revision, changes: dirty };
      dirty = [];
      return JSON.stringify({ change, barrier: { generation, required: domainVersions.filter(key => !critical || ['config','status'].includes(key.domain)) } });
    },
    async queryState(text) {
      const request = JSON.parse(text);
      calls.push(['query-state', request]);
      if (request.generation !== generation || !domainVersions.some(key => key.domain === request.domain && key.groupKey === request.groupKey)) throw new Error('unsubscribed observation');
      return JSON.stringify({ ...request, revision: this.revision, configRevision: 4, data: { [request.domain]: [] } });
    },
    notify(domain, revision) {
      this.revision = revision;
      for (const key of domainVersions) if (key.domain === domain) { key.revision = revision; dirty.push({ ...key }); }
    },
    isRuntimeConfigRevisionCurrent() { return true; },
    queryUiPreferences() { return '{"language":"system","theme":"dark","advanced":false,"systemLocale":"en-US"}'; },
    async dispatchCommand(request) {
      const value = JSON.parse(request);
      calls.push(['dispatch', value]);
      return JSON.stringify(dispatch ? await dispatch(value) : { id: value.id, ok: true });
    },
    waitForUiRevision(revision) { const wait = deferred(); waits.push({ revision, wait }); return wait.promise; },
    cancelUiRevisionWait() { for (const item of waits) item.wait.resolve('7'); calls.push(['cancel-wait']); },
  };
  const api = loadArkts('entry/src/main/ets/components/PawsCoreGateway.ets', {
    '@kit.ArkUI': {},
    '@kit.ImageKit': { image: { createImagePacker() { return {
      async packing() { if (packingError) throw packingError; return Uint8Array.from([137,80,78,71]).buffer; },
      async release() { calls.push(['packer-release']); },
    }; } } },
    '@kit.AbilityKit': { ConfigurationConstant: { ColorMode: modes } },
    '@kit.CoreFileKit': {
      picker: { DocumentSelectOptions: class {}, DocumentViewPicker: class { async select() { return selected; } } },
      fileIo: {
        OpenMode: { READ_ONLY: 0 },
        async open() { calls.push(['file-open']); return { fd: 9 }; },
        async stat() { return { size: fileSize }; },
        async read(_fd, buffer) { if (readError) throw readError; new Uint8Array(buffer).set([97,98,99]); return buffer.byteLength; },
        async close() { calls.push(['file-close']); },
      },
    },
    '@kit.ArkTS': { util: {
      TextDecoder: { create() { return { decodeToString(bytes) { return new TextDecoder().decode(bytes); } }; } },
      Base64Helper: class { encodeToStringSync(bytes) { return Buffer.from(bytes).toString('base64'); } },
    } },
    '@kit.ScanKit': {
      generateBarcode: { ErrorCorrectionLevel: { LEVEL_M: 1 }, async createBarcode(text) {
        calls.push(['qr', text]); return { async release() { calls.push(['bitmap-release']); } };
      } },
      scanCore: { ScanType: { QR_CODE: 1 } }, scanBarcode: { startScanForResult: scan ?? (async () => ({ originalValue: '  https://example.test/profile  ' })) } },
    'libpaws_ui.so': { default: native, UiCommandSession: class {
      dispatch(request) { return native.dispatchCommand(request); }
      subscribeState(text) { return native.subscribeState(text); }
      observeState(critical) { return native.observeState(critical); }
      queryState(text) { return native.queryState(text); }
      close() { calls.push(['close-command-session']); }
    } },
    '@kit.PerformanceAnalysisKit': { hilog: { info() {}, warn() {} } },
    '../plugins/VpnPlugin': { VpnPlugin: Vpn },
    '../plugins/ExportPlugin': { ExportPlugin: class { async exportText(value) { calls.push(['export', value]); } } },
    '../common/Errors': loadArkts('entry/src/main/ets/common/Errors.ets', { globals: { Error } }),
    './PawsGatewayNode': { createGatewayNode() { return { isDisposed() { return disposed; }, dispose() { disposed = true; } }; } },
    globals: { Error, setTimeout(fn) { timers.push(fn); return timers.length; },
      clearTimeout(timer) { calls.push(['clear-timer', timer]); } },
  });
  const context = { setColorMode(mode) { calls.push(['color', mode]); }, async openLink(url) { calls.push(['url', url]); } };
  const host = new api.PawsHostServices(context);
  const surface = { isDisposed: false, setComponentSize(...args) { calls.push(['size', ...args]); }, emitComponentEvent(_id, name, value) { events.push([name, value]); } };
  const gateway = new api.PawsCoreGateway({}, 17, surface, host);
  return { api, host, gateway, native, calls, events, waits, timers, surface,
    dispatch: request => host.dispatch(request, gateway.session) };
}

test('bridge user errors preserve causes without attaching diagnostic stack fields', () => {
  const errors = loadArkts('entry/src/main/ets/common/Errors.ets', { globals: { Error } });
  const error = new Error('native controller request failed');
  error.name = 'NativeError';
  error.code = 401;
  error.stack = 'diagnostic-frame\n'.repeat(150);
  assert.equal(errors.describeUserError(error), 'code=401 NativeError: native controller request failed');
  assert.match(errors.describeError(error), /diagnostic-frame/,
    'the diagnostic formatter must retain its original stack behavior');
  assert.equal(errors.describeUserError({ name: 'PlatformError', code: 7, message: 'source unavailable', stack: error.stack }),
    'code=7 PlatformError: source unavailable');
  assert.equal(errors.describeUserError({ name: 'NativeError', reason: 'binding unavailable', stack: error.stack }),
    '{"name":"NativeError","reason":"binding unavailable"}');
  for (const value of ['original cause', 'full original message '.repeat(250), 0, false, null, undefined]) {
    assert.equal(errors.describeUserError(value), errors.describeError(value));
  }
  assert.equal(errors.describeUserError(new Error('full original message '.repeat(250))),
    'Error: ' + 'full original message '.repeat(250), 'message content is not truncated');
});

test('NSH query failures retain the real cause without a diagnostic stack in UiResult', async () => {
  const fixture = gatewayFixture();
  const error = new Error('native management configuration epoch was replaced');
  error.stack = 'query-only-diagnostic-frame\n'.repeat(150);
  fixture.native.queryState = async () => { throw error; };
  await fixture.gateway.runRequest({ id:435,command:'state.query',payload:{generation:1,domain:'proxies'} });
  const result = fixture.events.find(event => event[0] === 'result' && event[1].id === 435)[1];
  assert.equal(result.ok, false);
  assert.equal(result.error, 'Error: native management configuration epoch was replaced');
  assert.equal(result.error.includes('diagnostic-frame'), false);
  assert.equal(result.barrier, undefined);
  fixture.gateway.dispose();
});

test('NSH committed mutations preserve success and a stack-free state refresh warning', async () => {
  const fixture = gatewayFixture();
  const error = new Error('native observer unavailable');
  error.stack = 'post-commit-diagnostic-frame\n'.repeat(150);
  fixture.native.observeState = () => { throw error; };
  await fixture.gateway.runRequest({ id:5,command:'profiles.rename',payload:{} });
  const result = fixture.events.find(event => event[0] === 'result' && event[1].id === 5)[1];
  assert.equal(result.ok, true, 'a committed mutation is never presented as uncommitted');
  assert.equal(result.stateRefreshError, 'Operation completed, but state refresh failed: Error: native observer unavailable');
  assert.equal(result.error, undefined);
  assert.equal(result.stateRefreshError.includes('diagnostic-frame'), false);
  assert.equal(fixture.calls.filter(call => call[0] === 'dispatch').length, 1);
  fixture.gateway.dispose();
});

test('NSH gateway observes only metadata and preferences, then cancels subscriptions on disposal', async () => {
  const fixture = gatewayFixture();
  fixture.timers[0]();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(fixture.events.map(e => e[0]), ['preferences']);
  assert.equal(fixture.calls.filter(c => c[0] === 'query-state').length, 0);
  assert.equal(fixture.calls.filter(c => c[0] === 'color').length, 0, 'NSH preferences cannot trigger an Ability configuration rebuild');
  assert.equal(fixture.waits[0].revision, '7');
  fixture.gateway.dispose();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.events.length, 1);
  assert.equal(fixture.calls.filter(c => c[0] === 'cancel-wait').length, 1);
});

test('NSH configuration and idle wakeups never query business domains or re-emit preferences', async () => {
  const fixture = gatewayFixture();
  fixture.timers[0]();
  await new Promise(resolve => setImmediate(resolve));
  fixture.waits[0].wait.resolve('7');
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.calls.filter(call => call[0] === 'query-state').length, 0);
  assert.deepEqual(fixture.events.map(event => event[0]), ['preferences']);
  fixture.native.queryUiPreferences = () => '{"language":"system","theme":"light","advanced":false,"systemLocale":"en-US"}';
  fixture.waits[1].wait.resolve('7');
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(fixture.events.map(event => event[0]), ['preferences', 'preferences']);
  assert.deepEqual(fixture.calls.filter(call => call[0] === 'color'), []);
  fixture.gateway.dispose();
});

test('NSH registers subscription before explicit queries and changes contain only metadata', async () => {
  const fixture = gatewayFixture();
  await assert.rejects(fixture.dispatch({ id: 1, command: 'state.query', payload: {generation:1,domain:'rules'} }), /unsubscribed/);
  await fixture.gateway.runRequest({ id:2,command:'state.subscribe',payload:{generation:1,domains:[{domain:'rules'}]} });
  assert.deepEqual(fixture.events.map(e => e[0]), ['changed','result']);
  assert.equal(fixture.events[0][1].changes.some(key => key.domain === 'rules'),true);
  assert.equal(fixture.calls.filter(c => c[0] === 'query-state').length,1,'subscription cannot implicitly load business data');
  await fixture.gateway.runRequest({ id:3,command:'state.query',payload:{generation:1,domain:'rules'} });
  const result = fixture.events.at(-1)[1];
  assert.equal(result.data.domain,'rules');
  assert.equal(result.barrier,undefined,'queries cannot recursively wait on query barriers');
  assert.equal(JSON.stringify(fixture.events[0][1]).includes('data'),false);
  fixture.gateway.dispose();
});

test('NSH pending large domain query cannot delay Stop and dispose suppresses late delivery', async () => {
  const fixture = gatewayFixture();
  await fixture.gateway.runRequest({ id:1,command:'state.subscribe',payload:{generation:1,domains:[{domain:'rules'}]} });
  const pending = deferred();
  fixture.native.queryState = () => pending.promise;
  const query = fixture.gateway.runRequest({ id:2,command:'state.query',payload:{generation:1,domain:'rules'} });
  await fixture.gateway.runRequest({ id:3,command:'vpn.stop',payload:{} });
  const stop = fixture.events.find(e => e[0] === 'result' && e[1].id === 3)[1];
  assert.equal(stop.ok,true);
  assert.deepEqual(Array.from(stop.barrier.required, key => key.domain).sort(),['config','status']);
  const before = fixture.events.length;
  fixture.gateway.dispose();
  pending.resolve('{"generation":1,"domain":"rules","revision":7,"configRevision":4,"data":{"rules":[]}}');
  await query;
  assert.equal(fixture.events.length,before);
});

test('NSH failed change delivery is retried and committed success retains a state refresh warning', async () => {
  const fixture = gatewayFixture();
  const change = JSON.parse(fixture.native.subscribeState('{"generation":1,"domains":[]}'));
  const original = fixture.surface.emitComponentEvent;
  let fail = true;
  fixture.surface.emitComponentEvent = (...args) => { if (args[1] === 'changed' && fail) { fail=false;throw new Error('delivery failed'); } original(...args); };
  assert.throws(() => fixture.gateway.emitChanged(change),/delivery failed/);
  fixture.gateway.publishChanges();
  assert.equal(fixture.events[0][0],'changed');
  assert.equal(fixture.events[0][1].changes.length,2);
  fixture.native.observeState = () => { throw new Error('state unavailable'); };
  await fixture.gateway.runRequest({ id:4,command:'profiles.rename',payload:{} });
  const result = fixture.events.find(e => e[0] === 'result' && e[1].id === 4)[1];
  assert.equal(result.ok,true,'a post-commit query error cannot pretend the write failed');
  assert.match(result.stateRefreshError,/state unavailable/);
  fixture.gateway.dispose();
});

test('NSH app theme never changes global Ability resources and the latest system bar write wins', async () => {
  const fixture = gatewayFixture();
  const backgrounds = [], bars = [];
  const delayed = deferred();
  fixture.host.attachMainWindow({
    async setWindowBackgroundColor(color) { backgrounds.push(color); if (backgrounds.length === 1) await delayed.promise; },
    async setWindowSystemBarProperties(properties) { bars.push(properties); },
  });
  const dark = { theme: 'dark', systemColorMode: 1 };
  const light = { theme: 'light', systemColorMode: 0 };
  fixture.host.context.setColorMode = () => { throw new Error('global Ability appearance must remain untouched'); };
  fixture.host.applyPreferences(dark);
  await new Promise(resolve => setImmediate(resolve));
  fixture.host.applyPreferences(light);
  delayed.resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(fixture.calls.filter(call => call[0] === 'color'), []);
  assert.equal(bars.length, 1, 'stale dark update must not overwrite light');
  assert.equal(bars[0].statusBarColor, '#00000000');
  assert.equal(bars[0].navigationBarColor, '#00000000');
  assert.equal(bars[0].statusBarContentColor, '#ff000000');
  assert.equal(bars[0].navigationBarContentColor, '#ff000000');
  fixture.gateway.dispose();
  fixture.host.dispose();
});

test('NSH system bars follow system appearance only while the app uses system theme', async () => {
  const fixture = gatewayFixture();
  const bars = [];
  fixture.host.attachMainWindow({ async setWindowBackgroundColor() {}, async setWindowSystemBarProperties(properties) { bars.push(properties); } });
  fixture.host.applyPreferences({ theme: 'system', systemColorMode: 0 });
  await new Promise(resolve => setImmediate(resolve));
  fixture.host.applyPreferences({ theme: 'system', systemColorMode: 0 });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(bars.length, 1, 'unchanged system appearance must not repeat platform writes');
  fixture.host.applyPreferences({ theme: 'system', systemColorMode: 1 });
  await new Promise(resolve => setImmediate(resolve));
  fixture.host.applyPreferences({ theme: 'dark', systemColorMode: 1 });
  await new Promise(resolve => setImmediate(resolve));
  fixture.host.applyPreferences({ theme: 'dark', systemColorMode: 0 });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(bars.length, 3, 'explicit dark remains dark across actual system changes');
  fixture.host.applyPreferences({ theme: 'light', systemColorMode: 0 });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(fixture.calls.filter(call => call[0] === 'color'), []);
  assert.deepEqual(bars.map(value => value.statusBarContentColor), ['#ffffffff', '#ff000000', '#ffffffff', '#ff000000']);
  assert.deepEqual(bars.map(value => value.navigationBarContentColor), ['#ffffffff', '#ff000000', '#ffffffff', '#ff000000']);
  fixture.host.dispose();
  fixture.gateway.dispose();
});

test('NSH immersive page lets one SafeAreaView consume native visual insets', () => {
  const source = readFileSync(resolve(root, 'entry/src/main/ets/pages/Index.ets'), 'utf8');
  assert.match(source, /\.expandSafeArea\(\[SafeAreaType.SYSTEM\], \[SafeAreaEdge.TOP, SafeAreaEdge.BOTTOM\]\)/);
  assert.match(source, /\.backgroundColor\(Color.Transparent\)/);
  const rootModifiers = source.slice(source.lastIndexOf("    .height('100%')"));
  assert.doesNotMatch(rootModifiers, /\.padding\(/, 'the host must not add another fixed or native-inset padding');
});

test('NSH gateway Stop reaches the owner while an earlier Start is awaiting completion', async () => {
  const start = deferred();
  const fixture = gatewayFixture({ dispatch: async value => {
    if (value.command === 'vpn.start') await start.promise;
    return { id: value.id, ok: true, effect: { kind: value.command === 'vpn.start' ? 'start' : 'stop', owner: '', configRevision: '4', optionsJson: '{}' } };
  } });
  fixture.gateway.updateProps([{ name: 'request', value: { id: 1, command: 'vpn.start', payload: {} } }]);
  fixture.gateway.updateProps([{ name: 'request', value: { id: 2, command: 'vpn.stop', payload: {} } }]);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.calls.filter(c => c[0] === 'stop').length, 1, 'Stop cannot wait behind a pending Start');
  assert.equal(fixture.calls.filter(c => c[0] === 'start').length, 0);
  start.resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(fixture.events.filter(e => e[0] === 'result').map(e => e[1].id), [2, 1]);
  assert.equal(fixture.calls.filter(c => c[0] === 'start').length, 0, 'a late prepared Start cannot resurrect a stopped VPN');
  assert.equal(fixture.events.find(e => e[0] === 'result' && e[1].id === 1)[1].ok, false);
  fixture.gateway.dispose();
});

test('NSH configuration effects preserve session and revision fences and reject duplicate prop delivery', async () => {
  const fixture = gatewayFixture({ dispatch: async value => ({ id: value.id, ok: true,
    effect: { kind: 'restart', owner: 'old-owner', configRevision: '18446744073709551610', optionsJson: '{"mtu":1400}' } }) });
  const change = { name: 'request', value: { id: 4, command: 'vpnOptions.set', payload: { mtu: 1400 } } };
  fixture.gateway.updateProps([change]);
  fixture.gateway.updateProps([change]);
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(fixture.calls.find(c => c[0] === 'restart'), ['restart', 'old-owner', '18446744073709551610', '{"mtu":1400}']);
  assert.equal(fixture.calls.filter(c => c[0] === 'dispatch').length, 1);
  const result = fixture.events.find(e => e[0] === 'result' && e[1].id === 4)[1];
  assert.equal(result.ok, false);
  assert.match(result.error, /saved.*session changed/);
  fixture.gateway.dispose();
});

test('NSH system capabilities return picker and scan content without importing it', async () => {
  const fixture = gatewayFixture({ selected: ['file:///profiles/My%20Profile.yaml'] });
  const file = await fixture.dispatch({ id: 1, command: 'platform.importFile', payload: {} });
  assert.equal(file.data.name, 'My Profile.yaml');
  assert.equal(file.data.content, 'abc');
  assert.equal(fixture.calls.filter(c => c[0] === 'file-close').length, 1);
  const scan = await fixture.dispatch({ id: 2, command: 'platform.scan', payload: {} });
  assert.equal(scan.data, 'https://example.test/profile');
  assert.equal(fixture.calls.filter(c => c[0] === 'dispatch').length, 0);
  await assert.rejects(fixture.dispatch({ id: 3, command: 'platform.openUrl', payload: { url: 'file:///private' } }), /absolute HTTP/);
  fixture.gateway.dispose();
});

test('NSH scan cancellation uses the SDK code and never message substrings', async () => {
  let failure;
  const fixture = gatewayFixture({ scan: async () => {
    if (failure) throw failure;
    return { originalValue: ' https://example.test/subscription ' };
  } });
  try {
    const request = { id: 1, command: 'platform.scan', payload: {} };
    assert.equal((await fixture.dispatch(request)).data, 'https://example.test/subscription');
    failure = { code: 1000500002, message: 'localized user dismissal' };
    assert.equal((await fixture.dispatch({ ...request, id: 2 })).data, '');
    failure = { code: 1000500001, message: 'cancel token service failed' };
    await assert.rejects(fixture.dispatch({ ...request, id: 3 }), /cancel token service failed/);
    assert.equal(fixture.calls.filter(call => call[0] === 'dispatch').length, 0);
  } finally {
    fixture.gateway.dispose();
    fixture.host.dispose();
  }
});

test('NSH file import bounds memory and closes the descriptor after errors', async () => {
  for (const options of [{ fileSize: 16 * 1024 * 1024 + 1 }, { readError: new Error('read failed') }]) {
    const fixture = gatewayFixture({ ...options, selected: ['file:///profile.yaml'] });
    await assert.rejects(fixture.dispatch({ id: 1, command: 'platform.importFile', payload: {} }));
    assert.equal(fixture.calls.filter(c => c[0] === 'file-close').length, 1);
    fixture.gateway.dispose();
  }
});

test('NSH QR generation returns a real image payload and releases native resources on both paths', async () => {
  const fixture = gatewayFixture();
  const response = await fixture.dispatch({ id: 9, command: 'platform.qr', payload: { text: 'https://example.test/subscription' } });
  assert.equal(response.data.pngBase64, 'iVBORw==');
  assert.equal(response.data.source, 'data:image/png;base64,iVBORw==');
  assert.equal(fixture.calls.filter(c => c[0] === 'bitmap-release').length, 1);
  assert.equal(fixture.calls.filter(c => c[0] === 'packer-release').length, 1);
  fixture.gateway.dispose();
  const failing = gatewayFixture({ packingError: new Error('image unavailable') });
  await assert.rejects(failing.dispatch({ id: 10, command: 'platform.qr', payload: { text: 'text' } }), /image unavailable/);
  assert.equal(failing.calls.filter(c => c[0] === 'bitmap-release').length, 1);
  assert.equal(failing.calls.filter(c => c[0] === 'packer-release').length, 1);
  failing.gateway.dispose();
});

test('NSH file picker cancellation is an empty result and never opens a descriptor', async () => {
  const fixture = gatewayFixture();
  const response = await fixture.dispatch({ id: 11, command: 'platform.importFile', payload: {} });
  assert.equal(response.ok, true);
  assert.equal(response.data, null);
  assert.equal(fixture.calls.some(c => c[0] === 'file-open'), false);
  fixture.gateway.dispose();
});

test('NSH prepared Start rejects a configuration revision committed before its receipt arrives', async () => {
  const prepared = deferred();
  const fixture = gatewayFixture({ dispatch: async request => {
    await prepared.promise;
    return { id: request.id, ok: true, effect: { kind: 'start', owner: '', configRevision: '4', optionsJson: '{"mtu":1400}' } };
  } });
  let currentRevision = '4';
  fixture.native.isRuntimeConfigRevisionCurrent = revision => revision === currentRevision;
  const start = fixture.dispatch({ id: 12, command: 'vpn.start', payload: {} });
  currentRevision = '5';
  prepared.resolve();
  const result = await start;
  assert.equal(result.ok, false);
  assert.match(result.error, /Configuration changed while preparing VPN/);
  assert.equal(fixture.calls.some(c => c[0] === 'start'), false);
  fixture.gateway.dispose();
});

test('NSH receive ACK precedes asynchronous completion and duplicate props never resubmit', async () => {
  const pending = deferred();
  const fixture = gatewayFixture({ dispatch: async request => { await pending.promise; return { id: request.id, ok: true }; } });
  const change = { name: 'request', value: { id: 31, command: 'profiles.list', payload: {} } };
  fixture.gateway.updateProps([change]);
  fixture.gateway.updateProps([change]);
  assert.equal(fixture.events.length, 0, 'ACK waits until native mount listeners can be installed');
  fixture.timers[2]();
  fixture.timers[3]();
  assert.deepEqual(fixture.events.map(event => [event[0], event[1].id]), [['received', 31], ['received', 31]]);
  assert.equal(fixture.calls.filter(call => call[0] === 'dispatch').length, 1);
  pending.resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.events.at(-1)[0], 'result');
  fixture.gateway.dispose();
});

test('NSH prequeued ACK and late result have no effect after idempotent Gateway disposal', async () => {
  const pending = deferred();
  const fixture = gatewayFixture({ dispatch: async request => { await pending.promise; return { id: request.id, ok: true }; } });
  const change = { name: 'request', value: { id: 31, command: 'profiles.list', payload: {} } };
  fixture.gateway.updateProps([change]);
  fixture.gateway.updateProps([change]);
  fixture.gateway.dispose(); fixture.gateway.dispose();
  assert.equal(fixture.calls.filter(call => call[0] === 'close-command-session').length, 1);
  assert.deepEqual(fixture.calls.filter(call => call[0] === 'clear-timer'), [['clear-timer', 3], ['clear-timer', 4]]);
  fixture.timers[2](); fixture.timers[3](); // An already delivered timer may still run after cancellation.
  const timers = fixture.timers.length;
  fixture.gateway.updateProps([{ name: 'request', value: { id: 32, command: 'vpn.stop', payload: {} } }]);
  assert.equal(fixture.timers.length, timers);
  pending.resolve(); await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.events.length, 0, 'No fake ACK or late result can revive a retired owner');
  assert.equal(fixture.calls.filter(call => call[0] === 'dispatch').length, 1);
  assert.equal(fixture.calls.some(call => call[0] === 'stop'), false, 'disposing the bridge is not VPN Stop');
});

test('NSH source-surface or native-node disposal gates a queued ACK before Gateway receives its own disposal', async () => {
  for (const source of ['surface', 'node']) {
    const pending = deferred();
    const fixture = gatewayFixture({ dispatch: async request => { await pending.promise; return { id: request.id, ok: true }; } });
    fixture.gateway.updateProps([{ name: 'request', value: { id: 31, command: 'profiles.list', payload: {} } }]);
    if (source === 'surface') fixture.surface.isDisposed = true;
    else fixture.gateway.node.isDisposed = () => true;
    assert.equal(fixture.gateway.disposed, false);
    fixture.timers[2]();
    assert.equal(fixture.events.length, 0);
    fixture.gateway.dispose(); pending.resolve(); await new Promise(resolve => setImmediate(resolve));
    assert.equal(fixture.events.length, 0);
  }
});

test('NSH exact retired native registry error closes only this Gateway while surface flags are still live', async () => {
  const pending = deferred();
  const fixture = gatewayFixture({ dispatch: async request => { await pending.promise; return { id: request.id, ok: true }; } });
  const contexts = new Map([[17, { active: true }]]);
  let lookups = 0;
  // Effect port for Components.context(handle): registration retirement
  // removes only the handle; NshSurface.isDisposed and ComponentContent remain live.
  fixture.surface.emitComponentEvent = (handle) => {
    lookups++;
    if (!contexts.get(handle)?.active) {
      const error = new Error('native component is disposed'); error.code = 'GenericFailure'; throw error;
    }
    assert.fail('This fixture must never deliver a fake receipt');
  };
  const change = { name: 'request', value: { id: 31, command: 'profiles.list', payload: {} } };
  fixture.gateway.updateProps([change]); fixture.gateway.updateProps([change]);
  contexts.delete(17);
  assert.equal(fixture.surface.isDisposed, false);
  assert.equal(fixture.gateway.node.isDisposed(), false);
  assert.doesNotThrow(() => fixture.timers[2]());
  assert.equal(fixture.gateway.disposed, true);
  assert.equal(fixture.calls.filter(call => call[0] === 'close-command-session').length, 1);
  assert.ok(fixture.calls.some(call => call[0] === 'clear-timer' && call[1] === 4));
  fixture.timers[3]();
  pending.resolve(); await new Promise(resolve => setImmediate(resolve));
  assert.equal(lookups, 1, 'the remaining ACK/result cannot touch the retired registry again');
  assert.equal(fixture.events.length, 0);
  assert.equal(fixture.calls.some(call => call[0] === 'stop'), false);
  fixture.gateway.dispose();
  assert.equal(fixture.calls.filter(call => call[0] === 'close-command-session').length, 1);
});

test('NSH active emit failures and near-match disposal messages preserve exact thrown identity', () => {
  for (const message of ['delivery failed', 'prefix native component is disposed',
    'native component is disposed suffix', 'native component is disposed\nstack',
    'surface is disposed']) {
    const fixture = gatewayFixture();
    const error = new Error(message); error.code = 'GenericFailure';
    fixture.surface.emitComponentEvent = () => { throw error; };
    assert.throws(() => fixture.gateway.emit('received', { id: 1 }), actual => actual === error);
    assert.equal(fixture.gateway.disposed, false);
    assert.equal(fixture.calls.filter(call => call[0] === 'close-command-session').length, 0);
    fixture.gateway.dispose();
  }
});

test('NSH confirms the original retained VPN operation without submitting it again', async () => {
  const started = deferred();
  const fixture = gatewayFixture({ dispatch: async request => ({ id: request.id, ok: true,
    effect: { kind: 'start', owner: '', configRevision: '4', optionsJson: '{}' } }) });
  fixture.host.vpn.requestStartVpn = async options => { fixture.calls.push(['start', options]); await started.promise; };
  const start = fixture.dispatch({ id: 32, command: 'vpn.start', payload: {} });
  await new Promise(resolve => setImmediate(resolve));
  const pending = await fixture.dispatch({ id: 33, command: 'vpn.confirm', payload: { requestId: 32 } });
  assert.equal(pending.data.status, 'pending');
  assert.equal(pending.data.requestId, 32);
  assert.equal(fixture.calls.filter(call => call[0] === 'start').length, 1);
  started.resolve();
  assert.equal((await start).ok, true);
  const terminal = await fixture.dispatch({ id: 34, command: 'vpn.confirm', payload: { requestId: 32 } });
  assert.equal(terminal.data.status, 'succeeded');
  assert.equal(terminal.data.result, true);
  assert.equal(fixture.calls.filter(call => call[0] === 'dispatch').length, 1);
  const otherSession = new fixture.api.PawsCommandSession();
  const unavailable = await fixture.host.dispatch({ id: 35, command: 'vpn.confirm', payload: { requestId: 32 } }, otherSession);
  assert.equal(unavailable.data.status, 'unavailable', 'a new gateway cannot claim another session receipt');
  otherSession.close();
  fixture.gateway.dispose();
});

test('NSH recovery submits a new Stop intent and fences late prepared Start', async () => {
  const prepared = deferred();
  const fixture = gatewayFixture({ dispatch: async request => {
    if (request.command === 'vpn.start') await prepared.promise;
    return { id: request.id, ok: true, effect: { kind: request.command === 'vpn.start' ? 'start' : 'stop', owner: '', configRevision: '4', optionsJson: '{}' } };
  } });
  const start = fixture.dispatch({ id: 36, command: 'vpn.start', payload: {} });
  const recover = await fixture.dispatch({ id: 37, command: 'vpn.recover', payload: { requestId: 36 } });
  assert.equal(recover.ok, true);
  assert.equal(recover.data.requestId, 36);
  assert.equal(recover.data.status, 'succeeded');
  assert.equal(fixture.calls.filter(call => call[0] === 'stop').length, 1);
  prepared.resolve();
  assert.equal((await start).ok, false);
  assert.equal(fixture.calls.filter(call => call[0] === 'start').length, 0);
  const terminal = await fixture.dispatch({ id: 38, command: 'vpn.confirm', payload: { requestId: 36 } });
  assert.equal(terminal.data.status, 'failed');
  assert.match(terminal.data.error, /superseded/);
  fixture.gateway.dispose();
});

test('NSH saved configuration followup has a confirmable receipt without repeating the save', async () => {
  const restarted = deferred();
  const fixture = gatewayFixture({ dispatch: async request => ({ id: request.id, ok: true,
    effect: { kind: 'restart', owner: '', configRevision: '4', optionsJson: '{}' } }) });
  fixture.host.vpn.requestStartVpn = async options => { fixture.calls.push(['start', options]); await restarted.promise; };
  const save = fixture.dispatch({ id: 40, command: 'vpnOptions.set', payload: { mtu: 1400 } });
  await new Promise(resolve => setImmediate(resolve));
  const operation = fixture.events.find(event => event[0] === 'operation');
  assert.deepEqual(JSON.parse(JSON.stringify(operation[1])), { requestId: 40, action: 'restart' });
  const pending = await fixture.dispatch({ id: 41, command: 'vpn.confirm', payload: { requestId: 40 } });
  assert.equal(pending.data.status, 'pending');
  assert.equal(fixture.calls.filter(call => call[0] === 'dispatch').length, 1, 'confirm does not save the configuration again');
  restarted.resolve();
  assert.equal((await save).ok, true);
  const terminal = await fixture.dispatch({ id: 42, command: 'vpn.confirm', payload: { requestId: 40 } });
  assert.equal(terminal.data.status, 'succeeded');
  assert.equal(fixture.calls.filter(call => call[0] === 'start').length, 1);
  fixture.gateway.dispose();
});

test('NSH recovery cannot claim success when its actual Stop was superseded', async () => {
  const fixture = gatewayFixture({ dispatch: async request => ({ id: request.id, ok: true,
    effect: { kind: 'stop', owner: '', configRevision: '4', optionsJson: '{}' } }) });
  fixture.host.vpn.requestStopVpn = async () => false;
  const recovered = await fixture.dispatch({ id: 43, command: 'vpn.recover', payload: { requestId: 32 } });
  assert.equal(recovered.ok, false);
  assert.equal(recovered.data.status, 'failed');
  assert.match(recovered.error, /Stop was superseded/);
  fixture.gateway.dispose();
});

test('NSH gateway close cancels preparation ownership but completes an acquired configuration effect', async () => {
  const committed = deferred();
  const fixture = gatewayFixture({ dispatch: async request => {
    await committed.promise;
    return { id: request.id, ok: true, effect: { kind: 'restart', owner: '', configRevision: '4', optionsJson: '{}' } };
  } });
  fixture.gateway.updateProps([{ name: 'request', value: { id: 39, command: 'vpnOptions.set', payload: {} } }]);
  fixture.gateway.dispose();
  committed.resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(fixture.calls.filter(call => call[0] === 'close-command-session').length, 1);
  assert.equal(fixture.calls.filter(call => call[0] === 'start').length, 1, 'the committed response effect survives gateway disposal');
  assert.equal(fixture.events.filter(event => event[0] !== 'operation').length, 0, 'the disposed view receives no late state or receipts');
});

test('NSH command receipt follows changed metadata and carries a state barrier', async () => {
  const fixture = gatewayFixture();
  fixture.native.subscribeState('{"generation":1,"domains":[{"domain":"profiles"}]}');
  fixture.native.notify('profiles',8);
  fixture.gateway.updateProps([{ name: 'request', value: { id: 13, command: 'profiles.rename', payload: { name: 'new' } } }]);
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(fixture.events.map(event => event[0]), ['preferences', 'changed', 'result']);
  assert.equal(fixture.events[2][1].id, 13);
  assert.equal(fixture.events[2][1].barrier.required.find(key => key.domain === 'profiles').revision,8);
  fixture.gateway.dispose();
});

test('NSH bootstrap failures are visible instead of becoming successful initialization', async () => {
  const fixture = gatewayFixture({ dispatch: async request => ({ id: request.id, ok: false, error: 'profile preparation failed' }) });
  fixture.timers[1]();
  await new Promise(resolve => setImmediate(resolve));
  const failure = fixture.events.find(event => event[0] === 'result')[1];
  assert.equal(failure.id, 0);
  assert.equal(failure.ok, false);
  assert.equal(failure.error, 'profile preparation failed');
  fixture.gateway.dispose();
});
