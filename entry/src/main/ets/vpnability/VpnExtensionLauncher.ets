import { vpnExtension } from '@kit.NetworkKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import pawsUi from 'libpaws_ui.so';
import { describeError } from '../common/Errors';
import { buildVpnBootstrapWant, PawsPlatformSharedMemoryFds } from './VpnConfig';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsVpn';
const ACCEPTED_HANDOFF_WAIT_MS: number = 10000;

/** Start the Extension once, then transfer the existing IPC descriptors. */
export async function startVpnExtensionWithHandoff(
  optionsJson: string,
  attemptId: string,
  sharedMemory: PawsPlatformSharedMemoryFds,
): Promise<void> {
  const token = pawsUi.preparePlatformVpnHandoff(attemptId);
  try {
    hilog.info(LOG_DOMAIN, LOG_TAG,
      'dispatching descriptor-free system VPN bootstrap attempt %{public}s', attemptId);
    // On first authorization, QEMU can create the Extension while keeping the
    // system start Promise pending. The Extension must receive its FDs before
    // the Promise can be treated as a reliable completion signal.
    const systemStart: Promise<void> = vpnExtension.startVpnExtensionAbility(
      buildVpnBootstrapWant(attemptId, token),
    );
    let handoffFinished: boolean = false;
    let acceptedTimer: number = -1;
    const systemFailure: Promise<void> = new Promise<void>((_resolve, reject): void => {
      systemStart.then((): void => {
        hilog.info(LOG_DOMAIN, LOG_TAG,
          'system VPN bootstrap accepted attempt %{public}s', attemptId);
        if (!handoffFinished) {
          acceptedTimer = setTimeout((): void => {
            reject(new Error('system accepted VPN start but Extension did not receive handoff'));
          }, ACCEPTED_HANDOFF_WAIT_MS);
        }
      }, (err: Error): void => {
        if (handoffFinished) {
          hilog.warn(LOG_DOMAIN, LOG_TAG,
            'system VPN start rejected after handoff attempt %{public}s: %{public}s',
            attemptId, describeError(err));
        } else {
          reject(err);
        }
      });
    });
    hilog.info(LOG_DOMAIN, LOG_TAG,
      'sending VPN session FD handoff attempt %{public}s', attemptId);
    try {
      await Promise.race([
        pawsUi.sendPlatformVpnHandoff(
          sharedMemory.ashmemFd,
          sharedMemory.notificationFd,
          attemptId,
          optionsJson,
        ),
        systemFailure,
      ]);
    } finally {
      handoffFinished = true;
      if (acceptedTimer >= 0) {
        clearTimeout(acceptedTimer);
      }
    }
  } catch (err) {
    throw new Error(describeError(err) || 'start VPN Extension with session handoff failed');
  }
}
