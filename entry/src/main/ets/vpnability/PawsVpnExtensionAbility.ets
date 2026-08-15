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
    pawsUi.setPlatformVpnRunning(false);
    this.destroyVpn();
  }

  private configureNativeHome(): void {
    try {
      setPawsBundleName(this.context.applicationInfo.name);
      pawsUi.configureAppHome(`${this.context.filesDir}/paws`);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configured native home');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'configure native home failed %{public}s', describeError(err));
    }
  }

  private startFromWant(want: Want): void {
    this.configureNativeHome();
    const sharedMemory = readPlatformSharedMemoryFds(want);
    if (!sharedMemory) {
      // The first-authorization system dialog may bootstrap the Extension with
      // a replacement Want that contains no application parameters. Keep the
      // Extension idle until EntryAbility redispatches the original Want via
      // onRequest; it owns the bounded failure if that rebound never arrives.
      hilog.warn(
        LOG_DOMAIN,
        LOG_TAG,
        'VPN authorization bootstrap has no shared-memory descriptors; waiting for rebound request',
      );
      return;
    }
    try {
      pawsUi.attachPlatformSharedMemory(sharedMemory.ashmemFd, sharedMemory.notificationFd);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'attached platform shared memory');
      this.startPlatformSubscription();
    } catch (err) {
      this.starting = false;
      this.recordVpnFailure(`attach platform shared memory failed: ${describeError(err)}`);
      return;
    }
    let attemptId: string;
    try {
      attemptId = readPlatformStartAttemptId(want);
      pawsUi.bindPlatformVpnStart(attemptId);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'bound VPN start attempt %{public}s', attemptId);
    } catch (err) {
      const message = describeError(err) || 'VPN request has no valid start attempt';
      this.recordVpnFailure(message);
      hilog.error(LOG_DOMAIN, LOG_TAG, 'VPN request rejected %{public}s', message);
      this.starting = false;
      return;
    }
    if (this.tunFd >= 0) {
      this.starting = false;
      try {
        pawsUi.setPlatformVpnRunning(true);
        hilog.info(LOG_DOMAIN, LOG_TAG, 'reported existing VPN to rebound platform shared memory');
      } catch (err) {
        this.recordVpnFailure(`report existing VPN state failed: ${describeError(err)}`);
      }
      return;
    }
    if (this.starting) {
      try {
        pawsUi.setPlatformVpnStarting(true);
        hilog.info(LOG_DOMAIN, LOG_TAG, 'reported pending VPN to rebound platform shared memory');
      } catch (err) {
        this.recordVpnFailure(`report pending VPN state failed: ${describeError(err)}`);
      }
      return;
    }
    this.starting = true;
    try {
      pawsUi.setPlatformVpnStarting(true);
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'record VPN starting failed %{public}s', describeError(err));
    }
    const optionsJson = readOptionsJson(want);
    const options = parseOptions(optionsJson);
    const config = new PawsVpnConfig(options);
    const startedAt = Date.now();

    try {
      this.vpnConnection = vpnExtension.createVpnConnection(this.context);
      const nativePrepare: Promise<string> = pawsUi.prepareVpn()
        .then((_loaded: boolean) => '')
        .catch((err: Error) => describeError(err) || 'prepare native vpn failed');
      this.vpnConnection
        .create(config)
        .then(async (fd: number) => {
          this.tunFd = fd;
          hilog.info(
            LOG_DOMAIN,
            LOG_TAG,
            'created tun fd %{public}d in %{public}d ms',
            fd,
            Date.now() - startedAt,
          );
          try {
            await this.vpnConnection?.protectProcessNet();
            try {
              pawsUi.setPlatformNetworkProtected(true);
            } catch (stateErr) {
              hilog.error(LOG_DOMAIN, LOG_TAG, 'record protect success failed %{public}s', describeError(stateErr));
            }
            hilog.info(LOG_DOMAIN, LOG_TAG, 'protected process network');
          } catch (err) {
            const message = describeError(err) || 'protect process network failed';
            try {
              pawsUi.setPlatformNetworkProtected(false, message);
            } catch (stateErr) {
              hilog.error(LOG_DOMAIN, LOG_TAG, 'record protect failure failed %{public}s', describeError(stateErr));
            }
            hilog.error(LOG_DOMAIN, LOG_TAG, 'protect process network failed %{public}s', message);
          }
          const prepareError = await nativePrepare;
          if (prepareError.length > 0) {
            throw new Error(prepareError);
          }
          hilog.info(LOG_DOMAIN, LOG_TAG, 'native profile ready in %{public}d ms', Date.now() - startedAt);
          await pawsUi.startVpn(fd, optionsJson);
          this.startTelemetry();
          this.starting = false;
          hilog.info(LOG_DOMAIN, LOG_TAG, 'VPN started in %{public}d ms', Date.now() - startedAt);
        })
        .catch(async (err: Error) => {
          const message = describeError(err) || 'VPN startup pipeline failed';
          hilog.error(LOG_DOMAIN, LOG_TAG, 'VPN startup pipeline failed %{public}s', message);
          await this.destroyVpnConnection();
          this.vpnConnection = undefined;
          this.tunFd = -1;
          this.starting = false;
          this.recordVpnFailure(message);
        });
    } catch (err) {
      const message = describeError(err) || 'start vpn failed';
      hilog.error(LOG_DOMAIN, LOG_TAG, 'start vpn failed %{public}s', message);
      this.vpnConnection = undefined;
      this.tunFd = -1;
      this.starting = false;
      this.recordVpnFailure(message);
    }
  }

  private recordVpnFailure(message: string): void {
    this.stopSpeedNotification();
    try {
      pawsUi.setPlatformVpnFailed(message);
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'record VPN failure failed %{public}s', describeError(err));
    }
  }

  private destroyVpn(): void {
    this.stopPlatformSubscription();
    this.stopTelemetry();
    this.stopSpeedNotification();
    pawsUi.stopVpn()
      .then(() => this.persistTelemetry(false))
      .catch((err: Error) => {
        hilog.error(LOG_DOMAIN, LOG_TAG, 'stop native vpn failed %{public}s', describeError(err));
      });
    if (!this.vpnConnection) {
      this.tunFd = -1;
      this.starting = false;
      return;
    }
    this.destroyVpnConnection();
    this.vpnConnection = undefined;
    this.tunFd = -1;
    this.starting = false;
  }

  private startPlatformSubscription(): void {
    if (this.platformSubscriptionActive) {
      return;
    }
    this.platformSubscriptionActive = true;
    this.runPlatformSubscription();
  }

  private stopPlatformSubscription(): void {
    this.platformSubscriptionActive = false;
    // Wake the waiter parked in waitForPlatformChangeEvent so the loop can
    // observe the flag and unwind without waiting for the next peer frame.
    try {
      pawsUi.cancelPlatformChangeWait();
    } catch (err) {
      hilog.warn(LOG_DOMAIN, LOG_TAG, 'cancel platform change wait failed %{public}s', describeError(err));
    }
  }

  private async runPlatformSubscription(): Promise<void> {
    while (this.platformSubscriptionActive) {
      try {
        // Fully event driven: the native wait parks on the shared-memory
        // notification socket until the UI publishes a control frame (or the
        // wait is cancelled by stopPlatformSubscription). No polling.
        await pawsUi.waitForPlatformChangeEvent();
        if (this.platformSubscriptionActive) {
          pawsUi.syncPlatformChanges();
        }
      } catch (err) {
        hilog.error(LOG_DOMAIN, LOG_TAG, 'platform subscription failed %{public}s', describeError(err));
        this.platformSubscriptionActive = false;
      }
    }
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

  private async destroyVpnConnection(): Promise<void> {
    const connection = this.vpnConnection;
    if (!connection) {
      return;
    }
    try {
      await connection.destroy();
      pawsUi.setPlatformNetworkProtected(false);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'destroy vpn success');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'destroy vpn failed %{public}s', describeError(err));
    }
  }
}
