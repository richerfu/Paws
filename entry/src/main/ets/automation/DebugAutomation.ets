import { Want } from '@kit.AbilityKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import pawsUi from 'libpaws_ui.so';
import { describeError } from '../common/Errors';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsAutomation';

interface PawsAutomationParams {
  pawsProfileContent?: string;
  pawsProfileContentBase64?: string;
  pawsProfileContentEscaped?: string;
  pawsProfileUrl?: string;
  pawsProfileName?: string;
  pawsAutoStartVpn?: string | boolean;
  pawsDelayProxy?: string;
  pawsDelayUrl?: string;
  pawsDelayTimeoutMs?: string | number;
  pawsExpectDelayFailure?: string | boolean;
  pawsEchoProxy?: string;
  pawsEchoUrl?: string;
  pawsEchoPayload?: string;
  pawsEchoTimeoutMs?: string | number;
  pawsExpectEchoFailure?: string | boolean;
}

interface PawsSnapshot {
  vpnOptions?: Record<string, Object>;
}

/**
 * Debug/smoke automation driven by Ability Want parameters. Runs profile
 * import, optional VPN start, proxy delay and echo checks. Kept out of the
 * Ability entry so the entry only assembles plugins and forwards events.
 */
export function runDebugAutomation(
  want: Want,
  startVpn: (optionsJson: string) => Promise<void>,
): Promise<void> {
  return run(want, startVpn).catch((err: Error) => {
    hilog.error(LOG_DOMAIN, LOG_TAG, 'debug automation failed %{public}s', describeError(err));
    throw new Error(describeError(err) || 'debug automation failed');
  });
}

async function run(
  want: Want,
  startVpn: (optionsJson: string) => Promise<void>,
): Promise<void> {
  const profileContent = automationProfileContent(want);
  const profileUrl = wantString(want, 'pawsProfileUrl');
  const profileName = wantString(want, 'pawsProfileName') || 'Paws Smoke Profile';
  const delayProxy = wantString(want, 'pawsDelayProxy');
  const echoProxy = wantString(want, 'pawsEchoProxy');
  if (
    profileContent.length === 0 &&
    profileUrl.length === 0 &&
    !wantBool(want, 'pawsAutoStartVpn') &&
    delayProxy.length === 0 &&
    echoProxy.length === 0
  ) {
    return;
  }

  let profileId = '';
  if (profileContent.length > 0) {
    profileId = await pawsUi.importProfileFromContent(profileName, 'harmony-smoke', profileContent);
  } else if (profileUrl.length > 0) {
    profileId = await pawsUi.importProfileFromUrl(profileUrl, profileName);
  }

  if (profileId.length > 0) {
    await pawsUi.activateProfile(profileId);
    await pawsUi.reloadConfig(profileId);
    hilog.info(LOG_DOMAIN, LOG_TAG, 'debug automation profile ready %{public}s', profileId);
  }

  if (wantBool(want, 'pawsAutoStartVpn')) {
    hilog.info(LOG_DOMAIN, LOG_TAG, 'debug automation requested VPN start');
    startVpn(activeVpnOptionsJson()).catch((err: Error) => {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'debug automation VPN start failed %{public}s', err.message);
    });
  }

  await runDebugDelay(want);
  await runDebugEcho(want);
}

function activeVpnOptionsJson(): string {
  try {
    const snapshot = JSON.parse(pawsUi.querySnapshot()) as PawsSnapshot;
    if (snapshot.vpnOptions) {
      return JSON.stringify(snapshot.vpnOptions);
    }
  } catch (err) {
    hilog.error(LOG_DOMAIN, LOG_TAG, 'read active VPN options failed %{public}s', describeError(err));
  }
  return pawsUi.defaultVpnOptions();
}

async function runDebugDelay(want: Want): Promise<void> {
  const proxy = wantString(want, 'pawsDelayProxy');
  if (proxy.length === 0) {
    return;
  }
  const url = wantString(want, 'pawsDelayUrl');
  const timeoutMs = wantNumber(want, 'pawsDelayTimeoutMs') || 5000;
  const expectFailure = wantBool(want, 'pawsExpectDelayFailure');
  try {
    const delay = await pawsUi.testProxyDelay(proxy, url.length > 0 ? url : undefined, timeoutMs);
    if (expectFailure) {
      throw new Error(`delay unexpectedly succeeded in ${delay} ms`);
    }
    hilog.info(LOG_DOMAIN, LOG_TAG, 'debug automation delay result %{public}s %{public}d', proxy, delay);
  } catch (err) {
    if (expectFailure) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'debug automation delay failed as expected %{public}s',
        describeError(err));
      return;
    }
    hilog.error(LOG_DOMAIN, LOG_TAG, 'debug automation delay failed %{public}s', describeError(err));
    throw new Error(describeError(err) || 'debug automation delay failed');
  }
}

async function runDebugEcho(want: Want): Promise<void> {
  const proxy = wantString(want, 'pawsEchoProxy');
  if (proxy.length === 0) {
    return;
  }
  const url = wantString(want, 'pawsEchoUrl');
  if (url.length === 0) {
    throw new Error('debug echo URL is required');
  }
  const payload = wantString(want, 'pawsEchoPayload') || 'paws-echo-payload';
  const timeoutMs = wantNumber(want, 'pawsEchoTimeoutMs') || 5000;
  const expectFailure = wantBool(want, 'pawsExpectEchoFailure');
  try {
    const echoed = await pawsUi.testProxyEcho(proxy, url, payload, timeoutMs);
    if (expectFailure) {
      throw new Error(`echo unexpectedly succeeded: ${echoed}`);
    }
    if (echoed !== payload) {
      throw new Error('echo payload mismatch');
    }
    hilog.info(LOG_DOMAIN, LOG_TAG, 'debug automation echo result %{public}s %{public}d',
      proxy, echoed.length);
  } catch (err) {
    if (expectFailure) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'debug automation echo failed as expected %{public}s',
        describeError(err));
      return;
    }
    hilog.error(LOG_DOMAIN, LOG_TAG, 'debug automation echo failed %{public}s', describeError(err));
    throw new Error(describeError(err) || 'debug automation echo failed');
  }
}

function automationParam(want: Want, key: keyof PawsAutomationParams): string | number | boolean | undefined {
  const params = want.parameters as PawsAutomationParams | undefined;
  if (!params) {
    return undefined;
  }
  switch (key) {
    case 'pawsProfileContent':
      return params.pawsProfileContent;
    case 'pawsProfileContentBase64':
      return params.pawsProfileContentBase64;
    case 'pawsProfileContentEscaped':
      return params.pawsProfileContentEscaped;
    case 'pawsProfileUrl':
      return params.pawsProfileUrl;
    case 'pawsProfileName':
      return params.pawsProfileName;
    case 'pawsAutoStartVpn':
      return params.pawsAutoStartVpn;
    case 'pawsDelayProxy':
      return params.pawsDelayProxy;
    case 'pawsDelayUrl':
      return params.pawsDelayUrl;
    case 'pawsDelayTimeoutMs':
      return params.pawsDelayTimeoutMs;
    case 'pawsExpectDelayFailure':
      return params.pawsExpectDelayFailure;
    case 'pawsEchoProxy':
      return params.pawsEchoProxy;
    case 'pawsEchoUrl':
      return params.pawsEchoUrl;
    case 'pawsEchoPayload':
      return params.pawsEchoPayload;
    case 'pawsEchoTimeoutMs':
      return params.pawsEchoTimeoutMs;
    case 'pawsExpectEchoFailure':
      return params.pawsExpectEchoFailure;
    default:
      return undefined;
  }
}

function wantString(want: Want, key: keyof PawsAutomationParams): string {
  const value = automationParam(want, key);
  return typeof value === 'string' ? value : '';
}

function wantBool(want: Want, key: keyof PawsAutomationParams): boolean {
  const value = automationParam(want, key);
  return value === true || value === 'true' || value === '1' || value === 'yes';
}

function wantNumber(want: Want, key: keyof PawsAutomationParams): number {
  const value = automationParam(want, key);
  if (typeof value === 'number') {
    return value;
  }
  if (typeof value === 'string' && value.length > 0) {
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) ? parsed : 0;
  }
  return 0;
}

function automationProfileContent(want: Want): string {
  const raw = wantString(want, 'pawsProfileContent');
  if (raw.length > 0) {
    return raw;
  }
  const base64 = wantString(want, 'pawsProfileContentBase64');
  if (base64.length > 0) {
    return decodeBase64Utf8(base64);
  }
  const escaped = wantString(want, 'pawsProfileContentEscaped');
  return escaped.length > 0 ? escaped.replace(/\\n/g, '\n') : '';
}

function decodeBase64Utf8(value: string): string {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  const normalized = value.replace(/-/g, '+').replace(/_/g, '/').replace(/\s/g, '');
  const bytes: number[] = [];
  let buffer = 0;
  let bits = 0;
  for (let index = 0; index < normalized.length; index += 1) {
    const char = normalized.charAt(index);
    if (char === '=') {
      break;
    }
    const digit = alphabet.indexOf(char);
    if (digit < 0) {
      continue;
    }
    buffer = (buffer << 6) | digit;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      bytes.push((buffer >> bits) & 0xff);
    }
  }
  return decodeUtf8(bytes);
}

function decodeUtf8(bytes: number[]): string {
  let result = '';
  for (let index = 0; index < bytes.length;) {
    const first = bytes[index];
    if (first < 0x80) {
      result += String.fromCharCode(first);
      index += 1;
    } else if ((first & 0xe0) === 0xc0 && index + 1 < bytes.length) {
      result += String.fromCharCode(((first & 0x1f) << 6) | (bytes[index + 1] & 0x3f));
      index += 2;
    } else if ((first & 0xf0) === 0xe0 && index + 2 < bytes.length) {
      result += String.fromCharCode(
        ((first & 0x0f) << 12) | ((bytes[index + 1] & 0x3f) << 6) | (bytes[index + 2] & 0x3f),
      );
      index += 3;
    } else if ((first & 0xf8) === 0xf0 && index + 3 < bytes.length) {
      const codePoint =
        ((first & 0x07) << 18) |
        ((bytes[index + 1] & 0x3f) << 12) |
        ((bytes[index + 2] & 0x3f) << 6) |
        (bytes[index + 3] & 0x3f);
      const offset = codePoint - 0x10000;
      result += String.fromCharCode(0xd800 + (offset >> 10), 0xdc00 + (offset & 0x3ff));
      index += 4;
    } else {
      result += '\uFFFD';
      index += 1;
    }
  }
  return result;
}
