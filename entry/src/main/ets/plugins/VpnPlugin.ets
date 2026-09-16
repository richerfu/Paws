import {
  AsyncPluginBase,
  BridgeCallContext,
  BridgeContextRequirement,
  BridgePluginContext,
  BridgePluginHookContext,
  BridgeTypedValue,
} from '@ohos-rs/ability';
import { vpnExtension } from '@kit.NetworkKit';
import { notificationManager } from '@kit.NotificationKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import pawsUi from 'libpaws_ui.so';
import { describeError } from '../common/Errors';
import { buildVpnWant, PawsPlatformSharedMemoryFds } from '../vpnability/VpnConfig';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsVpn';

const VPN_START_REQUEST_TYPE = 'paws.VpnStartRequest';
const VPN_START_RESPONSE_TYPE = 'paws.VpnStartResponse';
const VPN_STOP_REQUEST_TYPE = 'paws.VpnStopRequest';
const VPN_STOP_RESPONSE_TYPE = 'paws.VpnStopResponse';
const VPN_OWNED_STOP_REQUEST_TYPE = 'paws.VpnOwnedStopRequest';
const VPN_OWNED_STOP_RESPONSE_TYPE = 'paws.VpnOwnedStopResponse';
const VPN_RESTART_REQUEST_TYPE = 'paws.VpnRestartRequest';
const VPN_RESTART_RESPONSE_TYPE = 'paws.VpnRestartResponse';
const VPN_OPERATION_LOOKUP_REQUEST_TYPE = 'paws.VpnOperationLookupRequest';
const VPN_OPERATION_LOOKUP_RESPONSE_TYPE = 'paws.VpnOperationLookupResponse';
const VPN_OPERATION_WAIT_REQUEST_TYPE = 'paws.VpnOperationWaitRequest';
const VPN_OPERATION_WAIT_RESPONSE_TYPE = 'paws.VpnOperationWaitResponse';
const VPN_OPERATION_WAIT_MAX_MS: number = 45000;
const MAX_RETAINED_VPN_OPERATIONS: number = 16;
const VPN_COOPERATIVE_STOP_TIMEOUT_MS: number = 3000;

let vpnPluginInstanceSequence: number = 0;
let vpnOperationChain: Promise<void> = Promise.resolve();

interface VpnStartRequest {
  requestId: string;
  optionsJson: string;
}

interface VpnStopRequest {
  requestId: string;
}

interface VpnRestartRequest {
  requestId: string;
  expectedSessionId: string;
  expectedConfigRevision: string;
  optionsJson: string;
}

interface VpnOwnedStopRequest {
  requestId: string;
  expectedSessionId: string;
  expectedConfigRevision: string;
}

interface VpnOperationWaitRequest {
  operationId: string;
  waitMs: number;
}

interface VpnOperationLookupRequest {
  requestId: string;
}

interface VpnOperationLookupResponse {
  status: string;
  operationId: string;
  operationStatus: string;
  result: boolean;
  error: string;
}

interface VpnOperationTerminal {
  status: string;
  result: boolean;
  error: string;
}

interface VpnOperationRecord {
  requestId: string;
  action: string;
  payloadFingerprint: string;
  settled: Promise<void>;
  terminal?: VpnOperationTerminal;
}

/**
 * `paws.vpn` — system VPN extension control. Owns the full start orchestration
 * (shared-memory initialization, durable owner reconciliation,
 * first-authorization redispatch and outcome reporting) so the Ability entry
 * stays a thin assembly point.
 */
export class VpnPlugin extends AsyncPluginBase {
  readonly id = 'paws.vpn';
  readonly requires: BridgeContextRequirement[] = ['ability'];

  private vpnStartRequest: Promise<void> | undefined = undefined;
  private pendingVpnStartDispatches: Map<string, Promise<void>> = new Map();
  private activeVpnStartAttemptId: string = '';
  private connectedVpnSessionId: string = '';
  private operationGeneration: number = 0;
  private operationChain: Promise<void> = Promise.resolve();
  private platformSharedMemory?: PawsPlatformSharedMemoryFds;
  private readonly bridgeSessionNonce: string =
    `${Date.now().toString(36)}-${++vpnPluginInstanceSequence}-${Math.floor(Math.random() * 0x100000000).toString(36)}`;
  private nextBridgeOperationId: number = 0;
  private bridgeOperations: Map<string, VpnOperationRecord> = new Map();
  private requestOperations: Map<string, string> = new Map();

  onInstall(_context: BridgePluginHookContext): void {
    this.initializePlatformSharedMemory();
  }

  async invokeAsync(
    action: string,
    request: BridgeTypedValue,
    _context: BridgeCallContext,
  ): Promise<BridgeTypedValue> {
    if (action === 'start-vpn') {
      if (request.typeName !== VPN_START_REQUEST_TYPE || typeof request.value !== 'object') {
        throw new Error(`paws.vpn expected ${VPN_START_REQUEST_TYPE}, got ${request.typeName}`);
      }
      const payload = request.value as VpnStartRequest;
      if (typeof payload.requestId !== 'string' || typeof payload.optionsJson !== 'string') {
        throw new Error(`paws.vpn invalid start request shape`);
      }
      const operationId = this.submitBridgeOperation(
        payload.requestId,
        action,
        payload.optionsJson,
        (): Promise<void> =>
        this.requestStartVpn(payload.optionsJson));
      return { typeName: VPN_START_RESPONSE_TYPE, value: { operationId } };
    }
    if (action === 'stop-vpn') {
      if (request.typeName !== VPN_STOP_REQUEST_TYPE || typeof request.value !== 'object') {
        throw new Error(`paws.vpn expected ${VPN_STOP_REQUEST_TYPE}, got ${request.typeName}`);
      }
      const payload = request.value as VpnStopRequest;
      if (typeof payload.requestId !== 'string') {
        throw new Error('paws.vpn invalid stop request shape');
      }
      const operationId = this.submitBridgeOperation(
        payload.requestId,
        action,
        '',
        (): Promise<boolean> => this.requestStopVpn(),
      );
      return { typeName: VPN_STOP_RESPONSE_TYPE, value: { operationId } };
    }
    if (action === 'stop-vpn-if-current') {
      if (request.typeName !== VPN_OWNED_STOP_REQUEST_TYPE || typeof request.value !== 'object') {
        throw new Error(`paws.vpn expected ${VPN_OWNED_STOP_REQUEST_TYPE}, got ${request.typeName}`);
      }
      const payload = request.value as VpnOwnedStopRequest;
      if (typeof payload.requestId !== 'string' ||
        typeof payload.expectedSessionId !== 'string' ||
        typeof payload.expectedConfigRevision !== 'string') {
        throw new Error('paws.vpn invalid owned stop request shape');
      }
      const operationId = this.submitBridgeOperation(
        payload.requestId,
        action,
        `${payload.expectedSessionId}\u0000${payload.expectedConfigRevision}`,
        (): Promise<boolean> =>
        this.requestStopVpnIfCurrent(payload.expectedSessionId, payload.expectedConfigRevision));
      return { typeName: VPN_OWNED_STOP_RESPONSE_TYPE, value: { operationId } };
    }
    if (action === 'restart-vpn') {
      if (request.typeName !== VPN_RESTART_REQUEST_TYPE || typeof request.value !== 'object') {
        throw new Error(`paws.vpn expected ${VPN_RESTART_REQUEST_TYPE}, got ${request.typeName}`);
      }
      const payload = request.value as VpnRestartRequest;
      if (typeof payload.requestId !== 'string' ||
        typeof payload.expectedSessionId !== 'string' ||
        typeof payload.expectedConfigRevision !== 'string' ||
        typeof payload.optionsJson !== 'string') {
        throw new Error('paws.vpn invalid restart request shape');
      }
      const operationId = this.submitBridgeOperation(
        payload.requestId,
        action,
        `${payload.expectedSessionId}\u0000${payload.expectedConfigRevision}\u0000${payload.optionsJson}`,
        (): Promise<boolean> =>
        this.requestRestartVpn(
          payload.expectedSessionId,
          payload.expectedConfigRevision,
          payload.optionsJson,
        ));
      return { typeName: VPN_RESTART_RESPONSE_TYPE, value: { operationId } };
    }
    if (action === 'lookup-vpn-operation') {
      if (request.typeName !== VPN_OPERATION_LOOKUP_REQUEST_TYPE || typeof request.value !== 'object') {
        throw new Error(`paws.vpn expected ${VPN_OPERATION_LOOKUP_REQUEST_TYPE}, got ${request.typeName}`);
      }
      const payload = request.value as VpnOperationLookupRequest;
      if (typeof payload.requestId !== 'string' || payload.requestId.length === 0) {
        throw new Error('paws.vpn invalid operation lookup request shape');
      }
      return {
        typeName: VPN_OPERATION_LOOKUP_RESPONSE_TYPE,
        value: this.lookupBridgeOperation(payload.requestId),
      };
    }
    if (action === 'await-vpn-operation') {
      if (request.typeName !== VPN_OPERATION_WAIT_REQUEST_TYPE || typeof request.value !== 'object') {
        throw new Error(`paws.vpn expected ${VPN_OPERATION_WAIT_REQUEST_TYPE}, got ${request.typeName}`);
      }
      const payload = request.value as VpnOperationWaitRequest;
      if (typeof payload.operationId !== 'string' ||
        typeof payload.waitMs !== 'number' || !Number.isInteger(payload.waitMs) ||
        payload.waitMs < 1 || payload.waitMs > VPN_OPERATION_WAIT_MAX_MS) {
        throw new Error('paws.vpn invalid operation wait request shape');
      }
      return {
        typeName: VPN_OPERATION_WAIT_RESPONSE_TYPE,
        value: await this.awaitBridgeOperation(payload.operationId, payload.waitMs),
      };
    }
    throw new Error(`Unsupported paws.vpn action '${action}'`);
  }

  private submitBridgeOperation(
    requestId: string,
    action: string,
    payloadFingerprint: string,
    create: () => Promise<void | boolean>,
  ): string {
    if (requestId.length === 0) {
      throw new Error('VPN operation requestId must not be empty');
    }
    const existingOperationId = this.requestOperations.get(requestId);
    if (existingOperationId !== undefined) {
      const existing = this.bridgeOperations.get(existingOperationId);
      if (existing === undefined) {
        throw new Error('VPN operation receipt mapping is no longer observable');
      }
      if (existing.action !== action || existing.payloadFingerprint !== payloadFingerprint) {
        throw new Error(`VPN operation requestId '${requestId}' was reused with a different request`);
      }
      return existingOperationId;
    }
    this.reclaimCompletedBridgeOperations();
    if (this.bridgeOperations.size >= MAX_RETAINED_VPN_OPERATIONS) {
      throw new Error('too many active VPN platform operations');
    }
    const operationId = `${this.bridgeSessionNonce}:${++this.nextBridgeOperationId}`;
    const record: VpnOperationRecord = {
      requestId,
      action,
      payloadFingerprint,
      settled: Promise.resolve(),
    };
    let operation: Promise<void | boolean>;
    try {
      operation = create();
    } catch (err) {
      operation = Promise.reject(err);
    }
    record.settled = operation.then(
      (result?: void | boolean): void => {
        record.terminal = {
          status: 'succeeded',
          result: typeof result === 'boolean' ? result : true,
          error: '',
        };
      },
      (err: Error): void => {
        record.terminal = {
          status: 'failed',
          result: false,
          error: describeError(err) || 'VPN platform operation failed',
        };
      },
    );
    this.bridgeOperations.set(operationId, record);
    this.requestOperations.set(requestId, operationId);
    return operationId;
  }

  private reclaimCompletedBridgeOperations(): void {
    if (this.bridgeOperations.size < MAX_RETAINED_VPN_OPERATIONS) {
      return;
    }
    for (const operationId of this.bridgeOperations.keys()) {
      const record = this.bridgeOperations.get(operationId);
      if (record !== undefined && record.terminal !== undefined) {
        this.bridgeOperations.delete(operationId);
        if (this.requestOperations.get(record.requestId) === operationId) {
          this.requestOperations.delete(record.requestId);
        }
      }
      if (this.bridgeOperations.size < MAX_RETAINED_VPN_OPERATIONS) {
        return;
      }
    }
  }

  private async awaitBridgeOperation(
    operationId: string,
    waitMs: number,
  ): Promise<VpnOperationTerminal> {
    if (!operationId.startsWith(`${this.bridgeSessionNonce}:`)) {
      return { status: 'unavailable', result: false, error: '' };
    }
    const record = this.bridgeOperations.get(operationId);
    if (!record) {
      return { status: 'unavailable', result: false, error: '' };
    }
    if (record.terminal === undefined) {
      let timeoutId: number = -1;
      const timeout = new Promise<void>((resolve): void => {
        timeoutId = setTimeout(resolve, waitMs);
      });
      await Promise.race([record.settled, timeout]);
      if (timeoutId >= 0) {
        clearTimeout(timeoutId);
      }
    }
    if (record.terminal === undefined) {
      return { status: 'pending', result: false, error: '' };
    }
    return record.terminal;
  }

  private lookupBridgeOperation(requestId: string): VpnOperationLookupResponse {
    const operationId = this.requestOperations.get(requestId);
    if (operationId === undefined) {
      return {
        status: 'unknown-session',
        operationId: '',
        operationStatus: 'unknown',
        result: false,
        error: '',
      };
    }
    const record = this.bridgeOperations.get(operationId);
    if (record === undefined) {
      return {
        status: 'unknown-session',
        operationId: '',
        operationStatus: 'unknown',
        result: false,
        error: '',
      };
    }
    const terminal = record.terminal;
    return {
      status: 'found',
      operationId,
      operationStatus: terminal?.status ?? 'pending',
      result: terminal?.result ?? false,
      error: terminal?.error ?? '',
    };
  }

  /**
   * Public entry for in-app callers (debug automation). Resolves the plugin
   * session context that BridgeHost injected on install.
   */
  async requestStartVpn(optionsJson: string): Promise<void> {
    await this.requestStartVpnWithContext(optionsJson, this.getContext());
  }

  /** Public entry for in-app callers (debug automation). */
  async requestStopVpn(): Promise<boolean> {
    return this.requestStopVpnWithContext(this.getContext());
  }

  async requestRestartVpn(
    expectedSessionId: string,
    expectedConfigRevision: string,
    optionsJson: string,
  ): Promise<boolean> {
    if (!pawsUi.isPlatformVpnSessionCurrent(expectedSessionId, expectedConfigRevision)) {
      return false;
    }
    const intentEpoch = pawsUi.advancePlatformVpnIntent();
    const generation = ++this.operationGeneration;
    const claimedSessionId = pawsUi.claimCurrentPlatformVpnStop(intentEpoch);
    if (claimedSessionId !== expectedSessionId) {
      return false;
    }
    return this.enqueueOperation(async (): Promise<boolean> => {
      if (generation !== this.operationGeneration ||
        !pawsUi.isPlatformVpnIntentCurrent(intentEpoch) ||
        !pawsUi.isPlatformVpnSessionCurrent(expectedSessionId, expectedConfigRevision)) {
        return false;
      }
      const stopped = await this.performStopVpn(expectedSessionId, intentEpoch, (): boolean =>
        generation === this.operationGeneration &&
          pawsUi.isPlatformVpnIntentCurrent(intentEpoch) &&
          pawsUi.isPlatformVpnSessionCurrent(expectedSessionId, expectedConfigRevision));
      if (!stopped) {
        return false;
      }
      if (generation !== this.operationGeneration ||
        !pawsUi.isPlatformVpnIntentCurrent(intentEpoch) ||
        !pawsUi.isRuntimeConfigRevisionCurrent(expectedConfigRevision)) {
        return false;
      }
      await this.performStartVpn(optionsJson, this.getContext(), generation, intentEpoch);
      return true;
    });
  }

  async requestStopVpnIfCurrent(
    expectedSessionId: string,
    expectedConfigRevision: string,
  ): Promise<boolean> {
    // Invalid/stale requests must not supersede an unrelated in-flight user
    // operation. Revalidate at the serialized queue head before touching the
    // system VPN owner.
    if (!pawsUi.isPlatformVpnSessionCurrent(expectedSessionId, expectedConfigRevision)) {
      return false;
    }
    const intentEpoch = pawsUi.advancePlatformVpnIntent();
    const generation = ++this.operationGeneration;
    const claimedSessionId = pawsUi.claimCurrentPlatformVpnStop(intentEpoch);
    if (claimedSessionId !== expectedSessionId) {
      return false;
    }
    return this.enqueueOperation(async (): Promise<boolean> => {
      if (generation !== this.operationGeneration ||
        !pawsUi.isPlatformVpnIntentCurrent(intentEpoch) ||
        !pawsUi.isPlatformVpnSessionCurrent(expectedSessionId, expectedConfigRevision)) {
        return false;
      }
      return this.performStopVpn(expectedSessionId, intentEpoch, (): boolean =>
        generation === this.operationGeneration &&
          pawsUi.isPlatformVpnIntentCurrent(intentEpoch) &&
          pawsUi.isPlatformVpnSessionCurrent(expectedSessionId, expectedConfigRevision));
    });
  }

  private enqueueOperation<T>(operation: () => Promise<T>): Promise<T> {
    const localReady = this.operationChain.catch((_err: Error): void => {});
    const request = vpnOperationChain
      .catch((_err: Error): void => {})
      .then((): Promise<void> => localReady)
      .then(operation);
    this.operationChain = request.then((_value: T): void => {}, (_err: Error): void => {});
    vpnOperationChain = this.operationChain;
    return request;
  }

  private initializePlatformSharedMemory(): void {
    try {
      const fields = pawsUi.initializePlatformSharedMemory().split(',');
      const ashmemFd = Number(fields[0]);
      const notificationFd = Number(fields[1]);
      if (fields.length !== 2 || !Number.isInteger(ashmemFd) || ashmemFd < 0 ||
        !Number.isInteger(notificationFd) || notificationFd < 0) {
        throw new Error('native platform shared memory returned invalid file descriptors');
      }
      this.platformSharedMemory = { ashmemFd, notificationFd };
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'initialize platform shared memory failed %{public}s',
        describeError(err));
      throw new Error(describeError(err) || 'initialize platform shared memory failed');
    }
  }

  private async requestStartVpnWithContext(
    optionsJson: string,
    context: BridgePluginContext,
  ): Promise<void> {
    if (this.vpnStartRequest) {
      return this.vpnStartRequest;
    }
    const intentEpoch = pawsUi.advancePlatformVpnIntent();
    const generation = ++this.operationGeneration;
    const request = this.enqueueOperation(async (): Promise<void> => {
      if (generation !== this.operationGeneration ||
        !pawsUi.isPlatformVpnIntentCurrent(intentEpoch)) {
        throw new Error('VPN start request was superseded');
      }
      await this.performStartVpn(optionsJson, context, generation, intentEpoch);
    });
    this.vpnStartRequest = request;
    try {
      await request;
    } finally {
      if (this.vpnStartRequest === request) {
        this.vpnStartRequest = undefined;
      }
    }
  }

  private dispatchVpnStart(
    optionsJson: string,
    attemptId: string,
    context: BridgePluginContext,
    generation: number,
    intentEpoch: string,
  ): void {
    const dispatch = this.runVpnStartDispatch(
      optionsJson,
      attemptId,
      context,
      generation,
      intentEpoch,
    ).catch((err: Error) => {
      const message = describeError(err) || 'system rejected VPN start';
      try {
        pawsUi.failUnattachedPlatformVpnStart(attemptId, message);
      } catch (innerErr) {
        hilog.error(
          LOG_DOMAIN,
          LOG_TAG,
          'publish system VPN rejection failed %{public}s',
          describeError(innerErr),
        );
      }
    });
    this.pendingVpnStartDispatches.set(attemptId, dispatch);
    dispatch.finally((): void => {
      if (this.pendingVpnStartDispatches.get(attemptId) === dispatch) {
        this.pendingVpnStartDispatches.delete(attemptId);
      }
    });
  }

  private async invokeVpnExtensionStart(optionsJson: string, attemptId: string): Promise<void> {
    try {
      await vpnExtension.startVpnExtensionAbility(
        buildVpnWant(optionsJson, this.platformSharedMemory, attemptId),
      );
    } catch (err) {
      throw new Error(describeError(err) || 'system rejected VPN start');
    }
  }

  private async runVpnStartDispatch(
    optionsJson: string,
    attemptId: string,
    _context: BridgePluginContext,
    generation: number,
    intentEpoch: string,
  ): Promise<void> {
    await this.invokeVpnExtensionStart(optionsJson, attemptId);
    if (generation !== this.operationGeneration ||
      !pawsUi.isPlatformVpnIntentCurrent(intentEpoch)) {
      return;
    }

    // The first-authorization dialog on some HarmonyOS builds starts the VPN
    // Extension with a replacement Want containing only bundleName/abilityName.
    // Redispatch the original payload unconditionally once the system promise
    // resolves: the Extension is either running (the Want is delivered again
    // via onRequest) or was already started with the full parameters (the
    // repeated start is idempotent). The terminal outcome is owned entirely
    // by the Extension's shared-memory events, so no timed attach probing is
    // needed.
    await this.invokeVpnExtensionStart(optionsJson, attemptId);
  }

  private async performStartVpn(
    optionsJson: string,
    context: BridgePluginContext,
    generation: number,
    intentEpoch: string,
  ): Promise<void> {
    let attemptId: string = '';
    try {
      if (generation !== this.operationGeneration ||
        !pawsUi.isPlatformVpnIntentCurrent(intentEpoch)) {
        throw new Error('VPN start request was superseded');
      }
      const recoverableSessionId = pawsUi.claimCurrentPlatformVpnStop(intentEpoch);
      if (recoverableSessionId.length > 0 ||
        pawsUi.isPlatformVpnStopCurrent(intentEpoch, recoverableSessionId)) {
        // A cold/recreated UI may have no process-local session state while
        // the durable journal still owns a Pending or Attached attempt. Fence
        // and reconcile that exact owner before publishing a replacement.
        await this.performStopVpn(recoverableSessionId, intentEpoch);
        if (generation !== this.operationGeneration ||
          !pawsUi.isPlatformVpnIntentCurrent(intentEpoch)) {
          throw new Error('VPN start request was superseded');
        }
      }
      attemptId = pawsUi.beginPlatformVpnStartForIntent(intentEpoch);
      this.activeVpnStartAttemptId = attemptId;

      this.dispatchVpnStart(optionsJson, attemptId, context, generation, intentEpoch);

      const outcome = await pawsUi.awaitPlatformVpnStart(attemptId);
      if (generation !== this.operationGeneration ||
        !pawsUi.isPlatformVpnIntentCurrent(intentEpoch)) {
        throw new Error('VPN start request was superseded');
      }
      if (outcome !== 'connected') {
        throw new Error(`unexpected VPN start outcome: ${outcome}`);
      }
      this.ensureSpeedNotificationPermission(context).catch((err: Error) => {
        hilog.warn(LOG_DOMAIN, LOG_TAG, 'deferred notification permission failed %{public}s',
          describeError(err));
      });
      hilog.info(LOG_DOMAIN, LOG_TAG, 'VPN start completed attempt %{public}s', attemptId);
      this.connectedVpnSessionId = attemptId;
    } catch (err) {
      const message = describeError(err) || 'request VPN start failed';
      if (attemptId.length > 0) {
        try {
          pawsUi.failPlatformVpnStart(attemptId, message);
        } catch (innerErr) {
          hilog.error(
            LOG_DOMAIN,
            LOG_TAG,
            'publish VPN start failure failed %{public}s',
            describeError(innerErr),
          );
        }
      }
      hilog.error(LOG_DOMAIN, LOG_TAG, 'request VPN start failed %{public}s', message);
      throw new Error(message);
    } finally {
      if (this.activeVpnStartAttemptId === attemptId) {
        this.activeVpnStartAttemptId = '';
      }
    }
  }

  private async ensureSpeedNotificationPermission(context: BridgePluginContext): Promise<void> {
    try {
      if (await notificationManager.isNotificationEnabled()) {
        return;
      }
      await notificationManager.requestEnableNotification(context.abilityContext);
    } catch (err) {
      // Notification permission must never prevent the user from starting VPN.
      hilog.warn(LOG_DOMAIN, LOG_TAG, 'notification permission unavailable %{public}s',
        describeError(err));
    }
  }

  private async requestStopVpnWithContext(_context: BridgePluginContext): Promise<boolean> {
    const intentEpoch = pawsUi.advancePlatformVpnIntent();
    ++this.operationGeneration;
    // Core owns the cross-Plugin fence. In particular, a recreated Bridge has
    // no trustworthy local active/pending id, while Core can atomically make
    // the exact pending owner terminal before any late Want attaches.
    const sessionId = pawsUi.claimCurrentPlatformVpnStop(intentEpoch);
    return this.enqueueOperation(async (): Promise<boolean> => {
      return this.performStopVpn(sessionId, intentEpoch);
    });
  }

  private async performStopVpn(
    sessionId: string,
    intentEpoch: string,
    isStillAuthorized?: () => boolean,
  ): Promise<boolean> {
    try {
      // The start Promise may remain pending indefinitely after authorization.
      // Core has already persisted an exact terminal/journal fence, so a late
      // Want cannot bind and this Promise is not a stop barrier.
      this.pendingVpnStartDispatches.delete(sessionId);
      // Recheck a scoped stop immediately before touching the OS so a newer
      // intent or config revision cannot stop the session it superseded.
      if ((isStillAuthorized !== undefined && !isStillAuthorized()) ||
        !this.canDispatchPlatformVpnStop(intentEpoch, sessionId)) {
        return false;
      }
      let cleanupObservation: Promise<string> | undefined = undefined;
      let cleanupCompleted = sessionId.length === 0;
      if (sessionId.length > 0 && pawsUi.requestPlatformVpnStop(sessionId)) {
        cleanupObservation = pawsUi.awaitPlatformVpnStop(sessionId).then(
          (completed: boolean): string => completed ? 'complete' : 'superseded',
          (err: Error): string => `error:${describeError(err) || 'platform cleanup wait failed'}`,
        );
        const cooperativeResult = await this.waitForCooperativeCleanup(cleanupObservation);
        if (cooperativeResult === 'complete') {
          cleanupCompleted = true;
        } else if (cooperativeResult === 'superseded') {
          return false;
        } else if (cooperativeResult.startsWith('error:')) {
          hilog.warn(
            LOG_DOMAIN,
            LOG_TAG,
            'cooperative VPN cleanup wait failed %{public}s',
            cooperativeResult.slice('error:'.length),
          );
        } else {
          hilog.warn(
            LOG_DOMAIN,
            LOG_TAG,
            'cooperative VPN cleanup still pending for %{public}s; requesting OS stop',
            sessionId,
          );
        }
      }
      // A newer Plugin can publish an intent while cooperative cleanup is
      // pending. Atomically convert the still-current fence to InFlight and
      // invoke HarmonyOS synchronously before yielding; otherwise the newer
      // owner takes over the stop and this stale operation becomes a no-op.
      if (!this.canDispatchPlatformVpnStop(intentEpoch, sessionId) ||
        !pawsUi.beginPlatformVpnOsStop(intentEpoch, sessionId)) {
        return false;
      }
      try {
        await vpnExtension.stopVpnExtensionAbility(buildVpnWant());
      } catch (err) {
        if (!pawsUi.failPlatformVpnOsStop(intentEpoch, sessionId)) {
          hilog.error(LOG_DOMAIN, LOG_TAG,
            'failed to retain exact VPN OS stop fence after system rejection');
        }
        throw new Error(describeError(err) || 'system rejected VPN stop');
      }
      if (!pawsUi.completePlatformVpnOsStop(intentEpoch, sessionId)) {
        throw new Error(`VPN OS stop confirmation did not match exact fence ${sessionId}`);
      }
      if (sessionId.length > 0 && !cleanupCompleted) {
        const recoveredAfterConfirmedStop =
          await pawsUi.recoverPlatformVpnCleanupAfterConfirmedStop(sessionId);
        cleanupCompleted = recoveredAfterConfirmedStop;
        if (recoveredAfterConfirmedStop) {
          hilog.warn(
            LOG_DOMAIN,
            LOG_TAG,
            'recovered VPN cleanup after confirmed OS stop and exact-owner proof %{public}s',
            sessionId,
          );
        }
        if (!cleanupCompleted) {
          throw new Error(
            `VPN cleanup owner ${sessionId} remained live or was superseded after confirmed OS stop`);
        }
      }
      if (this.connectedVpnSessionId === sessionId) {
        this.connectedVpnSessionId = '';
      }
      if (sessionId.length > 0) {
        hilog.info(LOG_DOMAIN, LOG_TAG, 'VPN stop cleanup confirmed attempt %{public}s', sessionId);
      } else {
        hilog.info(LOG_DOMAIN, LOG_TAG, 'VPN OS stop confirmed with no journaled owner');
      }
      return true;
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'request VPN stop failed %{public}s', describeError(err));
      throw new Error(describeError(err) || 'request VPN stop failed');
    }
  }

  private async waitForCooperativeCleanup(cleanup: Promise<string>): Promise<string> {
    let timeoutId: number = -1;
    const timeout = new Promise<string>((resolve: (value: string) => void): void => {
      timeoutId = setTimeout((): void => resolve('pending'), VPN_COOPERATIVE_STOP_TIMEOUT_MS);
    });
    const result = await Promise.race([cleanup, timeout]);
    if (timeoutId >= 0) {
      clearTimeout(timeoutId);
    }
    return result;
  }

  private canDispatchPlatformVpnStop(intentEpoch: string, sessionId: string): boolean {
    return pawsUi.isPlatformVpnStopCurrent(intentEpoch, sessionId) ||
      (sessionId.length === 0 && pawsUi.isPlatformVpnIntentCurrent(intentEpoch));
  }
}
