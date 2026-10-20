import { SystemExitRecorder } from './SystemExitRecorder';
import { updateTinyPawsForms } from '../widget/TinyPawsFormUpdate';
import { LifecycleJournal } from './LifecycleJournal';
import { AbilityConstant, Want, bundleManager } from '@kit.AbilityKit';
import { connection, vpnExtension, VpnExtensionAbility } from '@kit.NetworkKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import pawsUi from 'libpaws_ui.so';
import * as mihomoCore from 'libcore.so';
import {
  PawsVpnConfig,
  PawsPlatformSharedMemoryFds,
  parseOptions,
  readPlatformSharedMemoryFds,
  readPlatformHandoffToken,
  readPlatformStartAttemptId,
  readOptionsJson,
  setPawsBundleName,
} from './VpnConfig';

import { getPhysicalNetwork } from './UnderlyingNetwork';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsVpn';
const PLATFORM_OPERATION_TIMEOUT_MS: number = 30000;

interface PawsPlatformVpnHandoffPayload {
  attemptId: string;
  optionsJson: string;
}

interface MihomoVpnPlan {
  initParams: string;
  setupParams: string;
}

interface SingBoxVpnPlan {
  configJson: string;
  workDir: string;
  proxyCount: number;
  groupCount: number;
  ruleCount: number;
  skippedRuleCount: number;
}

interface MihomoTraffic {
  up: number;
  down: number;
}

interface NativePrepareResult {
  error: string;
  planJson: string;
}

function withTimeout<T>(operation: Promise<T>, timeoutMs: number, label: string): Promise<T> {
  return new Promise<T>((resolve: (value: T) => void, reject: (reason?: Object | string) => void): void => {
    let settled = false;
    const timer = setTimeout((): void => {
      if (!settled) {
        settled = true;
        reject(new Error(`${label} timed out after ${timeoutMs}ms`));
      }
    }, timeoutMs);
    operation.then((value: T): void => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        resolve(value);
      }
    }).catch((err: Error): void => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        reject(err);
      }
    });
  });
}

interface PawsErrorLike {
  code?: number;
  message?: string;
  name?: string;
  stack?: string;
}

function describeError(err: Error | Object | string | number | boolean | null | undefined): string {
  if (err instanceof Error) {
    return `${err.name}: ${err.message}${err.stack ? ` ${err.stack}` : ''}`;
  }
  if (typeof err === 'string') {
    return err;
  }
  if (typeof err === 'number' || typeof err === 'boolean') {
    return err.toString();
  }
  if (!err) {
    return '';
  }
  const like = err as PawsErrorLike;
  if (typeof like.message === 'string' && like.message.length > 0) {
    const code = typeof like.code === 'number' ? `code=${like.code} ` : '';
    const name = typeof like.name === 'string' && like.name.length > 0 ? `${like.name}: ` : '';
    return `${code}${name}${like.message}${like.stack ? ` ${like.stack}` : ''}`;
  }
  const json = JSON.stringify(err);
  return json && json !== '{}' ? json : err.toString();
}

export default class PawsVpnExtensionAbility extends VpnExtensionAbility {
  private vpnConnection?: vpnExtension.VpnConnection;
  private tunFd: number = -1;
  private starting: boolean = false;
  private telemetryTimer: number = -1;
  private platformSubscriptionActive: boolean = false;
  private platformSubscriptionGeneration: number = 0;
  private requestGeneration: number = 0;
  private inFlightAttemptId: string = '';
  private pendingHandoffAttemptId: string = '';
  private activeAttemptId: string = '';
  private journal: LifecycleJournal = new LifecycleJournal();
  private systemExitRecorder: SystemExitRecorder = new SystemExitRecorder();
  private heartbeatTimer: number = -1;
  private heartbeatTicks: number = 0;
  private requestChain: Promise<void> = Promise.resolve();
  private pendingPlatformOperation?: Promise<void>;
  private pendingPlatformOperationConnection?: vpnExtension.VpnConnection;
  private pendingNativePrepare?: Promise<void>;
  private pendingNativePrepareAttemptId: string = '';
  private pendingCleanup?: Promise<string>;
  private pendingCleanupConnection?: vpnExtension.VpnConnection;
  private activeEngine: string = '';
  private activeOptionsJson: string = '';
  private activeUnderlyingNetId: number = 0;
  private lastMihomoSetupParams: string = '';
  private lastSingBoxConfigJson: string = '';
  private reloadChain: Promise<void> = Promise.resolve();
  private uidBundleNameMap: Map<number, string> = new Map<number, string>();
  private defaultNetworkConnection?: connection.NetConnection;
  private networkRefreshTimer: number = -1;
  private networkMonitorGeneration: number = 0;
  private networkMonitorStartedAt: number = 0;
  private knownNetworkIds: Set<number> = new Set<number>();

  public onCreate(want: Want): void {
    super.onCreate(want);
    this.configureNativeHome();
    this.journal.open(this.context.filesDir);
    try { this.systemExitRecorder.start(this.context.filesDir, undefined, true); }
    catch (_) { hilog.warn(LOG_DOMAIN, LOG_TAG, 'VPN system exit recorder unavailable'); }
    hilog.info(LOG_DOMAIN, LOG_TAG, 'onCreate');
    this.startFromWant(want);
  }

  public onRequest(want: Want, _startId: number): void {
    this.startFromWant(want);
  }

  public onDestroy(): void {
    this.journal.record('extension-destroy-requested', this.tunFd >= 0);
    this.requestGeneration += 1;
    this.pendingHandoffAttemptId = '';
    this.stopHeartbeat();
    this.stopPlatformSubscription();
    this.stopDefaultNetworkMonitor();
    this.destroyVpn().catch((err: Error): void => {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'onDestroy cleanup failed %{public}s', describeError(err));
    });
  }

  public onMemoryLevel(level: AbilityConstant.MemoryLevel): void {
    this.trimEngineMemory(`memory-level-${String(level)}`);
  }

  private configureNativeHome(): boolean {
    try {
      setPawsBundleName(this.context.applicationInfo.name);
      pawsUi.configureAppHome(`${this.context.filesDir}/paws`);
      return true;
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'configure native home failed %{public}s', describeError(err));
      return false;
    }
  }

  private startFromWant(want: Want): void {
    this.enqueueStartFromWant(want);
  }

  private enqueueStartFromWant(want: Want): void {
    if (!this.configureNativeHome()) {
      return;
    }
    const sharedMemory = readPlatformSharedMemoryFds(want);
    if (!sharedMemory) {
      try {
        this.receivePlatformHandoff(
          readPlatformStartAttemptId(want),
          readPlatformHandoffToken(want),
        );
      } catch (err) {
        hilog.error(LOG_DOMAIN, LOG_TAG,
          'descriptor-free VPN bootstrap rejected %{public}s', describeError(err));
      }
      return;
    }
    let attemptId: string;
    let optionsJson: string;
    try {
      attemptId = readPlatformStartAttemptId(want);
      optionsJson = readOptionsJson(want);
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'VPN request rejected %{public}s', describeError(err));
      return;
    }
    this.queueRequest(attemptId, optionsJson, sharedMemory);
  }

  private receivePlatformHandoff(attemptId: string, token: string): void {
    if (attemptId === this.pendingHandoffAttemptId || attemptId === this.inFlightAttemptId ||
      attemptId === this.activeAttemptId) {
      return;
    }
    this.pendingHandoffAttemptId = attemptId;
    hilog.info(LOG_DOMAIN, LOG_TAG,
      'waiting for VPN session FD handoff attempt %{public}s', attemptId);
    pawsUi.receiveAndAttachPlatformVpnHandoff(token).then(
      (payloadJson: string): void => {
        const payload = JSON.parse(payloadJson) as PawsPlatformVpnHandoffPayload;
        if (payload.attemptId !== attemptId || typeof payload.optionsJson !== 'string') {
          throw new Error('VPN session FD handoff payload does not match bootstrap attempt');
        }
        hilog.info(LOG_DOMAIN, LOG_TAG,
          'received and attached VPN session FD handoff attempt %{public}s', attemptId);
        this.queueRequest(attemptId, payload.optionsJson);
      },
    ).catch((err: Error): void => {
      const message = describeError(err) || 'VPN session FD handoff failed';
      hilog.error(LOG_DOMAIN, LOG_TAG, 'VPN session FD handoff failed %{public}s', message);
      try {
        pawsUi.failUnattachedPlatformVpnStart(attemptId, message);
      } catch (_innerErr) {
        // The attempt may already have been superseded by the UI.
      }
    }).finally((): void => {
      if (this.pendingHandoffAttemptId === attemptId) {
        this.pendingHandoffAttemptId = '';
      }
    });
  }

  private queueRequest(
    attemptId: string,
    optionsJson: string,
    sharedMemory?: PawsPlatformSharedMemoryFds,
  ): void {
    if (sharedMemory && !this.isLatestWant(
      sharedMemory.ashmemFd, sharedMemory.notificationFd, attemptId)) {
      return;
    }
    if (attemptId === this.inFlightAttemptId || attemptId === this.activeAttemptId) {
      return;
    }
    const generation = ++this.requestGeneration;
    this.inFlightAttemptId = attemptId;
    this.requestChain = this.requestChain.then(async (): Promise<void> => {
      await this.handleRequest(
        attemptId,
        optionsJson,
        generation,
        sharedMemory,
      );
    }).catch(async (err: Error): Promise<void> => {
      if (generation !== this.requestGeneration || this.inFlightAttemptId !== attemptId) {
        return;
      }
      const message = describeError(err) || 'VPN startup pipeline failed';
      hilog.error(LOG_DOMAIN, LOG_TAG, 'VPN startup pipeline failed %{public}s', message);
      if (this.activeAttemptId === attemptId) {
        this.recordVpnFailureForAttempt(attemptId, message);
        await this.destroyVpn();
      } else {
        // Validation/attach/bind can fail before this process adopts the
        // owner. Settle only the matching still-unattached UI transaction;
        // the exact core callback rejects a terminal or replaced attempt.
        try {
          pawsUi.failUnattachedPlatformVpnStart(attemptId, message);
        } catch (innerErr) {
          hilog.error(LOG_DOMAIN, LOG_TAG, 'publish unattached VPN failure failed %{public}s',
            describeError(innerErr));
        }
      }
    }).finally((): void => {
      if (generation === this.requestGeneration && this.inFlightAttemptId === attemptId) {
        this.inFlightAttemptId = '';
      }
    });
  }

  private async handleRequest(
    attemptId: string,
    optionsJson: string,
    generation: number,
    sharedMemory?: PawsPlatformSharedMemoryFds,
  ): Promise<void> {
    if (generation !== this.requestGeneration) {
      return;
    }
    await this.waitForPendingCleanup();
    if (generation !== this.requestGeneration ||
      (sharedMemory !== undefined && !this.isLatestWant(
        sharedMemory.ashmemFd, sharedMemory.notificationFd, attemptId))) {
      return;
    }
    if (this.activeAttemptId.length > 0 && this.activeAttemptId !== attemptId) {
      await this.destroyVpn();
      // destroyVpn retains a timed-out or failed platform teardown as a
      // barrier. Do not replace IPC ownership until that exact cleanup has
      // completed successfully.
      await this.waitForPendingCleanup();
      if (generation !== this.requestGeneration) {
        return;
      }
    }

    this.stopPlatformSubscription();
    if (sharedMemory) {
      pawsUi.attachPlatformSharedMemory(sharedMemory.ashmemFd, sharedMemory.notificationFd);
    }
    const ownerIdentity = pawsUi.bindPlatformVpnStart(attemptId);
    hilog.info(
      LOG_DOMAIN,
      LOG_TAG,
      'bound VPN owner attempt %{public}s process %{public}s',
      attemptId,
      ownerIdentity,
    );
    this.startPlatformSubscription();
    this.activeAttemptId = attemptId;

    // Stop publishes its terminal lane state before racing the journal
    // Pending-to-Stopping CAS. If this Extension won Pending-to-Attached,
    // bind re-synchronizes that lane; consume the terminal state before
    // creating native or VpnConnection resources and acknowledge cleanup.
    const boundLifecycle = pawsUi.extensionTick(attemptId);
    if (boundLifecycle === 'failed' || boundLifecycle === 'disconnected' ||
      boundLifecycle === 'stopping') {
      await this.destroyVpn();
      return;
    }

    if (this.tunFd >= 0 && this.vpnConnection) {
      if (boundLifecycle === 'connected') {
        this.startTelemetry();
        this.startHeartbeat(generation, attemptId);
        return;
      }
      await this.destroyVpn();
      throw new Error('existing native VPN owner is no longer connected');
    }

    this.starting = true;
    try {
      await this.performStart(optionsJson, attemptId, generation);
    } finally {
      if (this.isCurrentRequest(generation, attemptId)) {
        this.starting = false;
      }
    }
  }

  private isLatestWant(ashmemFd: number, notificationFd: number, attemptId: string): boolean {
    try {
      pawsUi.validatePlatformVpnStartRequest(ashmemFd, notificationFd, attemptId);
      return true;
    } catch (err) {
      try {
        if (pawsUi.acknowledgeTerminalPlatformVpnStartDelivery(
          ashmemFd,
          notificationFd,
          attemptId,
        )) {
          hilog.info(
            LOG_DOMAIN,
            LOG_TAG,
            'acknowledged delivery of terminal VPN request %{public}s',
            attemptId,
          );
        }
      } catch (ackErr) {
        hilog.warn(
          LOG_DOMAIN,
          LOG_TAG,
          'terminal VPN delivery acknowledgement failed %{public}s',
          describeError(ackErr),
        );
      }
      hilog.warn(LOG_DOMAIN, LOG_TAG, 'ignored stale VPN request %{public}s %{public}s',
        attemptId, describeError(err));
      return false;
    }
  }

  private isCurrentRequest(generation: number, attemptId: string): boolean {
    return generation === this.requestGeneration && attemptId === this.activeAttemptId;
  }

  private requireCurrentRequest(
    generation: number,
    attemptId: string,
    connection?: vpnExtension.VpnConnection,
  ): void {
    if (!this.isCurrentRequest(generation, attemptId) ||
      (connection !== undefined && this.vpnConnection !== connection)) {
      throw new Error('stale VPN request generation');
    }
  }

  private async awaitPlatformOperation<T>(
    connection: vpnExtension.VpnConnection,
    operation: Promise<T>,
    label: string,
  ): Promise<T> {
    let settled = false;
    const barrier: Promise<void> = operation.then((_value: T): void => {
      settled = true;
    }).catch((_err: Error): void => {
      settled = true;
    });
    this.pendingPlatformOperation = barrier;
    this.pendingPlatformOperationConnection = connection;
    try {
      return await withTimeout(operation, PLATFORM_OPERATION_TIMEOUT_MS, label);
    } finally {
      if (settled && this.pendingPlatformOperation === barrier) {
        this.pendingPlatformOperation = undefined;
        this.pendingPlatformOperationConnection = undefined;
      }
    }
  }

  private async performStart(
    optionsJson: string,
    attemptId: string,
    generation: number,
  ): Promise<void> {
    this.requireCurrentRequest(generation, attemptId);
    await this.waitForPendingCleanup();
    this.requireCurrentRequest(generation, attemptId);
    if (!pawsUi.setPlatformVpnStarting(attemptId, true)) {
      throw new Error('platform VPN start attempt is no longer active');
    }

    const options = parseOptions(optionsJson);
    const engine = pawsUi.resolveVpnEngine(optionsJson);
    if (engine !== 'meow' && engine !== 'mihomo' && engine !== 'sing-box') {
      throw new Error(`unsupported resolved VPN engine: ${engine}`);
    }
    // The embedded mihomo wrapper uses a 9000-byte userspace TUN, matching
    // FlClash's HarmonyOS service. Keep the Rust engine's configured MTU for
    // meow-rs, but give mihomo the interface shape it was built and tested
    // against.
    const config = new PawsVpnConfig(options, engine === 'meow' ? undefined : 9000);
    let underlyingNetId = 0;
    if (engine === 'sing-box' || engine === 'meow' || engine === 'mihomo') {
      const underlyingNetwork: connection.NetHandle = await getPhysicalNetwork();
      underlyingNetId = underlyingNetwork.netId;
      pawsUi.setPhysicalNetwork(underlyingNetId);
      if (underlyingNetId <= 0) {
        throw new Error('could not resolve the underlying physical network');
      }
    }
    const vpnOwner = vpnExtension.createVpnConnection(this.context);
    this.vpnConnection = vpnOwner;
    const startedAt = Date.now();

    let createOperation: Promise<number>;
    try {
      createOperation = vpnOwner.create(config);
    } catch (err) {
      throw new Error(`VpnConnection.create rejected synchronously: ${describeError(err)}`);
    }
    const fd = await this.awaitPlatformOperation(vpnOwner, createOperation, 'VpnConnection.create');
    if (!this.isCurrentRequest(generation, attemptId) || this.vpnConnection !== vpnOwner) {
      await this.destroyVpn();
      throw new Error('stale VPN request generation after TUN create');
    }
    this.tunFd = fd;
    hilog.info(LOG_DOMAIN, LOG_TAG, 'created tun fd %{public}d', fd);
    hilog.info(LOG_DOMAIN, LOG_TAG, 'VPN phase create finished in %{public}d ms', Date.now() - startedAt);

    const protectStartedAt = Date.now();
    try {
      await this.awaitPlatformOperation(vpnOwner, vpnOwner.protectProcessNet(),
        'VpnConnection.protectProcessNet');
      this.requireCurrentRequest(generation, attemptId, vpnOwner);
      if (!pawsUi.setPlatformNetworkProtected(attemptId, true)) {
        throw new Error('platform VPN protection result belongs to a stale attempt');
      }
      hilog.info(LOG_DOMAIN, LOG_TAG, 'protected process network');
      hilog.info(LOG_DOMAIN, LOG_TAG, 'VPN phase protect finished in %{public}d ms', Date.now() - protectStartedAt);
    } catch (err) {
      const message = describeError(err) || 'protect process network failed';
      pawsUi.setPlatformNetworkProtected(attemptId, false, message);
      throw new Error(message);
    }

    // Config/provider IO must use the protected physical network.
    const prepareStartedAt = Date.now();
    const nativePrepare: Promise<NativePrepareResult> = (engine === 'mihomo'
      ? Promise.resolve().then((): NativePrepareResult => ({
          error: '',
          planJson: pawsUi.prepareMihomoVpn(optionsJson, attemptId),
        }))
      : engine === 'sing-box'
        ? Promise.resolve().then((): NativePrepareResult => ({
            error: '',
            planJson: pawsUi.prepareSingBoxVpn(optionsJson, attemptId),
          }))
      : pawsUi.prepareVpn(attemptId).then((_loaded: boolean): NativePrepareResult => ({
          error: '',
          planJson: '',
        })))
      .catch((err: Error): NativePrepareResult => ({
        error: describeError(err) || 'prepare native VPN failed',
        planJson: '',
      }));
    const nativePrepareBarrier: Promise<void> = nativePrepare.then((_result: NativePrepareResult): void => {});
    this.pendingNativePrepare = nativePrepareBarrier;
    this.pendingNativePrepareAttemptId = attemptId;
    nativePrepareBarrier.finally((): void => {
      if (this.pendingNativePrepare === nativePrepareBarrier) {
        this.pendingNativePrepare = undefined;
        this.pendingNativePrepareAttemptId = '';
      }
    });

    const prepareResult = await this.awaitPlatformOperation(vpnOwner, nativePrepare, 'native VPN prepare');
    if (prepareResult.error.length > 0) {
      throw new Error(prepareResult.error);
    }
    this.requireCurrentRequest(generation, attemptId, vpnOwner);
    hilog.info(LOG_DOMAIN, LOG_TAG, 'VPN phase config finished in %{public}d ms', Date.now() - prepareStartedAt);
    this.activeEngine = engine;
    this.activeOptionsJson = optionsJson;
    this.activeUnderlyingNetId = underlyingNetId;
    if (engine === 'mihomo') {
      await this.awaitPlatformOperation(
        vpnOwner,
        this.startMihomo(fd, optionsJson, attemptId, prepareResult.planJson),
        'mihomo VPN start',
      );
    } else if (engine === 'sing-box') {
      await this.awaitPlatformOperation(
        vpnOwner,
        this.startSingBox(fd, underlyingNetId, optionsJson, attemptId, prepareResult.planJson),
        'sing-box VPN start',
      );
    } else {
      await this.awaitPlatformOperation(
        vpnOwner,
        pawsUi.startVpn(fd, optionsJson, attemptId),
        'native VPN start',
      );
    }
    this.requireCurrentRequest(generation, attemptId, vpnOwner);
    const lifecycle = pawsUi.extensionTick(attemptId);
    if (lifecycle !== 'connected') {
      throw new Error(`native VPN worker entered ${lifecycle}`);
    }
    await pawsUi.persistEngineSession(attemptId, engine, underlyingNetId);
    this.journal.record('vpn-connected', true);
    updateTinyPawsForms(this.context.filesDir, 'VPN 已建立 · 联网状态请在应用检测');
    this.startTelemetry();
    this.startHeartbeat(generation, attemptId);
    try {
      this.startDefaultNetworkMonitor(generation, attemptId);
    } catch (err) {
      // Network-change reconciliation improves handovers but is not allowed to
      // invalidate an already established system VPN on older SDK builds.
      hilog.warn(LOG_DOMAIN, LOG_TAG, 'default network monitor start skipped %{public}s', describeError(err));
    }
    hilog.info(LOG_DOMAIN, LOG_TAG, 'VPN started in %{public}d ms', Date.now() - startedAt);
  }

  private async startMihomo(
    fd: number,
    optionsJson: string,
    attemptId: string,
    planJson: string,
  ): Promise<void> {
    const plan = JSON.parse(planJson) as MihomoVpnPlan;
    if (!plan.initParams || !plan.setupParams) {
      throw new Error('mihomo startup plan is incomplete');
    }
    const setupError = await new Promise<string>((resolve: (value: string) => void): void => {
      mihomoCore.quickSetup(resolve, plan.initParams, plan.setupParams);
    });
    if (setupError.length > 0) {
      throw new Error(`mihomo setup failed: ${setupError}`);
    }
    this.lastMihomoSetupParams = plan.setupParams;
    const options = parseOptions(optionsJson);
    const addresses = options.addresses?.length ? options.addresses : ['172.19.0.1/30'];
    const dnsAddresses = options.dnsHijacking === false
      ? (options.dnsAddresses?.length ? options.dnsAddresses : ['172.19.0.2'])
      : ['0.0.0.0'];
    const owner = this.vpnConnection;
    if (!owner) {
      throw new Error('VpnConnection is unavailable for mihomo');
    }
    // Paws' Rust stack names are not mihomo stack names. "mixed" is the
    // FlClash HarmonyOS default and combines the system TCP stack with the
    // userspace UDP stack. Passing the Rust-only "netstack-smoltcp" value (or
    // forcing gVisor) can make this wrapper reject an otherwise valid TUN.
    const configuredStack = options.stack?.toLowerCase() ?? '';
    const mihomoStack = configuredStack === 'gvisor' || configuredStack === 'system'
      ? configuredStack
      : 'mixed';
    const started: boolean = mihomoCore.startTUN(
      (socketFd: number): Promise<void> => owner.protect(socketFd),
      (protocol: number, source: string, target: string): number =>
        this.resolveUid(protocol, source, target),
      (uid: number): string => this.resolvePackage(uid),
      fd,
      mihomoStack,
      addresses.join(','),
      dnsAddresses.join(','),
    );
    if (!started) {
      throw new Error('mihomo rejected the HarmonyOS TUN descriptor');
    }
    try {
      pawsUi.startExternalVpn(optionsJson, attemptId);
    } catch (err) {
      mihomoCore.stopTun();
      throw new Error(describeError(err) || 'mihomo external VPN registration failed');
    }
    this.trimEngineMemory('mihomo-start');
  }

  private async startSingBox(
    fd: number,
    underlyingNetId: number,
    optionsJson: string,
    attemptId: string,
    planJson: string,
  ): Promise<void> {
    const plan = JSON.parse(planJson) as SingBoxVpnPlan;
    if (!plan.configJson || !plan.workDir) {
      throw new Error('sing-box startup plan is incomplete');
    }
    pawsUi.startSingBox(fd, underlyingNetId, plan.configJson, plan.workDir);
    this.lastSingBoxConfigJson = plan.configJson;
    try {
      pawsUi.startExternalVpn(optionsJson, attemptId);
      hilog.info(
        LOG_DOMAIN,
        LOG_TAG,
        'sing-box started %{public}d proxies %{public}d groups %{public}d rules %{public}d skipped',
        plan.proxyCount,
        plan.groupCount,
        plan.ruleCount,
        plan.skippedRuleCount,
      );
    } catch (err) {
      pawsUi.stopSingBox();
      throw new Error(describeError(err) || 'sing-box external VPN registration failed');
    }
  }

  private toNetAddress(addr: string): connection.NetAddress {
    const separator = addr.lastIndexOf(':');
    const host = addr.slice(0, separator);
    const port = Number(addr.slice(separator + 1));
    const address = host.startsWith('[') && host.endsWith(']') ? host.slice(1, -1) : host;
    const family = address.includes(':') ? 2 : 1;
    return { address, family, port };
  }

  private resolveUid(protocol: number, source: string, target: string): number {
    try {
      return connection.getConnectOwnerUidSync(
        protocol,
        this.toNetAddress(source),
        this.toNetAddress(target),
      );
    } catch (_) {
      return -1;
    }
  }

  private resolvePackage(uid: number): string {
    if (!this.uidBundleNameMap.has(uid)) {
      try {
        this.uidBundleNameMap.set(uid, bundleManager.getBundleNameByUidSync(uid));
      } catch (_) {
        this.uidBundleNameMap.set(uid, '');
      }
    }
    return this.uidBundleNameMap.get(uid) ?? '';
  }

  private recordVpnFailureForAttempt(attemptId: string, message: string): void {
    this.journal.record('vpn-failure', this.tunFd >= 0);
    try {
      pawsUi.setPlatformVpnFailed(attemptId, message);
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'record VPN failure failed %{public}s', describeError(err));
    }
  }

  private beginVpnCleanup(
    connection: vpnExtension.VpnConnection | undefined,
    attemptId: string,
  ): Promise<string> {
    if (this.pendingCleanup) {
      if (connection === undefined || connection === this.pendingCleanupConnection) {
        return this.pendingCleanup;
      }
      return Promise.resolve('a previous VpnConnection cleanup is still pending');
    }
    const pendingOperation = connection !== undefined &&
      this.pendingPlatformOperationConnection === connection ? this.pendingPlatformOperation : undefined;
    if (pendingOperation) {
      this.pendingPlatformOperation = undefined;
      this.pendingPlatformOperationConnection = undefined;
    }
    const pendingNativePrepare = attemptId.length > 0 &&
      this.pendingNativePrepareAttemptId === attemptId ? this.pendingNativePrepare : undefined;
    if (pendingNativePrepare) {
      this.pendingNativePrepare = undefined;
      this.pendingNativePrepareAttemptId = '';
    }
    const cleanup: Promise<string> = (async (): Promise<string> => {
      const errors: string[] = [];
      if (attemptId.length > 0) {
        try {
          await pawsUi.stopVpn(attemptId);
        } catch (err) {
          errors.push(`native VPN stop: ${describeError(err)}`);
        }
      }
      if (pendingOperation) {
        // Promise timeouts do not cancel create/protect/start. Preserve this
        // exact operation as a barrier before destroying its connection.
        await pendingOperation;
      }
      if (pendingNativePrepare) {
        // Native prepare is not cancelled by an ArkTS timeout. Keep it inside
        // the same cleanup barrier as create/protect before releasing owner.
        await pendingNativePrepare;
      }
      if (connection) {
        try {
          await connection.destroy();
        } catch (err) {
          errors.push(`VpnConnection.destroy: ${describeError(err)}`);
        }
      }
      if (errors.length === 0 && attemptId.length > 0) {
        try {
          if (!pawsUi.completePlatformVpnCleanup(attemptId)) {
            errors.push('platform VPN cleanup acknowledgement rejected');
          } else {
            hilog.info(
              LOG_DOMAIN,
              LOG_TAG,
              'VPN cleanup acknowledged after native join and connection destroy attempt %{public}s',
              attemptId,
            );
          }
        } catch (err) {
          errors.push(`platform VPN cleanup acknowledgement: ${describeError(err)}`);
        }
      }
      return errors.join('; ');
    })();
    this.pendingCleanup = cleanup;
    this.pendingCleanupConnection = connection;
    return cleanup;
  }

  private async waitForPendingCleanup(): Promise<void> {
    const cleanup = this.pendingCleanup;
    if (!cleanup) {
      return;
    }
    let error = '';
    try {
      error = await withTimeout(cleanup, PLATFORM_OPERATION_TIMEOUT_MS, 'pending VPN cleanup');
    } catch (err) {
      hilog.warn(LOG_DOMAIN, LOG_TAG, 'pending VPN cleanup timed out, releasing barrier %{public}s', describeError(err));
      if (this.pendingCleanup === cleanup) {
        this.pendingCleanup = undefined;
        this.pendingCleanupConnection = undefined;
      }
      return;
    }
    if (this.pendingCleanup === cleanup) {
      this.pendingCleanup = undefined;
      this.pendingCleanupConnection = undefined;
    }
    if (error.length > 0) {
      throw new Error(`previous VPN cleanup failed: ${error}`);
    }
  }

  private async destroyVpn(): Promise<void> {
    this.stopHeartbeat();
    this.stopPlatformSubscription();
    this.stopDefaultNetworkMonitor();
    this.stopTelemetry();
    const connection = this.vpnConnection;
    const attemptId = this.activeAttemptId;
    const engine = this.activeEngine;
    const nativeStopStartedAt = Date.now();
    if (this.activeEngine === 'mihomo') {
      try {
        mihomoCore.stopTun();
      } catch (err) {
        hilog.warn(LOG_DOMAIN, LOG_TAG, 'mihomo stop failed %{public}s', describeError(err));
      }
    } else if (this.activeEngine === 'sing-box') {
      try {
        pawsUi.stopSingBox();
      } catch (err) {
        hilog.warn(LOG_DOMAIN, LOG_TAG, 'sing-box stop failed %{public}s', describeError(err));
      }
    }
    hilog.info(LOG_DOMAIN, LOG_TAG, 'VPN phase native stop finished in %{public}d ms engine=%{public}s',
      Date.now() - nativeStopStartedAt, engine);
    this.activeEngine = '';
    this.activeOptionsJson = '';
    this.activeUnderlyingNetId = 0;
    this.lastMihomoSetupParams = '';
    this.lastSingBoxConfigJson = '';
    this.vpnConnection = undefined;
    this.tunFd = -1;
    this.starting = false;
    this.activeAttemptId = '';
    const cleanup = this.beginVpnCleanup(connection, attemptId);
    try {
      const error = await withTimeout(cleanup, PLATFORM_OPERATION_TIMEOUT_MS, 'VPN cleanup');
      if (error.length > 0) {
        throw new Error(error);
      }
      this.journal.record('vpn-cleanup-complete', false);
      updateTinyPawsForms(this.context.filesDir, 'VPN 已停止');
      if (this.pendingCleanup === cleanup) {
        this.pendingCleanup = undefined;
        this.pendingCleanupConnection = undefined;
      }
    } catch (err) {
      // Keep pendingCleanup installed. A later request awaits the same Promise
      // and cannot create a classic VPN that a late destroy could tear down.
      hilog.error(LOG_DOMAIN, LOG_TAG, 'VPN cleanup pending %{public}s', describeError(err));
    }
  }

  private startPlatformSubscription(): void {
    if (this.platformSubscriptionActive) {
      return;
    }
    this.platformSubscriptionActive = true;
    const generation = ++this.platformSubscriptionGeneration;
    this.runPlatformSubscription(generation);
  }

  private stopPlatformSubscription(): void {
    this.platformSubscriptionActive = false;
    this.platformSubscriptionGeneration += 1;
    // Wake the waiter parked in waitForPlatformChangeEvent so the loop can
    // observe the flag and unwind without waiting for the next peer frame.
    try {
      pawsUi.cancelPlatformChangeWait();
    } catch (err) {
      hilog.warn(LOG_DOMAIN, LOG_TAG, 'cancel platform change wait failed %{public}s', describeError(err));
    }
  }

  private async runPlatformSubscription(generation: number): Promise<void> {
    while (this.platformSubscriptionActive && generation === this.platformSubscriptionGeneration) {
      try {
        // Fully event driven: the native wait parks on the shared-memory
        // notification socket until the UI publishes a control frame (or the
        // wait is cancelled by stopPlatformSubscription). No polling.
        const changed = await pawsUi.waitForPlatformChangeEvent();
        if (this.platformSubscriptionActive && generation === this.platformSubscriptionGeneration) {
          pawsUi.syncPlatformChanges();
          // A Stop frame has priority over configuration/provider I/O. Go
          // engines previously prepared/reloaded first, delaying cooperative
          // teardown or throwing before the terminal state could be observed.
          this.reconcileExtensionTerminal();
          if (!this.platformSubscriptionActive || generation !== this.platformSubscriptionGeneration) {
            return;
          }
          try {
            if (this.activeEngine === 'mihomo' && this.activeAttemptId.length > 0) {
              await this.reloadMihomoConfig(false);
            } else if (this.activeEngine === 'sing-box' && this.activeAttemptId.length > 0) {
              await this.reloadSingBoxConfig(false);
            }
          } catch (err) {
            // A config error must not disconnect the control subscription;
            // later Stop frames still need to reach this exact owner.
            hilog.warn(LOG_DOMAIN, LOG_TAG, 'VPN config synchronization failed %{public}s', describeError(err));
          }
          this.reconcileExtensionTerminal();
          if (!changed) {
            await new Promise<void>((resolve: () => void): void => {
              setTimeout(resolve, 250);
            });
          }
        }
      } catch (err) {
        hilog.error(LOG_DOMAIN, LOG_TAG, 'platform subscription failed %{public}s', describeError(err));
        if (generation === this.platformSubscriptionGeneration) {
          this.platformSubscriptionActive = false;
        }
      }
    }
  }

  private async reloadMihomoConfig(force: boolean = false): Promise<void> {
    const run = async (): Promise<void> => {
      if (this.activeEngine !== 'mihomo' || this.activeAttemptId.length === 0) {
        return;
      }
      const planJson = pawsUi.prepareMihomoVpn(this.activeOptionsJson, this.activeAttemptId);
      const plan = JSON.parse(planJson) as MihomoVpnPlan;
      if (!force && plan.setupParams === this.lastMihomoSetupParams) {
        return;
      }
      const error = await new Promise<string>((resolve: (value: string) => void): void => {
        mihomoCore.quickSetup(resolve, plan.initParams, plan.setupParams);
      });
      if (error.length > 0) {
        throw new Error(`mihomo config reload failed: ${error}`);
      }
      this.lastMihomoSetupParams = plan.setupParams;
      this.trimEngineMemory('mihomo-reload');
      hilog.info(LOG_DOMAIN, LOG_TAG, 'mihomo config synchronized from platform control');
    };
    const next = this.reloadChain.then(run, run);
    this.reloadChain = next.catch((): void => {});
    await next;
  }

  private async reloadSingBoxConfig(force: boolean = false): Promise<void> {
    const run = async (): Promise<void> => {
      if (this.activeEngine !== 'sing-box' || this.activeAttemptId.length === 0) {
        return;
      }
      if (this.tunFd < 0) {
        throw new Error('sing-box reload has no TUN descriptor');
      }
      if (this.activeUnderlyingNetId <= 0) {
        throw new Error('sing-box reload has no underlying network handle');
      }
      const planJson = pawsUi.prepareSingBoxVpn(this.activeOptionsJson, this.activeAttemptId);
      const plan = JSON.parse(planJson) as SingBoxVpnPlan;
      if (!force && plan.configJson === this.lastSingBoxConfigJson) {
        return;
      }
      pawsUi.stopSingBox();
      pawsUi.startSingBox(this.tunFd, this.activeUnderlyingNetId, plan.configJson, plan.workDir);
      this.lastSingBoxConfigJson = plan.configJson;
      hilog.info(LOG_DOMAIN, LOG_TAG, 'sing-box config synchronized from platform control');
    };
    const next = this.reloadChain.then(run, run);
    this.reloadChain = next.catch((): void => {});
    await next;
  }

  private startHeartbeat(generation: number, attemptId: string): void {
    this.stopHeartbeat();
    this.reconcileExtensionTerminal();
    if (!this.isCurrentRequest(generation, attemptId) || this.tunFd < 0) {
      return;
    }
    this.heartbeatTicks = 0;
    this.heartbeatTimer = setInterval((): void => {
      if (!this.isCurrentRequest(generation, attemptId)) {
        this.stopHeartbeat();
        return;
      }
      this.reconcileExtensionTerminal();
      this.heartbeatTicks += 1;
      if (this.heartbeatTicks % 20 === 0) {
        this.trimEngineMemory('heartbeat');
      }
    }, 3000);
  }

  private stopHeartbeat(): void {
    if (this.heartbeatTimer >= 0) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = -1;
    }
  }

  /**
   * Observe the underlying default network and rebuild engine-side dialers
   * after Wi-Fi/mobile transitions. The initial callbacks emitted while the
   * VPN interface is being created are ignored; later changes are debounced so
   * a single handover cannot trigger multiple core reloads.
   */
  private startDefaultNetworkMonitor(generation: number, attemptId: string): void {
    this.stopDefaultNetworkMonitor();
    const monitor = connection.createNetConnection({
      netCapabilities: { bearerTypes: [], networkCap: [connection.NetCap.NET_CAPABILITY_INTERNET, connection.NetCap.NET_CAPABILITY_NOT_VPN] },
    });
    const monitorGeneration = ++this.networkMonitorGeneration;
    this.defaultNetworkConnection = monitor;
    this.networkMonitorStartedAt = Date.now();
    this.knownNetworkIds.clear();
    monitor.on('netAvailable', (handle: connection.NetHandle): void => {
      if (handle.netId !== undefined) {
        this.knownNetworkIds.add(handle.netId);
      }
      this.scheduleDefaultNetworkRefresh(generation, attemptId, monitorGeneration, 'available');
    });
    monitor.on('netLost', (handle: connection.NetHandle): void => {
      if (handle.netId !== undefined) {
        this.knownNetworkIds.delete(handle.netId);
      }
      this.scheduleDefaultNetworkRefresh(generation, attemptId, monitorGeneration, 'lost');
    });
    monitor.register((err?: Error): void => {
      if (err) {
        hilog.warn(LOG_DOMAIN, LOG_TAG, 'default network monitor unavailable %{public}s', describeError(err));
        if (this.defaultNetworkConnection === monitor) {
          this.defaultNetworkConnection = undefined;
        }
      } else {
        hilog.info(LOG_DOMAIN, LOG_TAG, 'default network monitor registered');
      }
    });
  }

  private scheduleDefaultNetworkRefresh(
    generation: number,
    attemptId: string,
    monitorGeneration: number,
    reason: string,
  ): void {
    if (monitorGeneration !== this.networkMonitorGeneration ||
      !this.isCurrentRequest(generation, attemptId) ||
      Date.now() - this.networkMonitorStartedAt < 3000) {
      return;
    }
    if (this.networkRefreshTimer >= 0) {
      clearTimeout(this.networkRefreshTimer);
    }
    this.networkRefreshTimer = setTimeout((): void => {
      this.networkRefreshTimer = -1;
      this.refreshAfterDefaultNetworkChange(generation, attemptId, monitorGeneration, reason)
        .catch((err: Error): void => {
          hilog.warn(LOG_DOMAIN, LOG_TAG, 'default network refresh failed %{public}s', describeError(err));
        });
    }, 1500);
  }

  private async refreshAfterDefaultNetworkChange(
    generation: number,
    attemptId: string,
    monitorGeneration: number,
    reason: string,
  ): Promise<void> {
    if (monitorGeneration !== this.networkMonitorGeneration ||
      !this.isCurrentRequest(generation, attemptId)) {
      return;
    }
    const physicalNetwork = await getPhysicalNetwork();
    if (physicalNetwork.netId <= 0) {
      return;
    }
    this.activeUnderlyingNetId = physicalNetwork.netId;
    pawsUi.setPhysicalNetwork(physicalNetwork.netId);
    if (this.activeEngine === 'mihomo') {
      await this.reloadMihomoConfig(true);
    } else if (this.activeEngine === 'sing-box') {
      await this.reloadSingBoxConfig(true);
    } else {
      pawsUi.closeAllConnections();
    }
    this.journal.record(`network-${reason}-reconciled`, true);
    updateTinyPawsForms(this.context.filesDir, '网络已切换 · VPN 连接已刷新');
    hilog.info(
      LOG_DOMAIN,
      LOG_TAG,
      'default network %{public}s reconciled engine=%{public}s networks=%{public}d',
      reason,
      this.activeEngine,
      this.knownNetworkIds.size,
    );
  }

  private stopDefaultNetworkMonitor(): void {
    this.networkMonitorGeneration += 1;
    if (this.networkRefreshTimer >= 0) {
      clearTimeout(this.networkRefreshTimer);
      this.networkRefreshTimer = -1;
    }
    const monitor = this.defaultNetworkConnection;
    this.defaultNetworkConnection = undefined;
    this.knownNetworkIds.clear();
    if (monitor) {
      monitor.unregister((err?: Error): void => {
        if (err) {
          hilog.info(LOG_DOMAIN, LOG_TAG, 'default network monitor unregister skipped %{public}s', describeError(err));
        }
      });
    }
  }

  private reconcileExtensionTerminal(): void {
    if (this.tunFd < 0 || this.activeAttemptId.length === 0) {
      return;
    }
    const generation = this.requestGeneration;
    const attemptId = this.activeAttemptId;
    let lifecycle: string;
    try {
      lifecycle = pawsUi.extensionTick(attemptId);
    } catch (err) {
      const message = describeError(err) || 'extension heartbeat failed';
      this.recordVpnFailureForAttempt(attemptId, message);
      lifecycle = 'failed';
    }
    if (lifecycle !== 'failed' && lifecycle !== 'disconnected' && lifecycle !== 'stopping') {
      return;
    }
    this.stopHeartbeat();
    this.stopPlatformSubscription();
    this.requestChain = this.requestChain.then(async (): Promise<void> => {
      if (this.isCurrentRequest(generation, attemptId)) {
        await this.destroyVpn();
      }
    });
  }

  private startTelemetry(): void {
    this.stopTelemetry();
    this.persistTelemetry();
    this.telemetryTimer = setInterval(() => this.persistTelemetry(), 1000);
  }

  private stopTelemetry(): void {
    if (this.telemetryTimer >= 0) {
      clearInterval(this.telemetryTimer);
      this.telemetryTimer = -1;
    }
  }

  private persistTelemetry(): void {
    try {
      if (this.activeEngine === 'mihomo') {
        const speed = JSON.parse(mihomoCore.getTraffic(false)) as MihomoTraffic;
        const total = JSON.parse(mihomoCore.getTotalTraffic(false)) as MihomoTraffic;
        pawsUi.updateExternalVpnTraffic(JSON.stringify({
          uploadSpeed: Math.max(0, speed.up),
          downloadSpeed: Math.max(0, speed.down),
          uploadTotal: Math.max(0, total.up),
          downloadTotal: Math.max(0, total.down),
        }));
      } else if (this.activeEngine === 'sing-box') {
        pawsUi.updateExternalVpnTraffic(JSON.stringify({
          uploadSpeed: 0,
          downloadSpeed: 0,
          uploadTotal: 0,
          downloadTotal: 0,
        }));
      } else {
        pawsUi.persistVpnTelemetry();
      }
    } catch (err) {
      hilog.warn(LOG_DOMAIN, LOG_TAG, 'persist VPN telemetry failed %{public}s', describeError(err));
    }
  }

  private trimEngineMemory(reason: string): void {
    if (this.activeEngine !== 'mihomo') {
      return;
    }
    try {
      if (typeof mihomoCore.forceGC === 'function') {
        mihomoCore.forceGC();
      }
    } catch (err) {
      hilog.warn(
        LOG_DOMAIN,
        LOG_TAG,
        'trim engine memory (%{public}s) failed %{public}s',
        reason,
        describeError(err),
      );
    }
  }

}
