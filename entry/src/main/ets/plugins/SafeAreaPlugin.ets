import {
  AsyncPluginBase,
  BridgeCallContext,
  BridgeContextRequirement,
  BridgeLifecycleEvent,
  BridgePluginContext,
  BridgeTypedValue,
} from '@ohos-rs/ability';
import window from '@ohos.window';

const VISUAL_AVOID_AREA_TYPES: window.AvoidAreaType[] = [
  window.AvoidAreaType.TYPE_SYSTEM,
  window.AvoidAreaType.TYPE_CUTOUT,
  window.AvoidAreaType.TYPE_NAVIGATION_INDICATOR,
];

export type AvoidAreaPublisher = (options: window.AvoidAreaOptions) => void;

/**
 * `paws.safe-area` captures the main window's initial visual avoid areas.
 *
 * The snapshot is forwarded through Arkit's existing window lifecycle before
 * the XComponent is mounted. This gives native layout a correct first-frame
 * value without shrinking the ArkTS host tree.
 */
export class SafeAreaPlugin extends AsyncPluginBase {
  readonly id = 'paws.safe-area';
  readonly version = 1;
  readonly requires: BridgeContextRequirement[] = ['window-stage'];

  private captured: boolean = false;
  private readonly publishAvoidArea: AvoidAreaPublisher;

  constructor(publishAvoidArea: AvoidAreaPublisher) {
    super();
    this.publishAvoidArea = publishAvoidArea;
  }

  onLifecycle(event: BridgeLifecycleEvent, _context: BridgePluginContext): void {
    if (event.kind === 'window-stage-destroy') {
      this.captured = false;
    }
  }

  async captureInitialSafeArea(): Promise<void> {
    if (this.captured) {
      return;
    }
    const win = await this.getContext().getWindowStage().getMainWindow();
    this.captured = true;
    for (const type of VISUAL_AVOID_AREA_TYPES) {
      try {
        this.publishAvoidArea({ type, area: win.getWindowAvoidArea(type) });
      } catch (_) {
        // Unsupported avoid-area types must not block the first page load.
      }
    }
  }

  async invokeAsync(
    action: string,
    _request: BridgeTypedValue,
    _context: BridgeCallContext,
  ): Promise<BridgeTypedValue> {
    throw new Error(`Unsupported paws.safe-area action '${action}'`);
  }
}
