import { liveViewManager } from '@kit.LiveViewKit';
import { WantAgent } from '@kit.AbilityKit';
import { hilog } from '@kit.PerformanceAnalysisKit';

/** Huawei capability is optional. Rights/settings failures never stop the VPN. */
export class TinyPawsLiveView {
  private active: boolean = false;
  private unavailable: boolean = false;
  private busy: boolean = false;
  private generation: number = 0;
  private makeView(text: string, agent?: WantAgent): liveViewManager.LiveView {
    return {
      id: 17042, event: 'PROGRESS', isMute: true,
      liveViewData: { primary: {
        title: 'Paws · VPN', content: [{ text }], keepTime: 0, clickAction: agent,
        layoutData: { layoutType: liveViewManager.LayoutType.LAYOUT_TYPE_DEFAULT, progress: 0 },
      } },
    };
  }
  async update(text: string, agent?: WantAgent): Promise<void> {
    if (this.busy || this.unavailable) { return; }
    const generation = this.generation;
    this.busy = true;
    try {
      if (!await liveViewManager.isLiveViewEnabled()) { this.unavailable = true; return; }
      if (generation !== this.generation) { return; }
      const view = this.makeView(text, agent);
      const result = this.active ? await liveViewManager.updateLiveView(view) : await liveViewManager.startLiveView(view);
      if (generation !== this.generation) { await liveViewManager.stopLiveView(this.makeView('VPN 已停止')); return; }
      if (result.resultCode !== 0) { throw new Error(`live view result ${result.resultCode}`); }
      this.active = true;
    } catch (error) {
      this.unavailable = true;
      hilog.warn(0, 'TinyPawsLiveView', 'Live View unavailable; keeping standard VPN notification: %{public}s', String(error));
    } finally { this.busy = false; }
  }
  stop(): void {
    this.generation += 1;
    this.unavailable = false;
    if (this.active) {
      this.active = false;
      liveViewManager.stopLiveView(this.makeView('VPN 已停止')).catch(() => {});
    }
  }
}
