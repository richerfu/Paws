import { Want, wantAgent, WantAgent } from '@kit.AbilityKit';
import { vpnExtension, VpnExtensionAbility } from '@kit.NetworkKit';
import { notificationManager } from '@kit.NotificationKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import pawsUi from 'libpaws_ui.so';
import {
  PawsVpnConfig,
  PAWS_ENTRY_ABILITY,
  PAWS_MODULE_NAME,
  parseOptions,
  readPlatformSharedMemoryFds,
  readPlatformStartAttemptId,
  readOptionsJson,
  currentPawsBundleName,
  setPawsBundleName,
} from './VpnConfig';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsVpn';
const VPN_SPEED_NOTIFICATION_ID: number = 1001;
const NOTIFICATION_RETRY_DELAY_MS: number = 30000;
const PLATFORM_OPERATION_TIMEOUT_MS: number = 30000;

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

interface PawsTrafficSnapshot {
  downloadSpeed?: number;
  uploadSpeed?: number;
}

interface PawsRuntimeSnapshot {
  traffic?: PawsTrafficSnapshot;
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

function formatSpeed(bytesPerSecond: number | undefined): string {
  const value = Math.max(0, bytesPerSecond ?? 0);
  if (value < 1024) {
    return `${Math.round(value)} B/s`;
  }
  if (value < 1024 * 1024) {
    const kilobytes = value / 1024;
    return `${kilobytes < 10 ? kilobytes.toFixed(1) : Math.round(kilobytes)} KB/s`;
  }
  if (value < 1024 * 1024 * 1024) {
    const megabytes = value / (1024 * 1024);
    return `${megabytes < 10 ? megabytes.toFixed(1) : Math.round(megabytes)} MB/s`;
  }
  const gigabytes = value / (1024 * 1024 * 1024);
  return `${gigabytes < 10 ? gigabytes.toFixed(1) : Math.round(gigabytes)} GB/s`;
}

export default class PawsVpnExtensionAbility extends VpnExtensionAbility {
  private vpnConnection?: vpnExtension.VpnConnection;
  private tunFd: number = -1;
  private starting: boolean = false;
  private telemetryTimer: number = -1;
  private notificationPublishing: boolean = false;
  private notificationFailureLogged: boolean = false;
  private notificationRetryAt: number = 0;
  private notificationGeneration: number = 0;
  private lastNotificationText: string = '';
  private notificationWantAgent?: WantAgent;
  private notificationWantAgentRequest?: Promise<WantAgent>;
  private platformSubscriptionActive: boolean = false;
  private platformSubscriptionGeneration: number = 0;
  private requestGeneration: number = 0;
  private inFlightAttemptId: string = '';
  private activeAttemptId: string = '';
  private heartbeatTimer: number = -1;
  private requestChain: Promise<void> = Promise.resolve();
  private pendingPlatformOperation?: Promise<void>;
  private pendingPlatformOperationConnection?: vpnExtension.VpnConnection;
  private pendingNativePrepare?: Promise<void>;
  private pendingNativePrepareAttemptId: string = '';
  private pendingCleanup?: Promise<string>;
  private pendingCleanupConnection?: vpnExtension.VpnConnection;

  public onCreate(want: Want): void {
    super.onCreate(want);
    hilog.info(LOG_DOMAIN, LOG_TAG, 'onCreate');
    this.startFromWant(want);
  }

  public onRequest(want: Want, startId: number): void {
    hilog.info(LOG_DOMAIN, LOG_TAG, 'onRequest %{public}d', startId);
    this.startFromWant(want);
  }

  public onDestroy(): void {
    hilog.info(LOG_DOMAIN, LOG_TAG, 'onDestroy');
    this.requestGeneration += 1;
    this.stopHeartbeat();
    this.stopPlatformSubscription();
    this.destroyVpn().catch((err: Error): void => {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'onDestroy cleanup failed %{public}s', describeError(err));
    });
  }

  private configureNativeHome(): boolean {
    try {
      setPawsBundleName(this.context.applicationInfo.name);
      pawsUi.configureAppHome(`${this.context.filesDir}/paws`);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configured native home');
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
      hilog.warn(LOG_DOMAIN, LOG_TAG,
        'VPN authorization bootstrap has no shared-memory descriptors; waiting for rebound request');
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
    if (!this.isLatestWant(sharedMemory.ashmemFd, sharedMemory.notificationFd, attemptId)) {
      return;
    }
    // The authorization workaround intentionally redispatches the same Want.
    // Validate its descriptors first, then keep an already queued, starting,
    // or connected owner intact instead of superseding it with itself.
    if (attemptId === this.inFlightAttemptId || attemptId === this.activeAttemptId) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'ignored duplicate VPN request %{public}s', attemptId);
      return;
    }
    const generation = ++this.requestGeneration;
    this.inFlightAttemptId = attemptId;
    this.requestChain = this.requestChain.then(async (): Promise<void> => {
      await this.handleRequest(
        sharedMemory.ashmemFd,
        sharedMemory.notificationFd,
        attemptId,
        optionsJson,
        generation,
      );
    }).catch(async (err: Error): Promise<void> => {
      if (generation !== this.requestGeneration || this.inFlightAttemptId !== attemptId) {
        hilog.info(LOG_DOMAIN, LOG_TAG, 'ignored stale VPN request completion');
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
    ashmemFd: number,
    notificationFd: number,
    attemptId: string,
    optionsJson: string,
    generation: number,
  ): Promise<void> {
    if (generation !== this.requestGeneration) {
      return;
    }
    await this.waitForPendingCleanup();
    if (generation !== this.requestGeneration ||
      !this.isLatestWant(ashmemFd, notificationFd, attemptId)) {
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
    pawsUi.attachPlatformSharedMemory(ashmemFd, notificationFd);
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
    const config = new PawsVpnConfig(options);
    const connection = vpnExtension.createVpnConnection(this.context);
    this.vpnConnection = connection;
    const startedAt = Date.now();
    const nativePrepare: Promise<string> = pawsUi.prepareVpn(attemptId)
      .then((_loaded: boolean): string => '')
      .catch((err: Error): string => describeError(err) || 'prepare native VPN failed');
    const nativePrepareBarrier: Promise<void> = nativePrepare.then((_error: string): void => {});
    this.pendingNativePrepare = nativePrepareBarrier;
    this.pendingNativePrepareAttemptId = attemptId;
    nativePrepareBarrier.finally((): void => {
      if (this.pendingNativePrepare === nativePrepareBarrier) {
        this.pendingNativePrepare = undefined;
        this.pendingNativePrepareAttemptId = '';
      }
    });

    let createOperation: Promise<number>;
    try {
      createOperation = connection.create(config);
    } catch (err) {
      throw new Error(`VpnConnection.create rejected synchronously: ${describeError(err)}`);
    }
    const fd = await this.awaitPlatformOperation(connection, createOperation, 'VpnConnection.create');
    if (!this.isCurrentRequest(generation, attemptId) || this.vpnConnection !== connection) {
      await this.destroyVpn();
      throw new Error('stale VPN request generation after TUN create');
    }
    this.tunFd = fd;
    hilog.info(LOG_DOMAIN, LOG_TAG, 'created tun fd %{public}d in %{public}d ms',
      fd, Date.now() - startedAt);

    try {
      await this.awaitPlatformOperation(connection, connection.protectProcessNet(),
        'VpnConnection.protectProcessNet');
      this.requireCurrentRequest(generation, attemptId, connection);
      if (!pawsUi.setPlatformNetworkProtected(attemptId, true)) {
        throw new Error('platform VPN protection result belongs to a stale attempt');
      }
    } catch (err) {
      const message = describeError(err) || 'protect process network failed';
      pawsUi.setPlatformNetworkProtected(attemptId, false, message);
      throw new Error(message);
    }

    const prepareError = await this.awaitPlatformOperation(connection, nativePrepare, 'native VPN prepare');
    if (prepareError.length > 0) {
      throw new Error(prepareError);
    }
    this.requireCurrentRequest(generation, attemptId, connection);
    await this.awaitPlatformOperation(
      connection,
      pawsUi.startVpn(fd, optionsJson, attemptId),
      'native VPN start',
    );
    this.requireCurrentRequest(generation, attemptId, connection);
    const lifecycle = pawsUi.extensionTick(attemptId);
    if (lifecycle !== 'connected') {
      throw new Error(`native VPN worker entered ${lifecycle}`);
    }
    this.startTelemetry();
    this.startHeartbeat(generation, attemptId);
    hilog.info(LOG_DOMAIN, LOG_TAG, 'VPN started in %{public}d ms', Date.now() - startedAt);
  }

  private recordVpnFailureForAttempt(attemptId: string, message: string): void {
    this.stopSpeedNotification();
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
    const error = await cleanup;
    if (this.pendingCleanup === cleanup && error.length === 0) {
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
    this.stopTelemetry();
    this.stopSpeedNotification();
    const connection = this.vpnConnection;
    const attemptId = this.activeAttemptId;
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

  private startHeartbeat(generation: number, attemptId: string): void {
    this.stopHeartbeat();
    this.reconcileExtensionTerminal();
    if (!this.isCurrentRequest(generation, attemptId) || this.tunFd < 0) {
      return;
    }
    this.heartbeatTimer = setInterval((): void => {
      if (!this.isCurrentRequest(generation, attemptId)) {
        this.stopHeartbeat();
        return;
      }
      this.reconcileExtensionTerminal();
    }, 3000);
  }

  private stopHeartbeat(): void {
    if (this.heartbeatTimer >= 0) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = -1;
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

  private persistTelemetry(publishNotification: boolean = true): void {
    try {
      pawsUi.persistVpnTelemetry();
      if (publishNotification) {
        this.publishSpeedNotification();
      }
    } catch (err) {
      hilog.warn(LOG_DOMAIN, LOG_TAG, 'persist VPN telemetry failed %{public}s', describeError(err));
    }
  }

  private getNotificationWantAgent(): Promise<WantAgent> {
    if (this.notificationWantAgent) {
      return Promise.resolve(this.notificationWantAgent);
    }
    if (this.notificationWantAgentRequest) {
      return this.notificationWantAgentRequest;
    }
    let request: Promise<WantAgent>;
    try {
      request = wantAgent.getWantAgent({
        wants: [
          {
            bundleName: currentPawsBundleName(),
            moduleName: PAWS_MODULE_NAME,
            abilityName: PAWS_ENTRY_ABILITY,
            parameters: {
              pawsNotification: true,
            },
          },
        ],
        actionType: wantAgent.OperationType.START_ABILITY,
        requestCode: VPN_SPEED_NOTIFICATION_ID,
        actionFlags: [wantAgent.WantAgentFlags.UPDATE_PRESENT_FLAG],
      });
    } catch (err) {
      return Promise.reject(new Error(`create notification want agent failed: ${describeError(err)}`));
    }
    this.notificationWantAgentRequest = request;
    request
      .then((agent: WantAgent) => {
        this.notificationWantAgent = agent;
      })
      .catch(() => {})
      .finally(() => {
        if (this.notificationWantAgentRequest === request) {
          this.notificationWantAgentRequest = undefined;
        }
      });
    return request;
  }

  private async publishSpeedNotification(): Promise<void> {
    if (this.notificationPublishing || Date.now() < this.notificationRetryAt) {
      return;
    }

    let snapshot: PawsRuntimeSnapshot;
    try {
      snapshot = JSON.parse(pawsUi.querySnapshot()) as PawsRuntimeSnapshot;
    } catch (err) {
      hilog.warn(LOG_DOMAIN, LOG_TAG, 'read notification telemetry failed %{public}s', describeError(err));
      return;
    }
    const text = `↓ ${formatSpeed(snapshot.traffic?.downloadSpeed)}  ·  ↑ ${formatSpeed(snapshot.traffic?.uploadSpeed)}`;
    if (text === this.lastNotificationText) {
      return;
    }

    const generation = this.notificationGeneration;
    this.notificationPublishing = true;
    try {
      const notificationWantAgent = await this.getNotificationWantAgent();
      if (generation !== this.notificationGeneration) {
        return;
      }
      const request: notificationManager.NotificationRequest = {
        id: VPN_SPEED_NOTIFICATION_ID,
        notificationSlotType: notificationManager.SlotType.SERVICE_INFORMATION,
        isOngoing: true,
        isUnremovable: true,
        isAlertOnce: true,
        tapDismissed: false,
        showDeliveryTime: false,
        wantAgent: notificationWantAgent,
        content: {
          notificationContentType: notificationManager.ContentType.NOTIFICATION_CONTENT_BASIC_TEXT,
          normal: {
            title: 'Paws · VPN',
            text,
            additionalText: '实时网速',
          },
        },
      };
      await notificationManager.publish(request);
      if (generation !== this.notificationGeneration) {
        notificationManager.cancel(VPN_SPEED_NOTIFICATION_ID).catch(() => {});
        return;
      }
      this.lastNotificationText = text;
      this.notificationFailureLogged = false;
      this.notificationRetryAt = 0;
    } catch (err) {
      this.notificationRetryAt = Date.now() + NOTIFICATION_RETRY_DELAY_MS;
      if (!this.notificationFailureLogged) {
        hilog.warn(LOG_DOMAIN, LOG_TAG, 'publish VPN speed notification failed %{public}s', describeError(err));
        this.notificationFailureLogged = true;
      }
    } finally {
      this.notificationPublishing = false;
    }
  }

  private stopSpeedNotification(): void {
    this.notificationGeneration += 1;
    this.lastNotificationText = '';
    this.notificationRetryAt = 0;
    this.notificationFailureLogged = false;
    notificationManager.cancel(VPN_SPEED_NOTIFICATION_ID).catch(() => {});
  }

}
