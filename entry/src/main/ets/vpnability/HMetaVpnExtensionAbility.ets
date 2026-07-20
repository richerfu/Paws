import { Want } from '@kit.AbilityKit';
import { vpnExtension, VpnExtensionAbility } from '@kit.NetworkKit';
import { notificationManager } from '@kit.NotificationKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import hmetaUi from 'libhmeta_ui.so';
import {
  blockedApplications,
  DEFAULT_OPTIONS_JSON,
  HMetaVpnConfig,
  parseOptions,
  perAppMode,
  readOptionsJson,
  setHMetaBundleName,
  trustedApplications,
} from './VpnConfig';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'HMetaVpn';
const VPN_SPEED_NOTIFICATION_ID: number = 1001;
const NOTIFICATION_RETRY_DELAY_MS: number = 30000;

interface HMetaTrafficSnapshot {
  downloadSpeed?: number;
  uploadSpeed?: number;
}

interface HMetaRuntimeSnapshot {
  traffic?: HMetaTrafficSnapshot;
}

interface HMetaErrorLike {
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
  const like = err as HMetaErrorLike;
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

export default class HMetaVpnExtensionAbility extends VpnExtensionAbility {
  private vpnConnection?: vpnExtension.VpnConnection;
  private tunFd: number = -1;
  private starting: boolean = false;
  private telemetryTimer: number = -1;
  private notificationPublishing: boolean = false;
  private notificationFailureLogged: boolean = false;
  private notificationRetryAt: number = 0;
  private notificationGeneration: number = 0;
  private lastNotificationText: string = '';

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
    hmetaUi.setPlatformVpnRunning(false);
    this.destroyVpn();
  }

  private configureNativeHome(): void {
    try {
      setHMetaBundleName(this.context.applicationInfo.name);
      hmetaUi.configureAppHome(`${this.context.filesDir}/paws`);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configured native home');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'configure native home failed %{public}s', describeError(err));
    }
  }

  private startFromWant(want: Want): void {
    if (this.starting || this.tunFd >= 0) {
      return;
    }
    this.starting = true;
    this.configureNativeHome();
    try {
      hmetaUi.setPlatformVpnStarting(true);
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'record VPN starting failed %{public}s', describeError(err));
    }
    const rawOptionsJson = want.parameters?.optionsJson;
    const optionsJson = typeof rawOptionsJson === 'string'
      ? readOptionsJson(rawOptionsJson)
      : DEFAULT_OPTIONS_JSON;
    const options = parseOptions(optionsJson);
    const trusted = trustedApplications(options);
    const blocked = blockedApplications(options);
    const config = new HMetaVpnConfig(options, trusted, blocked);
    hilog.info(
      LOG_DOMAIN,
      LOG_TAG,
      'applying per-app VPN mode %{public}s trusted %{public}d blocked %{public}d',
      perAppMode(options),
      trusted.length,
      blocked.length,
    );
    const startedAt = Date.now();

    try {
      this.vpnConnection = vpnExtension.createVpnConnection(this.context);
      const nativePrepare: Promise<string> = hmetaUi.prepareVpn()
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
              hmetaUi.setPlatformNetworkProtected(true);
            } catch (stateErr) {
              hilog.error(LOG_DOMAIN, LOG_TAG, 'record protect success failed %{public}s', describeError(stateErr));
            }
            hilog.info(LOG_DOMAIN, LOG_TAG, 'protected process network');
          } catch (err) {
            const message = describeError(err) || 'protect process network failed';
            try {
              hmetaUi.setPlatformNetworkProtected(false, message);
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
          await hmetaUi.startVpn(fd, optionsJson);
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
      hmetaUi.setPlatformVpnFailed(message);
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'record VPN failure failed %{public}s', describeError(err));
    }
  }

  private destroyVpn(): void {
    this.stopTelemetry();
    this.stopSpeedNotification();
    hmetaUi.stopVpn()
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
      hmetaUi.persistVpnTelemetry();
      if (publishNotification) {
        this.publishSpeedNotification();
      }
    } catch (err) {
      hilog.warn(LOG_DOMAIN, LOG_TAG, 'persist VPN telemetry failed %{public}s', describeError(err));
    }
  }

  private publishSpeedNotification(): void {
    if (this.notificationPublishing || Date.now() < this.notificationRetryAt) {
      return;
    }

    let snapshot: HMetaRuntimeSnapshot;
    try {
      snapshot = JSON.parse(hmetaUi.querySnapshot()) as HMetaRuntimeSnapshot;
    } catch (err) {
      hilog.warn(LOG_DOMAIN, LOG_TAG, 'read notification telemetry failed %{public}s', describeError(err));
      return;
    }
    const text = `↓ ${formatSpeed(snapshot.traffic?.downloadSpeed)}  ·  ↑ ${formatSpeed(snapshot.traffic?.uploadSpeed)}`;
    if (text === this.lastNotificationText) {
      return;
    }

    const generation = this.notificationGeneration;
    const request: notificationManager.NotificationRequest = {
      id: VPN_SPEED_NOTIFICATION_ID,
      notificationSlotType: notificationManager.SlotType.SERVICE_INFORMATION,
      isOngoing: true,
      isUnremovable: true,
      isAlertOnce: true,
      tapDismissed: false,
      showDeliveryTime: false,
      content: {
        notificationContentType: notificationManager.ContentType.NOTIFICATION_CONTENT_BASIC_TEXT,
        normal: {
          title: 'Paws · VPN',
          text,
          additionalText: '实时网速',
        },
      },
    };
    this.notificationPublishing = true;
    notificationManager.publish(request)
      .then(() => {
        if (generation !== this.notificationGeneration) {
          notificationManager.cancel(VPN_SPEED_NOTIFICATION_ID).catch(() => {});
          return;
        }
        this.lastNotificationText = text;
        this.notificationFailureLogged = false;
        this.notificationRetryAt = 0;
      })
      .catch((err: Error) => {
        this.notificationRetryAt = Date.now() + NOTIFICATION_RETRY_DELAY_MS;
        if (!this.notificationFailureLogged) {
          hilog.warn(LOG_DOMAIN, LOG_TAG, 'publish VPN speed notification failed %{public}s', describeError(err));
          this.notificationFailureLogged = true;
        }
      })
      .finally(() => {
        this.notificationPublishing = false;
      });
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
      hmetaUi.setPlatformNetworkProtected(false);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'destroy vpn success');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'destroy vpn failed %{public}s', describeError(err));
    }
  }
}
