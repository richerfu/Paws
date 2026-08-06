import { AsyncPluginBase, BridgeCallContext, BridgeContextRequirement, BridgeTypedValue } from '@ohos-rs/ability';
import { fileIo, picker } from '@kit.CoreFileKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { describeError } from '../common/Errors';

const LOG_DOMAIN: number = 0x0000;
const LOG_TAG: string = 'PawsExport';

const EXPORT_REQUEST_TYPE = 'paws.ExportTextRequest';
const EXPORT_RESPONSE_TYPE = 'paws.ExportTextResponse';

interface ExportTextRequest {
  exportKind: string;
  suggestedName: string;
  content: string;
}

/**
 * `paws.export` — exports profile YAML or logs through the system document
 * picker with a pre-filled suggested file name (the built-in `ohos.files`
 * save dialog does not support suggesting a name).
 */
export class ExportPlugin extends AsyncPluginBase {
  readonly id = 'paws.export';
  readonly version = 1;
  readonly requires: BridgeContextRequirement[] = ['ability'];

  async invokeAsync(
    action: string,
    request: BridgeTypedValue,
    _context: BridgeCallContext,
  ): Promise<BridgeTypedValue> {
    if (action !== 'export-text') {
      throw new Error(`Unsupported paws.export action '${action}'`);
    }
    if (request.typeName !== EXPORT_REQUEST_TYPE || typeof request.value !== 'object') {
      throw new Error(`paws.export expected ${EXPORT_REQUEST_TYPE}, got ${request.typeName}`);
    }
    const payload = request.value as ExportTextRequest;
    if (typeof payload.exportKind !== 'string' ||
      typeof payload.suggestedName !== 'string' ||
      typeof payload.content !== 'string') {
      throw new Error(`paws.export invalid export request shape`);
    }
    await this.exportText(payload);
    return { typeName: EXPORT_RESPONSE_TYPE, value: {} };
  }

  private async exportText(payload: ExportTextRequest): Promise<void> {
    let fallbackName: string;
    let extension: string;
    let acceptedExtensions: Array<string>;
    let suffixChoices: Array<string>;
    let label: string;
    if (payload.exportKind === 'profile') {
      fallbackName = 'profile';
      extension = '.yaml';
      acceptedExtensions = ['.yaml', '.yml'];
      suffixChoices = ['YAML|.yaml,.yml'];
      label = 'profile YAML';
    } else if (payload.exportKind === 'log') {
      fallbackName = 'paws-log';
      extension = '.log';
      acceptedExtensions = ['.log'];
      suffixChoices = ['Log|.log', 'Text|.txt'];
      label = 'log';
    } else {
      throw new Error(`paws.export unknown export kind '${payload.exportKind}'`);
    }
    if (payload.content.length === 0) {
      throw new Error(`${label} export content is empty`);
    }
    const baseName = (payload.suggestedName ?? fallbackName)
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
      fileIo.writeSync(file.fd, payload.content);
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
}
