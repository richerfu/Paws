import { common, wantAgent, WantAgent } from '@kit.AbilityKit';
import { backgroundTaskManager } from '@kit.BackgroundTasksKit';
import { notificationManager } from '@kit.NotificationKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import pawsUi from 'libpaws_ui.so';
import {
  currentPawsBundleName,
  PAWS_ENTRY_ABILITY,
  PAWS_MODULE_NAME,
} from './VpnConfig';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsBackground';
const BACKGROUND_MODE: string = 'dataTransfer';
const PROGRESS_UPDATE_INTERVAL_MS: number = 5 * 60 * 1000;
const PROGRESS_WINDOW_BYTES: number = 100 * 1024 * 1024;

interface PawsTrafficSnapshot {
  downloadBytes?: number;
  uploadBytes?: number;
  downloadSpeed?: number;
  uploadSpeed?: number;
}

interface PawsRuntimeSnapshot {
  traffic?: PawsTrafficSnapshot;
}

/**
 * Owns the public HarmonyOS continuous data-transfer task for an established
 * VPN session. The system task is requested by EntryAbility as required by the
 * stage model. Its notification is updated with real tunnel counters before
 * the ten-minute DATA_TRANSFER expiry window.
 */
export class VpnContinuousTask {
  private context?: common.UIAbilityContext;
  private taskNotification?: backgroundTaskManager.ContinuousTaskNotification;
  private startRequest?: Promise<void>;
  private progressTimer: number = -1;
  private stopping: boolean = false;
  private cancelHandler?: (info: backgroundTaskManager.ContinuousTaskCancelInfo) => void;

  private readonly cancellationListener =
    (info: backgroundTaskManager.ContinuousTaskCancelInfo): void => {
      const activeId = this.taskNotification?.continuousTaskId;
      if (activeId !== undefined && info.id !== activeId) {
        return;
      }
      hilog.warn(
        LOG_DOMAIN,
        LOG_TAG,
        'continuous VPN task cancelled id=%{public}d reason=%{public}d detail=%{public}s',
        info.id,
        info.reason,
        info.detailedReason === undefined ? '' : info.detailedReason.toString(),
      );
      this.clearLocalState();
      const handler = this.cancelHandler;
      if (!this.stopping && handler !== undefined) {
        handler(info);
      }
    };

  start(
    context: common.UIAbilityContext,
    onCancelled: (info: backgroundTaskManager.ContinuousTaskCancelInfo) => void,
  ): Promise<void> {
    this.context = context;
    this.cancelHandler = onCancelled;
    if (this.taskNotification !== undefined) {
      return Promise.resolve();
    }
    if (this.startRequest !== undefined) {
      return this.startRequest;
    }
    const request = this.startInternal(context);
    this.startRequest = request;
    request.then((): void => {
      if (this.startRequest === request) {
        this.startRequest = undefined;
      }
    }, (): void => {
      if (this.startRequest === request) {
        this.startRequest = undefined;
      }
    });
    return request;
  }

  async stop(): Promise<void> {
    const context = this.context;
    const wasRequested = this.taskNotification !== undefined || this.startRequest !== undefined;
    this.stopping = true;
    this.clearProgressTimer();
    try {
      if (this.startRequest !== undefined) {
        await this.startRequest.catch((): void => {});
      }
      if (context !== undefined && wasRequested) {
        await backgroundTaskManager.stopBackgroundRunning(context);
      }
      if (wasRequested) {
        hilog.info(LOG_DOMAIN, LOG_TAG, 'continuous VPN task stopped');
      }
    } finally {
      backgroundTaskManager.off('continuousTaskCancel', this.cancellationListener);
      this.taskNotification = undefined;
      this.cancelHandler = undefined;
      this.context = undefined;
      this.stopping = false;
    }
  }

  private async startInternal(context: common.UIAbilityContext): Promise<void> {
    const agent = await this.createWantAgent();
    backgroundTaskManager.off('continuousTaskCancel', this.cancellationListener);
    backgroundTaskManager.on('continuousTaskCancel', this.cancellationListener);
    try {
      const task = await backgroundTaskManager.startBackgroundRunning(
        context,
        [BACKGROUND_MODE],
        agent,
      );
      this.taskNotification = task;
      await this.publishProgress(task, agent);
      this.progressTimer = setInterval((): void => {
        const currentTask = this.taskNotification;
        if (currentTask !== undefined) {
          this.publishProgress(currentTask, agent).catch((err: Error): void => {
            hilog.warn(LOG_DOMAIN, LOG_TAG,
              'continuous VPN progress update failed %{public}s', String(err));
          });
        }
      }, PROGRESS_UPDATE_INTERVAL_MS);
      hilog.info(
        LOG_DOMAIN,
        LOG_TAG,
        'continuous VPN task started notification=%{public}d task=%{public}s mode=dataTransfer',
        task.notificationId,
        task.continuousTaskId === undefined ? '' : task.continuousTaskId.toString(),
      );
    } catch (err) {
      backgroundTaskManager.off('continuousTaskCancel', this.cancellationListener);
      if (this.taskNotification !== undefined) {
        await backgroundTaskManager.stopBackgroundRunning(context).catch((): void => {});
      }
      this.clearLocalState();
      throw new Error(String(err));
    }
  }

  private createWantAgent(): Promise<WantAgent> {
    return wantAgent.getWantAgent({
      wants: [{
        bundleName: currentPawsBundleName(),
        moduleName: PAWS_MODULE_NAME,
        abilityName: PAWS_ENTRY_ABILITY,
        parameters: { pawsContinuousTask: true },
      }],
      actionType: wantAgent.OperationType.START_ABILITY,
      requestCode: 17043,
      actionFlags: [wantAgent.WantAgentFlags.UPDATE_PRESENT_FLAG],
    });
  }

  private async publishProgress(
    task: backgroundTaskManager.ContinuousTaskNotification,
    agent: WantAgent,
  ): Promise<void> {
    const traffic = this.readTraffic();
    const total = Math.max(0, (traffic.downloadBytes ?? 0) + (traffic.uploadBytes ?? 0));
    const progress = Math.floor((total % PROGRESS_WINDOW_BYTES) * 100 / PROGRESS_WINDOW_BYTES);
    const text = `↓ ${formatSpeed(traffic.downloadSpeed)} · ↑ ${formatSpeed(traffic.uploadSpeed)}`;
    const totalText = `累计 ${formatBytes(total)}`;
    const request: notificationManager.NotificationRequest = {
      id: task.notificationId,
      notificationSlotType: notificationManager.SlotType.LIVE_VIEW,
      isOngoing: true,
      isAlertOnce: true,
      tapDismissed: false,
      wantAgent: agent,
      content: {
        notificationContentType:
          notificationManager.ContentType.NOTIFICATION_CONTENT_SYSTEM_LIVE_VIEW,
        systemLiveView: {
          typeCode: 8,
          title: 'TinyPaws · VPN 正在传输数据',
          text: `${text} · ${totalText}`,
          progress: {
            maxValue: 100,
            currentValue: progress,
            isPercentage: false,
          },
        },
      },
      template: {
        name: 'downloadTemplate',
        data: {
          title: 'TinyPaws · VPN 正在运行',
          fileName: `${text} · ${totalText}`,
          progressValue: progress,
        },
      },
    };
    await notificationManager.publish(request);
    hilog.info(
      LOG_DOMAIN,
      LOG_TAG,
      'continuous VPN progress notification=%{public}d bytes=%{public}d progress=%{public}d',
      task.notificationId,
      total,
      progress,
    );
  }

  private readTraffic(): PawsTrafficSnapshot {
    try {
      const snapshot = JSON.parse(pawsUi.queryVpnSpeed()) as PawsRuntimeSnapshot;
      return snapshot.traffic ?? {};
    } catch (err) {
      hilog.warn(LOG_DOMAIN, LOG_TAG,
        'continuous VPN task could not read traffic %{public}s', String(err));
      return {};
    }
  }

  private clearLocalState(): void {
    this.clearProgressTimer();
    this.taskNotification = undefined;
    this.context = undefined;
  }

  private clearProgressTimer(): void {
    if (this.progressTimer >= 0) {
      clearInterval(this.progressTimer);
      this.progressTimer = -1;
    }
  }
}

function formatSpeed(bytesPerSecond: number | undefined): string {
  return `${formatBytes(Math.max(0, bytesPerSecond ?? 0))}/s`;
}

function formatBytes(bytes: number): string {
  const value = Math.max(0, bytes);
  if (value < 1024) {
    return `${Math.round(value)} B`;
  }
  if (value < 1024 * 1024) {
    return `${(value / 1024).toFixed(value < 10 * 1024 ? 1 : 0)} KB`;
  }
  if (value < 1024 * 1024 * 1024) {
    return `${(value / (1024 * 1024)).toFixed(value < 10 * 1024 * 1024 ? 1 : 0)} MB`;
  }
  return `${(value / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

export const vpnContinuousTask: VpnContinuousTask = new VpnContinuousTask();
