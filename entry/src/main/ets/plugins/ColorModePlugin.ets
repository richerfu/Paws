import { AsyncPluginBase, BridgeCallContext, BridgeContextRequirement, BridgeTypedValue } from '@ohos-rs/ability';
import { ConfigurationConstant } from '@kit.AbilityKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { describeError } from '../common/Errors';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsColorMode';

const COLOR_MODE_REQUEST_TYPE = 'paws.ColorModeRequest';
const COLOR_MODE_RESPONSE_TYPE = 'paws.ColorModeResponse';

interface ColorModeRequest {
  mode: number;
}

/**
 * `paws.color-mode` — applies the app-wide color mode from native UI on the
 * Ability context.
 */
export class ColorModePlugin extends AsyncPluginBase {
  readonly id = 'paws.color-mode';
  readonly requires: BridgeContextRequirement[] = ['ability'];

  async invokeAsync(
    action: string,
    request: BridgeTypedValue,
    context: BridgeCallContext,
  ): Promise<BridgeTypedValue> {
    if (action !== 'set-color-mode') {
      throw new Error(`Unsupported paws.color-mode action '${action}'`);
    }
    if (request.typeName !== COLOR_MODE_REQUEST_TYPE || typeof request.value !== 'object') {
      throw new Error(
        `paws.color-mode expected ${COLOR_MODE_REQUEST_TYPE}, got ${request.typeName}`,
      );
    }
    const payload = request.value as ColorModeRequest;
    if (typeof payload.mode !== 'number' || !Number.isInteger(payload.mode)) {
      throw new Error(`paws.color-mode invalid request shape`);
    }
    let mode = ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET;
    if (payload.mode === ConfigurationConstant.ColorMode.COLOR_MODE_DARK) {
      mode = ConfigurationConstant.ColorMode.COLOR_MODE_DARK;
    } else if (payload.mode === ConfigurationConstant.ColorMode.COLOR_MODE_LIGHT) {
      mode = ConfigurationConstant.ColorMode.COLOR_MODE_LIGHT;
    }
    try {
      await context.abilityContext.setColorMode(mode);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'applied app color mode %{public}d', mode);
    } catch (err) {
      const message = describeError(err) || 'apply app color mode failed';
      hilog.error(LOG_DOMAIN, LOG_TAG, 'apply app color mode failed %{public}s', message);
      throw new Error(message);
    }
    return { typeName: COLOR_MODE_RESPONSE_TYPE, value: {} };
  }
}
