import { AbilityConstant, Configuration, ConfigurationConstant, Want } from '@kit.AbilityKit';
import { i18n } from '@kit.LocalizationKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { LazyPlugin, NativeAbility } from '@ohos-rs/ability';
import { nativeShellPlugins } from '@nsh/ohos';
import { FilesPlugin } from '@ohos-rs/ability-plugin-files';
import { UrlPlugin } from '@ohos-rs/ability-plugin-url';
import { ResourcePlugin } from '@ohos-rs/ability-plugin-resource';
import { AccessibilityPlugin } from '@ohos-rs/ability-plugin-accessibility';
import { ArkuiPlugin } from '@ohos-rs/ability-plugin-arkui';
import { ClipboardPlugin } from '@ohos-rs/ability-plugin-clipboard';
import { WindowPlugin } from '@ohos-rs/ability-plugin-window';
import { WebviewPlugin } from '@ohos-rs/ability-plugin-webview';
import { VideoPlugin } from '@ohos-rs/ability-plugin-video';
import { initLogging } from 'libnsh_platform_ohos.so';
import window from '@ohos.window';
import pawsUi from 'libpaws_ui.so';
import { describeError } from '../common/Errors';
import { setPawsBundleName } from '../vpnability/VpnConfig';
import { runDebugAutomation } from '../automation/DebugAutomation';
import { VpnPlugin } from '../plugins/VpnPlugin';
import { installPawsHost, PawsHostServices } from '../components/PawsCoreGateway';
import { SystemExitRecorder } from '../vpnability/SystemExitRecorder';
import { WindowFrameMetrics } from '../diagnostics/WindowFrameMetrics';

import { getPhysicalNetwork } from '../vpnability/UnderlyingNetwork';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsEntry';

/**
 * Thin assembly point: loads the native module, installs the built-in
 * capability plugins (`ohos.files`, `ohos.url`) plus the application-owned
 * `paws.*` plugins, and forwards lifecycle events. All platform logic lives
 * in the plugins; debug automation lives in `DebugAutomation`.
 */
export default class EntryAbility extends NativeAbility {
  public moduleName: string = 'nsh_platform_ohos';
  public defaultPage: boolean = false;

  private resolveVpnPluginReady?: (plugin: VpnPlugin) => void;
  private vpnPluginReady: Promise<VpnPlugin> = new Promise((resolve): void => {
    this.resolveVpnPluginReady = resolve;
  });
  private vpnPlugin?: VpnPlugin;
  private hostServices?: PawsHostServices;
  private systemExitRecorder: SystemExitRecorder = new SystemExitRecorder();
  private windowFrameMetrics: WindowFrameMetrics = new WindowFrameMetrics();

  public bridgePlugins = [
    new LazyPlugin(() => new ResourcePlugin()),
    new LazyPlugin(() => new AccessibilityPlugin()),
    ...nativeShellPlugins(),
    new LazyPlugin(() => new ArkuiPlugin()),
    new LazyPlugin(() => new ClipboardPlugin({ metadataMimeType: 'application/x-paws-clipboard' })),
    new LazyPlugin(() => new WindowPlugin()),
    new LazyPlugin(() => new WebviewPlugin()),
    new LazyPlugin(() => new VideoPlugin()),
    new LazyPlugin(() => new FilesPlugin()),
    new LazyPlugin(() => new UrlPlugin()),
  ];

  private async requestStartVpn(optionsJson: string): Promise<void> {
    let timer: number = -1;
    try {
      const plugin = await Promise.race([
        this.vpnPluginReady,
        new Promise<VpnPlugin>((_resolve, reject): void => {
          timer = setTimeout((): void => reject(new Error('VPN plugin did not attach before debug startup deadline')), 30000);
        }),
      ]);
      await plugin.requestStartVpn(optionsJson);
    } finally {
      if (timer >= 0) { clearTimeout(timer); }
    }
  }

  private configureNativeHome(): void {
    pawsUi.configureAppHome(`${this.context.filesDir}/paws`);
    hilog.info(LOG_DOMAIN, LOG_TAG, 'configured native home');
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
      pawsUi.configureUiLocale(locale);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configured native locale %{public}s', locale);
    } catch (err) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configure native locale skipped %{public}s', describeError(err));
    }
  }

  private configureNativeAppearance(colorMode?: ConfigurationConstant.ColorMode): void {
    try {
      const mode = colorMode ?? this.context.config.colorMode ?? ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET;
      pawsUi.configureSystemColorMode(mode);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configured native color mode %{public}d', mode);
    } catch (err) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configure native color mode skipped %{public}s', describeError(err));
    }
  }

  private seedGeodataFromRawfiles(): void {
    try {
      const seeded = pawsUi.seedGeodataFromRawfiles(this.context.resourceManager);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'seeded %{public}d geodata rawfiles', seeded);
    } catch (err) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'geodata rawfile seed skipped %{public}s', describeError(err));
    }
  }

  private runOptionalDebugAutomation(want: Want): void {
    runDebugAutomation(
      want,
      this.context.applicationInfo.debug === true,
      (optionsJson: string) => this.requestStartVpn(optionsJson),
    )
      .catch((err: Error): void => {
        // Debug automation is never part of critical Ability/UI startup, but
        // its rejected promise must remain observable instead of becoming an
        // unhandled rejection or a false-positive smoke result.
        hilog.error(LOG_DOMAIN, LOG_TAG, 'debug automation terminated %{public}s', describeError(err));
      });
  }

  public onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    try {
      setPawsBundleName(this.context.applicationInfo.name);
      this.configureNativeHome();
      this.hostServices = installPawsHost(this.context);
      this.vpnPlugin = this.hostServices.vpn;
      this.resolveVpnPluginReady?.(this.vpnPlugin);
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'native initialization failed %{public}s', describeError(err));
      throw new Error(describeError(err) || 'critical native initialization failed');
    }
    // These enrich the initialized runtime but are independently optional;
    // each helper records its own precise platform failure.
    try { this.systemExitRecorder.start(this.context.filesDir, launchParam, false, this.context.applicationInfo.debug === true); }
    catch (_) { hilog.warn(LOG_DOMAIN, LOG_TAG, 'system exit recorder unavailable'); }
    this.configureNativeLocale();
    this.configureNativeAppearance();
    this.seedGeodataFromRawfiles();
    super.onCreate(want, launchParam);
    // NativeAbility attaches lazy plugins on its lifecycle queue. Never await
    // before super.onCreate, or race debug automation against that attachment.
    void this.enqueueLifecycleOperation('paws-network-ready', async (): Promise<void> => {
      // Network discovery must never hold up the WindowStage queue.
      getPhysicalNetwork().then((physical): void => {
        pawsUi.setPhysicalNetwork(physical.netId);
      }).catch((err: Error): void => {
        hilog.warn(LOG_DOMAIN, LOG_TAG, 'physical network initialization deferred %{public}s', describeError(err));
      });
      this.runOptionalDebugAutomation(want);
    });
  }

  public onNewWant(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    super.onNewWant(want, launchParam);
    this.systemExitRecorder.drain();
    this.vpnPlugin?.onAbilityForeground();
    void this.enqueueLifecycleOperation('paws-debug-new-want', async (): Promise<void> => {
      this.runOptionalDebugAutomation(want);
    });
  }

  public onForeground(): void {
    super.onForeground();
    this.systemExitRecorder.drain();
    this.vpnPlugin?.onAbilityForeground();
  }

  public async onDestroy(): Promise<void> {
    this.windowFrameMetrics.dispose();
    this.hostServices?.dispose();
    await super.onDestroy();
  }

  public onConfigurationUpdate(newConfig: Configuration): void {
    this.configureNativeLocale(newConfig.language);
    this.configureNativeAppearance(newConfig.colorMode);
    super.onConfigurationUpdate(newConfig);
  }

  protected async loadWindowStageContent(windowStage: window.WindowStage): Promise<void> {
    initLogging();
    const win = await windowStage.getMainWindow();
    this.hostServices?.attachMainWindow(win);
    try {
      await win.setWindowLayoutFullScreen(true);
    } catch (err) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'full-screen layout skipped %{public}s', describeError(err));
    }
    for (const type of [window.AvoidAreaType.TYPE_SYSTEM, window.AvoidAreaType.TYPE_CUTOUT,
      window.AvoidAreaType.TYPE_NAVIGATION_INDICATOR]) {
      try {
        hilog.info(LOG_DOMAIN, LOG_TAG, 'visual avoid area px type=%{public}d %{public}s',
          type, JSON.stringify(win.getWindowAvoidArea(type)));
      } catch (err) {
        hilog.info(LOG_DOMAIN, LOG_TAG, 'visual avoid area unavailable type=%{public}d %{public}s', type, describeError(err));
      }
    }
    try {
      await win.setUIContent('pages/Index');
    } catch (err) {
      const message = describeError(err) || 'set UI content failed';
      hilog.error(LOG_DOMAIN, LOG_TAG, 'set UI content failed %{public}s', message);
      throw new Error(message);
    }
    this.windowFrameMetrics.start(win, this.context.applicationInfo.debug === true);
  }
}
