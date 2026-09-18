/// <reference path="../types/vpnExtensionDebug.d.ts" />

import { updateVpnAuthorizedState } from '@ohos.net.vpnExtension';
import { hilog } from '@kit.PerformanceAnalysisKit';
import BuildProfile from 'BuildProfile';
import { describeError } from '../common/Errors';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsVpn';

export function isVpnEmulatorCompatibilityEnabled(): boolean {
  return BuildProfile.DEBUG;
}

/** DevEco emulator images can lack the system VPN authorization dialog. */
export function authorizeVpnForEmulatorDebug(): void {
  if (!isVpnEmulatorCompatibilityEnabled()) {
    return;
  }
  try {
    const accepted: boolean = updateVpnAuthorizedState(BuildProfile.BUNDLE_NAME);
    hilog.info(LOG_DOMAIN, LOG_TAG,
      'debug updateVpnAuthorizedState returned %{public}s; verify system ret is 0',
      String(accepted));
  } catch (err) {
    hilog.warn(LOG_DOMAIN, LOG_TAG,
      'debug updateVpnAuthorizedState failed %{public}s', describeError(err));
  }
}

export function canRecoverRejectedEmulatorStop(
  recoverUnattachedDebugOwner: boolean,
  sessionId: string,
  cooperativeStopRequested: boolean,
): boolean {
  return isVpnEmulatorCompatibilityEnabled() && recoverUnattachedDebugOwner &&
    sessionId.length > 0 && !cooperativeStopRequested;
}
