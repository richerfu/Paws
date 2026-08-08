import { AbilityConstant, Configuration, ConfigurationConstant, Want } from '@kit.AbilityKit';
import { i18n } from '@kit.LocalizationKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { EagerPlugin, LazyPlugin, NativeAbility } from '@ohos-rs/ability';
import { FilesPlugin } from '@ohos-rs/ability-plugin-files';
import { UrlPlugin } from '@ohos-rs/ability-plugin-url';
import window from '@ohos.window';
import hmetaUi from 'libhmeta_ui.so';
import { describeError } from '../common/Errors';
import { setHMetaBundleName } from '../vpnability/VpnConfig';
import { runDebugAutomation } from '../automation/DebugAutomation';
import { ClipboardPlugin } from '../plugins/ClipboardPlugin';
import { ColorModePlugin } from '../plugins/ColorModePlugin';
import { ExportPlugin } from '../plugins/ExportPlugin';
import { ScanPlugin } from '../plugins/ScanPlugin';
import { VpnPlugin } from '../plugins/VpnPlugin';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'HMetaEntry';

/**
 * Thin assembly point: loads the native module, installs the built-in
 * capability plugins (`ohos.files`, `ohos.url`) plus the application-owned
 * `paws.*` plugins, and forwards lifecycle events. All platform logic lives
 * in the plugins; debug automation lives in `DebugAutomation`.
 */
export default class EntryAbility extends NativeAbility {
  public moduleName: string = 'hmeta_ui';
  public defaultPage: boolean = false;

  private readonly vpnPlugin: VpnPlugin = new VpnPlugin();

  public bridgePlugins = [
    new LazyPlugin(() => new FilesPlugin()),
    new LazyPlugin(() => new UrlPlugin()),
    // The VPN plugin instance is shared with debug automation.
    new LazyPlugin(() => this.vpnPlugin),
    new LazyPlugin(() => new ScanPlugin()),
    new LazyPlugin(() => new ClipboardPlugin()),
    new LazyPlugin(() => new ColorModePlugin()),
    new LazyPlugin(() => new ExportPlugin()),
  ];

  private configureNativeHome(): void {
    try {
      hmetaUi.configureAppHome(`${this.context.filesDir}/paws`);
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

  private seedGeodataFromRawfiles(): void {
    try {
      const seeded = hmetaUi.seedGeodataFromRawfiles(this.context.resourceManager);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'seeded %{public}d geodata rawfiles', seeded);
    } catch (err) {
      hilog.info(LOG_DOMAIN, LOG_TAG, 'geodata rawfile seed skipped %{public}s', describeError(err));
    }
  }

  public async onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): Promise<void> {
    try {
      setHMetaBundleName(this.context.applicationInfo.name);
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
      await win.setUIContent('pages/Index');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'set UI content failed %{public}s', JSON.stringify(err) ?? '');
    }
  }
}
