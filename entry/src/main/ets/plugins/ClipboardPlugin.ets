import { AsyncPluginBase, BridgeCallContext, BridgeContextRequirement, BridgeTypedValue } from '@ohos-rs/ability';
import { pasteboard } from '@kit.BasicServicesKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { describeError } from '../common/Errors';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsClipboard';

const CLIPBOARD_REQUEST_TYPE = 'paws.ClipboardSetRequest';
const CLIPBOARD_RESPONSE_TYPE = 'paws.ClipboardSetResponse';

interface ClipboardSetRequest {
  text: string;
}

/**
 * `paws.clipboard` — copies text to the system pasteboard.
 */
export class ClipboardPlugin extends AsyncPluginBase {
  readonly id = 'paws.clipboard';
  readonly requires: BridgeContextRequirement[] = ['ability'];

  async invokeAsync(
    action: string,
    request: BridgeTypedValue,
    _context: BridgeCallContext,
  ): Promise<BridgeTypedValue> {
    if (action !== 'set-text') {
      throw new Error(`Unsupported paws.clipboard action '${action}'`);
    }
    if (request.typeName !== CLIPBOARD_REQUEST_TYPE || typeof request.value !== 'object') {
      throw new Error(
        `paws.clipboard expected ${CLIPBOARD_REQUEST_TYPE}, got ${request.typeName}`,
      );
    }
    const payload = request.value as ClipboardSetRequest;
    if (typeof payload.text !== 'string') {
      throw new Error(`paws.clipboard invalid request shape`);
    }
    try {
      const data = pasteboard.createData(pasteboard.MIMETYPE_TEXT_PLAIN, payload.text);
      await pasteboard.getSystemPasteboard().setData(data);
      hilog.info(LOG_DOMAIN, LOG_TAG, 'copied text to system pasteboard');
    } catch (err) {
      hilog.error(LOG_DOMAIN, LOG_TAG, 'copy text failed %{public}s', describeError(err));
      throw new Error(describeError(err) || 'copy text failed');
    }
    return { typeName: CLIPBOARD_RESPONSE_TYPE, value: {} };
  }
}
