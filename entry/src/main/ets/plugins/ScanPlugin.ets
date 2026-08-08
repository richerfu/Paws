import { AsyncPluginBase, BridgeCallContext, BridgeContextRequirement, BridgeTypedValue } from '@ohos-rs/ability';
import { scanBarcode, scanCore } from '@kit.ScanKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { describeError } from '../common/Errors';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsScan';
const SCAN_CANCELLED_CODE: number = 1000500002;

const SCAN_REQUEST_TYPE = 'paws.ScanRequest';
const SCAN_RESPONSE_TYPE = 'paws.ScanResponse';

interface ScanErrorLike {
  code?: number;
}

/**
 * `paws.scan` — QR scan of a subscription link through ScanKit. The trimmed
 * payload is returned to Rust; an empty string means the user cancelled.
 */
export class ScanPlugin extends AsyncPluginBase {
  readonly id = 'paws.scan';
  readonly version = 1;
  readonly requires: BridgeContextRequirement[] = ['ability'];

  async invokeAsync(
    action: string,
    request: BridgeTypedValue,
    context: BridgeCallContext,
  ): Promise<BridgeTypedValue> {
    if (action !== 'scan-qr') {
      throw new Error(`Unsupported paws.scan action '${action}'`);
    }
    if (request.typeName !== SCAN_REQUEST_TYPE || typeof request.value !== 'object') {
      throw new Error(`paws.scan expected ${SCAN_REQUEST_TYPE}, got ${request.typeName}`);
    }
    const content = await this.scanSubscription(context);
    return { typeName: SCAN_RESPONSE_TYPE, value: { content } };
  }

  private async scanSubscription(context: BridgeCallContext): Promise<string> {
    const options: scanBarcode.ScanOptions = {
      scanTypes: [scanCore.ScanType.QR_CODE],
      enableMultiMode: false,
      enableAlbum: true,
    };
    try {
      const result: scanBarcode.ScanResult =
        await scanBarcode.startScanForResult(context.abilityContext, options);
      return result.originalValue.trim();
    } catch (err) {
      const error = err as ScanErrorLike;
      if (error.code === SCAN_CANCELLED_CODE) {
        return '';
      }
      throw new Error(describeError(err) || 'scan subscription failed');
    }
  }
}
