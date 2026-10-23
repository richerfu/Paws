import window from '@ohos.window';
import { hilog } from '@kit.PerformanceAnalysisKit';

const LOG_TAG: string = 'PawsFrame';
const SLOW_FRAME_MS: number = 16.7;
const NS_PER_MS: number = 1000000;

type FrameMetricsCallback = (metrics: window.FrameMetrics) => void;
interface FrameMetricsFailure { code?: number; }

/** Debug-only numeric measurements, not frame-interval or presentation timing. */
export class WindowFrameMetrics {
  private win?: window.Window;
  private callback?: FrameMetricsCallback;
  private disposed: boolean = false;
  private frames: number = 0;
  private slowFrames: number = 0;

  /** Call only after setUIContent/loadContent has completed successfully. */
  start(win: window.Window, debug: boolean): void {
    if (!debug || this.disposed || this.win === win) { return; }
    this.detach();
    this.frames = 0;
    this.slowFrames = 0;
    const callback: FrameMetricsCallback = (metrics: window.FrameMetrics): void => {
      if (this.disposed || this.win !== win || this.callback !== callback) { return; }
      this.record(metrics);
    };
    this.win = win;
    this.callback = callback;
    try {
      win.on('frameMetricsMeasured', callback);
    } catch (error) {
      this.win = undefined;
      this.callback = undefined;
      this.reportFailure('register', error);
    }
  }

  dispose(): void {
    this.disposed = true;
    this.detach();
  }

  private record(metrics: window.FrameMetrics): void {
    this.frames++;
    const layoutMs = metrics.layoutMeasureDuration / NS_PER_MS;
    const inputMs = metrics.inputHandlingDuration / NS_PER_MS;
    const slow = layoutMs >= SLOW_FRAME_MS || inputMs >= SLOW_FRAME_MS;
    if (slow) { this.slowFrames++; }
    // The first callback proves delivery. Normal later frames only increment
    // counters: no strings, JSON, timers or timestamp-interval inference.
    if (this.frames === 1 || slow) {
      hilog.info(0, LOG_TAG,
        'frame count=%{public}d slow=%{public}d firstDraw=%{public}d vsyncNs=%{public}s layoutMs=%{public}s inputMs=%{public}s',
        this.frames, this.slowFrames, metrics.firstDrawFrame ? 1 : 0,
        metrics.vsyncTimestamp.toString(), layoutMs.toFixed(3), inputMs.toFixed(3));
    }
  }

  private detach(): void {
    const win = this.win;
    const callback = this.callback;
    // Retire this owner before off(), including queued/reentrant callbacks.
    this.win = undefined;
    this.callback = undefined;
    if (!win || !callback) { return; }
    try {
      win.off('frameMetricsMeasured', callback);
    } catch (error) {
      this.reportFailure('unregister', error);
    }
    hilog.info(0, LOG_TAG, 'stopped frames=%{public}d slow=%{public}d', this.frames, this.slowFrames);
  }

  private reportFailure(action: string, error: Object | undefined | null): void {
    const candidate = (error as FrameMetricsFailure | undefined | null)?.code;
    const code = typeof candidate === 'number' && Number.isFinite(candidate) ? candidate : -1;
    // Platform exception messages may contain arbitrary content; log only code.
    if (code === 801) {
      hilog.info(0, LOG_TAG, 'frame metrics capability unavailable action=%{public}s code=%{public}d', action, code);
    } else {
      hilog.warn(0, LOG_TAG, 'frame metrics skipped action=%{public}s code=%{public}d', action, code);
    }
  }
}
