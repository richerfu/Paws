import { AbilityConstant, Configuration, ConfigurationConstant, Want } from '@kit.AbilityKit';
import { i18n } from '@kit.LocalizationKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { EagerPlugin, LazyPlugin, NativeAbility } from '@ohos-rs/ability';
import { FilesPlugin } from '@ohos-rs/ability-plugin-files';
import { UrlPlugin } from '@ohos-rs/ability-plugin-url';
import window from '@ohos.window';
import pawsUi from 'libpaws_ui.so';
import { describeError } from '../common/Errors';
import { setPawsBundleName } from '../vpnability/VpnConfig';
import { runDebugAutomation } from '../automation/DebugAutomation';
import { ClipboardPlugin } from '../plugins/ClipboardPlugin';
import { ColorModePlugin } from '../plugins/ColorModePlugin';
import { ExportPlugin } from '../plugins/ExportPlugin';
import { SafeAreaPlugin } from '../plugins/SafeAreaPlugin';
import { ScanPlugin } from '../plugins/ScanPlugin';
import { VpnPlugin } from '../plugins/VpnPlugin';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsEntry';

/**
 * Thin assembly point: loads the native module, installs the built-in
 * capability plugins (`ohos.files`, `ohos.url`) plus the application-owned
 * `paws.*` plugins, and forwards lifecycle events. All platform logic lives
 * in the plugins; debug automation lives in `DebugAutomation`.
 */
export default class EntryAbility extends NativeAbility {
  public moduleName: string = 'paws_ui';
  public defaultPage: boolean = false;

  private readonly vpnPlugin: VpnPlugin = new VpnPlugin();
  private readonly safeAreaPlugin: SafeAreaPlugin = new SafeAreaPlugin(
    (options: window.AvoidAreaOptions): void => {
      this.forEachLifecycle((lifecycle) =>
        lifecycle.windowStageEventCallback.onAvoidAreaChange(options),
      );
    },
  );

  public bridgePlugins = [
    new LazyPlugin(() => new FilesPlugin()),
    new LazyPlugin(() => new UrlPlugin()),
    // VPN owns a process-lifetime IPC event pump and must be installed before
    // the UI subscribes to lifecycle events. The instance is also shared with
    // debug automation.
    new EagerPlugin(this.vpnPlugin),
    new LazyPlugin(() => new ScanPlugin()),
    new LazyPlugin(() => new ClipboardPlugin()),
    new LazyPlugin(() => new ColorModePlugin()),
    new LazyPlugin(() => new ExportPlugin()),
    new EagerPlugin(this.safeAreaPlugin),
  ];

  private configureNativeHome(): void {
    try {
      pawsUi.configureAppHome(`${this.context.filesDir}/paws`);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configured native home');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'configure native home failed %{public}s', describeError(err));
    }
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
      (pawsUi as ESObject).configureUiLocale(locale);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configured native locale %{public}s', locale);
    } catch (err) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'configure native locale skipped %{public}s', describeError(err));
    }
  }

  private configureNativeAppearance(colorMode?: ConfigurationConstant.ColorMode): void {
    try {
      const mode = colorMode ?? this.context.config.colorMode ?? ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET;
      (pawsUi as ESObject).configureSystemColorMode(mode);
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

  public async onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): Promise<void> {
    try {
      setPawsBundleName(this.context.applicationInfo.name);
      this.configureNativeHome();
      this.configureNativeLocale();
      this.configureNativeAppearance();
      this.seedGeodataFromRawfiles();
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'native initialization failed %{public}s', describeError(err));
    }
    await super.onCreate(want, launchParam);
    runDebugAutomation(want, (optionsJson: string) => this.vpnPlugin.requestStartVpn(optionsJson));
  }

  public onNewWant(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    super.onNewWant(want, launchParam);
    runDebugAutomation(want, (optionsJson: string) => this.vpnPlugin.requestStartVpn(optionsJson));
  }

  public onConfigurationUpdate(newConfig: Configuration): void {
    this.configureNativeLocale(newConfig.language);
    this.configureNativeAppearance(newConfig.colorMode);
    super.onConfigurationUpdate(newConfig);
  }

  public async onWindowStageCreate(windowStage: window.WindowStage): Promise<void> {
    await super.onWindowStageCreate(windowStage);
    try {
      const win = await windowStage.getMainWindow();
      await win.setWindowLayoutFullScreen(true);
      try {
        await this.safeAreaPlugin.captureInitialSafeArea();
      } catch (err) {
        hilog.info(LOG_DOMAIN, LOG_TAG, 'initial safe area skipped %{public}s', describeError(err));
      }
      await win.setUIContent('pages/Index');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'set UI content failed %{public}s', JSON.stringify(err) ?? '');
    }
  }
}
