import { AbilityConstant, Configuration, ConfigurationConstant, Want } from '@kit.AbilityKit';
import { pasteboard } from '@kit.BasicServicesKit';
import { fileIo, picker } from '@kit.CoreFileKit';
import { i18n } from '@kit.LocalizationKit';
import { vpnExtension } from '@kit.NetworkKit';
import { notificationManager } from '@kit.NotificationKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { scanBarcode, scanCore } from '@kit.ScanKit';
import { NativeAbility } from '@ohos-rs/ability';
import hmetaUi from 'libhmeta_ui.so';
import window from '@ohos.window';
import {
  buildVpnWant,
  HMetaPlatformSharedMemoryFds,
  setHMetaBundleName,
} from '../vpnability/VpnConfig';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'HMetaEntry';
const SCAN_CANCELLED_CODE: number = 1000500002;
const VPN_EXTENSION_ATTACH_GRACE_MS: number = 350;
const VPN_EXTENSION_REDISPATCH_ATTACH_TIMEOUT_MS: number = 5000;

interface HMetaAutomationParams {
  hmetaProfileContent?: string;
  hmetaProfileContentBase64?: string;
  hmetaProfileContentEscaped?: string;
  hmetaProfileUrl?: string;
  hmetaProfileName?: string;
  hmetaAutoStartVpn?: string | boolean;
  hmetaDelayProxy?: string;
  hmetaDelayUrl?: string;
  hmetaDelayTimeoutMs?: string | number;
  hmetaExpectDelayFailure?: string | boolean;
  hmetaEchoProxy?: string;
  hmetaEchoUrl?: string;
  hmetaEchoPayload?: string;
  hmetaEchoTimeoutMs?: string | number;
  hmetaExpectEchoFailure?: string | boolean;
}

interface HMetaSnapshot {
  vpnOptions?: Record<string, Object>;
}

interface HMetaTextExportRequest {
  suggestedName?: string;
  content?: string;
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

export default class EntryAbility extends NativeAbility {
  public moduleName: string | string[] = 'hmeta_ui';
  public defaultPage: boolean = false;
  private vpnStartRequest: Promise<void> | undefined = undefined;
  private activeVpnStartAttemptId: string = '';
  private platformSharedMemory?: HMetaPlatformSharedMemoryFds;

  private async pickProfileFileFromSystem(): Promise<Array<string>> {
    try {
      const documentPicker = new picker.DocumentViewPicker();
      const options = new picker.DocumentSelectOptions();
      options.maxSelectNumber = 1;
      options.fileSuffixFilters = ['YAML|.yaml,.yml', 'Text|.txt', 'All|.*'];
      const result = await documentPicker.select(options);
      if (!result || result.length === 0) {
        return [];
      }
      return result;
    } catch (_err) {
      return [];
    }
  }

  private async scanSubscriptionFromSystem(): Promise<string> {
    const options: scanBarcode.ScanOptions = {
      scanTypes: [scanCore.ScanType.QR_CODE],
      enableMultiMode: false,
      enableAlbum: true,
    };
    try {
      const result: scanBarcode.ScanResult =
        await scanBarcode.startScanForResult(this.context, options);
      return result.originalValue.trim();
    } catch (err) {
      const error = err as HMetaErrorLike;
      if (error.code === SCAN_CANCELLED_CODE) {
        return '';
      }
      throw new Error(describeError(err) || 'scan subscription failed');
    }
  }

  private async exportProfileToSystem(requestJson: string): Promise<void> {
    await this.exportTextToSystem(
      requestJson,
      'profile',
      '.yaml',
      ['.yaml', '.yml'],
      ['YAML|.yaml,.yml'],
      'profile YAML',
    );
  }

  private async exportLogToSystem(requestJson: string): Promise<void> {
    await this.exportTextToSystem(
      requestJson,
      'paws-log',
      '.log',
      ['.log'],
      ['Log|.log', 'Text|.txt'],
      'log',
    );
  }

  private async exportTextToSystem(
    requestJson: string,
    fallbackName: string,
    extension: string,
    acceptedExtensions: Array<string>,
    suffixChoices: Array<string>,
    label: string,
  ): Promise<void> {
    let request: HMetaTextExportRequest;
    try {
      request = JSON.parse(requestJson) as HMetaTextExportRequest;
    } catch (err) {
      throw new Error(`invalid ${label} export request: ${describeError(err)}`);
    }
    const content = request.content ?? '';
    if (content.length === 0) {
      throw new Error(`${label} export content is empty`);
    }
    const baseName = (request.suggestedName ?? fallbackName)
      .replace(/[\\/:*?"<>|]/g, '_')
      .trim() || fallbackName;
    const hasExtension = acceptedExtensions.some((value: string) => baseName.endsWith(value));
    const fileName = hasExtension ? baseName : `${baseName}${extension}`;
    const documentPicker = new picker.DocumentViewPicker();
    const options = new picker.DocumentSaveOptions();
    options.newFileNames = [fileName];
    options.fileSuffixChoices = suffixChoices;
    const result = await documentPicker.save(options);
    if (!result || result.length === 0 || result[0].length === 0) {
      throw new Error(`${label} export cancelled`);
    }
    let file: fileIo.File | undefined = undefined;
    try {
      file = fileIo.openSync(
        result[0],
        fileIo.OpenMode.WRITE_ONLY | fileIo.OpenMode.CREATE | fileIo.OpenMode.TRUNC,
      );
      fileIo.writeSync(file.fd, content);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'exported %{public}s', label);
    } catch (err) {
      throw new Error(describeError(err) || `write exported ${label} failed`);
    } finally {
      if (file) {
        try {
          fileIo.closeSync(file);
        } catch (err) {
          hilog.error(LOG_DOMAIN, LOG_TAG, 'close exported file failed %{public}s', describeError(err));
        }
      }
    }
  }

  private async requestStartVpn(optionsJson: string): Promise<void> {
    if (this.vpnStartRequest) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'reusing pending VPN start request');
      return this.vpnStartRequest;
    }
    const request = this.performStartVpn(optionsJson);
    this.vpnStartRequest = request;
    try {
      await request;
    } finally {
      if (this.vpnStartRequest === request) {
        this.vpnStartRequest = undefined;
      }
    }
  }

  private dispatchVpnStart(optionsJson: string, attemptId: string): void {
    this.runVpnStartDispatch(optionsJson, attemptId).catch((err: Error) => {
      const message = describeError(err) || 'system rejected VPN start';
      try {
        hmetaUi.failUnattachedPlatformVpnStart(attemptId, message);
      } catch (innerErr) {
        hilog.error(
          LOG_DOMAIN,
          LOG_TAG,
          'publish system VPN rejection failed %{public}s',
          describeError(innerErr),
        );
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

  private async runVpnStartDispatch(optionsJson: string, attemptId: string): Promise<void> {
    await this.invokeVpnExtensionStart(optionsJson, attemptId);
    hilog.info(LOG_DOMAIN, LOG_TAG, 'system acknowledged VPN start attempt %{public}s', attemptId);

    if (await hmetaUi.awaitPlatformVpnStartAttachment(attemptId, VPN_EXTENSION_ATTACH_GRACE_MS)) {
      return;
    }

    // The first-authorization dialog on some HarmonyOS builds starts the VPN
    // Extension with a new Want containing only bundleName/abilityName. Once
    // authorization succeeds, send the original payload again so the running
    // Extension receives the ashmem descriptors and attempt id via onRequest.
    hilog.warn(
      LOG_DOMAIN,
      LOG_TAG,
      'VPN Extension did not receive the authorized Want; redispatching attempt %{public}s',
      attemptId,
    );
    await this.invokeVpnExtensionStart(optionsJson, attemptId);
    hilog.info(LOG_DOMAIN, LOG_TAG, 'redispatched VPN start attempt %{public}s', attemptId);

    if (!(await hmetaUi.awaitPlatformVpnStartAttachment(
      attemptId,
      VPN_EXTENSION_REDISPATCH_ATTACH_TIMEOUT_MS,
    ))) {
      throw new Error('VPN Extension did not receive the shared-memory descriptors after authorization');
    }
  }

  private async performStartVpn(optionsJson: string): Promise<void> {
    let attemptId: string = '';
    try {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'request VPN start received');
      attemptId = hmetaUi.beginPlatformVpnStart();
      this.activeVpnStartAttemptId = attemptId;

      this.dispatchVpnStart(optionsJson, attemptId);

      const outcome = await hmetaUi.awaitPlatformVpnStart(attemptId);
      if (outcome !== 'connected') {
        throw new Error(`unexpected VPN start outcome: ${outcome}`);
      }
      this.ensureSpeedNotificationPermission().catch((err: Error) => {
        hilog.warn(LOG_DOMAIN, LOG_TAG, 'deferred notification permission failed %{public}s', describeError(err));
      });
      hilog.info(LOG_DOMAIN, LOG_TAG, 'VPN start completed attempt %{public}s', attemptId);
    } catch (err) {
      const message = describeError(err) || 'request VPN start failed';
      if (attemptId.length > 0) {
        try {
          hmetaUi.failPlatformVpnStart(attemptId, message);
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

  private async ensureSpeedNotificationPermission(): Promise<void> {
    try {
      if (await notificationManager.isNotificationEnabled()) {
        return;
      }
      await notificationManager.requestEnableNotification(this.context);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'notification permission enabled');
    } catch (err) {
      // Notification permission must never prevent the user from starting VPN.
      hilog.warn(LOG_DOMAIN, LOG_TAG, 'notification permission unavailable %{public}s', describeError(err));
    }
  }

  private async requestStopVpn(): Promise<void> {
    try {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'request VPN stop received');
      if (this.activeVpnStartAttemptId.length > 0) {
        hmetaUi.cancelPlatformVpnStart(this.activeVpnStartAttemptId);
      }
      await vpnExtension.stopVpnExtensionAbility(buildVpnWant());
      hmetaUi.setPlatformVpnRunning(false);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'requested VPN stop');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'request VPN stop failed %{public}s', describeError(err));
      throw new Error(describeError(err) || 'request VPN stop failed');
    }
  }

  private async openExternalUrl(url: string): Promise<void> {
    const webUrl = url.startsWith('https://') || url.startsWith('http://');
    const clashInstallUrl = url.startsWith('clash://install-config?url=');
    if (!webUrl && !clashInstallUrl) {
      throw new Error('unsupported external URL');
    }
    try {
      await this.context.startAbility({
        action: 'ohos.want.action.viewData',
        uri: url,
      });
      hilog.info(LOG_DOMAIN, LOG_TAG, 'opened external URL');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'open external URL failed %{public}s', describeError(err));
      throw new Error(describeError(err) || 'open external URL failed');
    }
  }

  private async copyText(text: string): Promise<void> {
    try {
      const data = pasteboard.createData(pasteboard.MIMETYPE_TEXT_PLAIN, text);
      await pasteboard.getSystemPasteboard().setData(data);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'copied text to system pasteboard');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'copy text failed %{public}s', describeError(err));
      throw new Error(describeError(err) || 'copy text failed');
    }
  }

  private automationParam(want: Want, key: keyof HMetaAutomationParams): string | number | boolean | undefined {
    const params = want.parameters as HMetaAutomationParams | undefined;
    if (!params) {
      return undefined;
    }
    switch (key) {
      case 'hmetaProfileContent':
        return params.hmetaProfileContent;
      case 'hmetaProfileContentBase64':
        return params.hmetaProfileContentBase64;
      case 'hmetaProfileContentEscaped':
        return params.hmetaProfileContentEscaped;
      case 'hmetaProfileUrl':
        return params.hmetaProfileUrl;
      case 'hmetaProfileName':
        return params.hmetaProfileName;
      case 'hmetaAutoStartVpn':
        return params.hmetaAutoStartVpn;
      case 'hmetaDelayProxy':
        return params.hmetaDelayProxy;
      case 'hmetaDelayUrl':
        return params.hmetaDelayUrl;
      case 'hmetaDelayTimeoutMs':
        return params.hmetaDelayTimeoutMs;
      case 'hmetaExpectDelayFailure':
        return params.hmetaExpectDelayFailure;
      case 'hmetaEchoProxy':
        return params.hmetaEchoProxy;
      case 'hmetaEchoUrl':
        return params.hmetaEchoUrl;
      case 'hmetaEchoPayload':
        return params.hmetaEchoPayload;
      case 'hmetaEchoTimeoutMs':
        return params.hmetaEchoTimeoutMs;
      case 'hmetaExpectEchoFailure':
        return params.hmetaExpectEchoFailure;
      default:
        return undefined;
    }
  }

  private configureNativeHome(): void {
    try {
      hmetaUi.configureAppHome(`${this.context.filesDir}/paws`);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configured native home');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'configure native home failed %{public}s', describeError(err));
    }
  }

  private initializePlatformSharedMemory(): void {
    const fields = hmetaUi.initializePlatformSharedMemory().split(',');
    const ashmemFd = Number(fields[0]);
    const notificationFd = Number(fields[1]);
    if (fields.length !== 2 || !Number.isInteger(ashmemFd) || ashmemFd < 0 ||
      !Number.isInteger(notificationFd) || notificationFd < 0) {
      throw new Error('native platform shared memory returned invalid file descriptors');
    }
    this.platformSharedMemory = { ashmemFd, notificationFd };
    hilog.info(LOG_DOMAIN, LOG_TAG, 'initialized platform shared memory');
  }

  private configureNativeLocale(language?: string): void {
    try {
      let locale = language ?? '';
      if (locale.length === 0) {
        locale = i18n.System.getSystemLanguage();
      }
      if (locale.length === 0) {
        return;
      }
      (hmetaUi as ESObject).configureUiLocale(locale);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configured native locale %{public}s', locale);
    } catch (err) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configure native locale skipped %{public}s', describeError(err));
    }
  }

  private configureNativeAppearance(colorMode?: ConfigurationConstant.ColorMode): void {
    try {
      const mode = colorMode ?? this.context.config.colorMode ?? ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET;
      (hmetaUi as ESObject).configureSystemColorMode(mode);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configured native color mode %{public}d', mode);
    } catch (err) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configure native color mode skipped %{public}s', describeError(err));
    }
  }

  private setAppColorMode(colorMode: number): void {
    let mode = ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET;
    if (colorMode === ConfigurationConstant.ColorMode.COLOR_MODE_DARK) {
      mode = ConfigurationConstant.ColorMode.COLOR_MODE_DARK;
    } else if (colorMode === ConfigurationConstant.ColorMode.COLOR_MODE_LIGHT) {
      mode = ConfigurationConstant.ColorMode.COLOR_MODE_LIGHT;
    }
    try {
      this.context.setColorMode(mode);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'applied app color mode %{public}d', mode);
    } catch (err) {
      const message = describeError(err);
      hilog.error(LOG_DOMAIN, LOG_TAG, 'apply app color mode failed %{public}s', message);
      throw new Error(message);
    }
  }

  private seedGeodataFromRawfiles(): void {
    try {
      const seeded = hmetaUi.seedGeodataFromRawfiles(this.context.resourceManager);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'seeded %{public}d geodata rawfiles', seeded);
    } catch (err) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'geodata rawfile seed skipped %{public}s', describeError(err));
    }
  }

  private wantString(want: Want, key: keyof HMetaAutomationParams): string {
    const value = this.automationParam(want, key);
    return typeof value === 'string' ? value : '';
  }

  private wantBool(want: Want, key: keyof HMetaAutomationParams): boolean {
    const value = this.automationParam(want, key);
    return value === true || value === 'true' || value === '1' || value === 'yes';
  }

  private wantNumber(want: Want, key: keyof HMetaAutomationParams): number {
    const value = this.automationParam(want, key);
    if (typeof value === 'number') {
      return value;
    }
    if (typeof value === 'string' && value.length > 0) {
      const parsed = Number.parseInt(value, 10);
      return Number.isFinite(parsed) ? parsed : 0;
    }
    return 0;
  }

  private automationProfileContent(want: Want): string {
    const raw = this.wantString(want, 'hmetaProfileContent');
    if (raw.length > 0) {
      return raw;
    }
    const base64 = this.wantString(want, 'hmetaProfileContentBase64');
    if (base64.length > 0) {
      return this.decodeBase64Utf8(base64);
    }
    const escaped = this.wantString(want, 'hmetaProfileContentEscaped');
    return escaped.length > 0 ? escaped.replace(/\\n/g, '\n') : '';
  }

  private decodeBase64Utf8(value: string): string {
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
    return this.decodeUtf8(bytes);
  }

  private decodeUtf8(bytes: number[]): string {
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

  private activeVpnOptionsJson(): string {
    try {
      const snapshot = JSON.parse(hmetaUi.querySnapshot()) as HMetaSnapshot;
      if (snapshot.vpnOptions) {
        return JSON.stringify(snapshot.vpnOptions);
      }
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'read active VPN options failed %{public}s', describeError(err));
    }
    return hmetaUi.defaultVpnOptions();
  }

  private async runDebugDelay(want: Want): Promise<void> {
    const proxy = this.wantString(want, 'hmetaDelayProxy');
    if (proxy.length === 0) {
      return;
    }
    const url = this.wantString(want, 'hmetaDelayUrl');
    const timeoutMs = this.wantNumber(want, 'hmetaDelayTimeoutMs') || 5000;
    const expectFailure = this.wantBool(want, 'hmetaExpectDelayFailure');
    try {
      const delay = await hmetaUi.testProxyDelay(proxy, url.length > 0 ? url : undefined, timeoutMs);
      if (expectFailure) {
        throw new Error(`delay unexpectedly succeeded in ${delay} ms`);
      }
      hilog.info(LOG_DOMAIN, LOG_TAG, 'debug automation delay result %{public}s %{public}d', proxy, delay);
    } catch (err) {
      if (expectFailure) {
        hilog.info(LOG_DOMAIN, LOG_TAG, 'debug automation delay failed as expected %{public}s', describeError(err));
        return;
      }
      hilog.error(LOG_DOMAIN, LOG_TAG, 'debug automation delay failed %{public}s', describeError(err));
      throw new Error(describeError(err) || 'debug automation delay failed');
    }
  }

  private async runDebugEcho(want: Want): Promise<void> {
    const proxy = this.wantString(want, 'hmetaEchoProxy');
    if (proxy.length === 0) {
      return;
    }
    const url = this.wantString(want, 'hmetaEchoUrl');
    if (url.length === 0) {
      throw new Error('debug echo URL is required');
    }
    const payload = this.wantString(want, 'hmetaEchoPayload') || 'hmeta-echo-payload';
    const timeoutMs = this.wantNumber(want, 'hmetaEchoTimeoutMs') || 5000;
    const expectFailure = this.wantBool(want, 'hmetaExpectEchoFailure');
    try {
      const echoed = await hmetaUi.testProxyEcho(proxy, url, payload, timeoutMs);
      if (expectFailure) {
        throw new Error(`echo unexpectedly succeeded: ${echoed}`);
      }
      if (echoed !== payload) {
        throw new Error('echo payload mismatch');
      }
      hilog.info(LOG_DOMAIN, LOG_TAG, 'debug automation echo result %{public}s %{public}d', proxy, echoed.length);
    } catch (err) {
      if (expectFailure) {
        hilog.info(LOG_DOMAIN, LOG_TAG, 'debug automation echo failed as expected %{public}s', describeError(err));
        return;
      }
      hilog.error(LOG_DOMAIN, LOG_TAG, 'debug automation echo failed %{public}s', describeError(err));
      throw new Error(describeError(err) || 'debug automation echo failed');
    }
  }

  private async runDebugAutomation(want: Want): Promise<void> {
    const profileContent = this.automationProfileContent(want);
    const profileUrl = this.wantString(want, 'hmetaProfileUrl');
    const profileName = this.wantString(want, 'hmetaProfileName') || 'Paws Smoke Profile';
    const delayProxy = this.wantString(want, 'hmetaDelayProxy');
    const echoProxy = this.wantString(want, 'hmetaEchoProxy');
    if (
      profileContent.length === 0 &&
      profileUrl.length === 0 &&
      !this.wantBool(want, 'hmetaAutoStartVpn') &&
      delayProxy.length === 0 &&
      echoProxy.length === 0
    ) {
      return;
    }

    try {
      let profileId = '';
      if (profileContent.length > 0) {
        profileId = await hmetaUi.importProfileFromContent(profileName, 'harmony-smoke', profileContent);
      } else if (profileUrl.length > 0) {
        profileId = await hmetaUi.importProfileFromUrl(profileUrl, profileName);
      }

      if (profileId.length > 0) {
        await hmetaUi.activateProfile(profileId);
        await hmetaUi.reloadConfig(profileId);
        hilog.info(LOG_DOMAIN, LOG_TAG, 'debug automation profile ready %{public}s', profileId);
      }

      if (this.wantBool(want, 'hmetaAutoStartVpn')) {
        hilog.info(LOG_DOMAIN, LOG_TAG, 'debug automation requested VPN start');
        this.requestStartVpn(this.activeVpnOptionsJson()).catch((err: Error) => {
          hilog.error(LOG_DOMAIN, LOG_TAG, 'debug automation VPN start failed %{public}s', err.message);
        });
      }

      await this.runDebugDelay(want);
      await this.runDebugEcho(want);
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'debug automation failed %{public}s', describeError(err));
      throw new Error(describeError(err) || 'debug automation failed');
    }
  }

  public async onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): Promise<void> {
    try {
      setHMetaBundleName(this.context.applicationInfo.name);
      this.configureNativeHome();
      this.initializePlatformSharedMemory();
      this.configureNativeLocale();
      this.configureNativeAppearance();
      this.seedGeodataFromRawfiles();
      hmetaUi.registerPlatformCallbacks({
        pickProfileFile: async () => await this.pickProfileFileFromSystem(),
        scanSubscription: async () => await this.scanSubscriptionFromSystem(),
        requestStartVpn: async (optionsJson: string) => await this.requestStartVpn(optionsJson),
        requestStopVpn: async () => await this.requestStopVpn(),
        openExternalUrl: async (url: string) => await this.openExternalUrl(url),
        copyText: async (text: string) => await this.copyText(text),
        exportProfile: async (requestJson: string) => await this.exportProfileToSystem(requestJson),
        exportLog: async (requestJson: string) => await this.exportLogToSystem(requestJson),
        setColorMode: async (colorMode: number) => this.setAppColorMode(colorMode),
      });
      hilog.info(LOG_DOMAIN, LOG_TAG, 'registered native platform callbacks');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'register callbacks failed %{public}s', describeError(err));
    }
    await super.onCreate(want, launchParam);
    this.runDebugAutomation(want).catch((err: Error) => {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'create Want automation failed %{public}s', err.message);
    });
  }

  public onNewWant(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    super.onNewWant(want, launchParam);
    this.runDebugAutomation(want).catch((err: Error) => {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'new Want automation failed %{public}s', err.message);
    });
  }

  public onConfigurationUpdate(newConfig: Configuration): void {
    this.configureNativeLocale(newConfig.language);
    this.configureNativeAppearance(newConfig.colorMode);
    super.onConfigurationUpdate(newConfig);
  }

  public async onDestroy() {
    if (this.activeVpnStartAttemptId.length > 0) {
      try {
        hmetaUi.cancelPlatformVpnStart(this.activeVpnStartAttemptId);
      } catch (err) {
        hilog.warn(LOG_DOMAIN, LOG_TAG, 'cancel active VPN start failed %{public}s', describeError(err));
      }
      this.activeVpnStartAttemptId = '';
    }
    super.onDestroy();
  }

  public async onWindowStageCreate(windowStage: window.WindowStage): Promise<void> {
    await super.onWindowStageCreate(windowStage);
    try {
      const win = await windowStage.getMainWindow();
      await win.setUIContent('pages/Index');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'set UI content failed %{public}s', JSON.stringify(err) ?? '');
    }
  }
}
