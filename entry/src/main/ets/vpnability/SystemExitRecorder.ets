import { AbilityConstant } from '@kit.AbilityKit';
import { fileIo } from '@kit.CoreFileKit';
import { hiAppEvent, hilog } from '@kit.PerformanceAnalysisKit';

interface SystemExitRecord {
  receivedAt: number;
  eventTime: string;
  event: string;
  reason: string;
  subReason?: string;
  killId?: string;
  rss?: string;
  pss?: string;
  uid?: string;
  importance?: string;
  desc?: string;
  process: string;
  pid: string;
  foreground: string;
}

/** Each process writes its own file. Never infer a kill cause from missing events. */
export class SystemExitRecorder {
  private path: string = '';
  private records: SystemExitRecord[] = [];
  private holder?: hiAppEvent.AppEventPackageHolder;

  start(filesDir: string, launch?: AbilityConstant.LaunchParam, vpn: boolean = false): void {
    this.path = `${filesDir}/paws/${vpn ? 'system-exits-vpn' : 'system-exits'}.json`;
    try {
      if (fileIo.statSync(this.path).size <= 65536) {
        const previous = JSON.parse(fileIo.readTextSync(this.path)) as SystemExitRecord[];
        if (Array.isArray(previous)) { this.records = previous.slice(-63); }
      }
    } catch (_) {}
    if (launch !== undefined) { this.append({ receivedAt: Date.now(), eventTime: '', event: 'ui-launch-last-exit',
      reason: `${launch.lastExitReason}: ${launch.lastExitMessage ?? ''}`.slice(0, 512),
      process: 'UIAbility', pid: '', foreground: '' }); }
    try {
      this.holder = hiAppEvent.addWatcher({
        name: vpn ? 'TinyPawsVpnExit' : 'TinyPawsSystemExit',
        appEventFilters: [{ domain: 'OS', names: [hiAppEvent.event.APP_KILLED,
          hiAppEvent.event.APP_CRASH, hiAppEvent.event.APP_FREEZE, hiAppEvent.event.RESOURCE_OVERLIMIT] }],
        onReceive: (_domain: string, groups: hiAppEvent.AppEventGroup[]): void => {
          for (const group of groups) {
            for (const info of group.appEventInfos) { this.receive(info); }
          }
        },
      });
      if (!this.holder) { hilog.warn(0, 'TinyPawsExit', 'OS termination watcher returned no holder'); return; }
      this.drain();
      setTimeout((): void => this.drain(), 30000);
      setTimeout((): void => this.drain(), 120000);
      hilog.info(0, 'TinyPawsExit', 'OS termination watcher registered');
    } catch (_) { hilog.warn(0, 'TinyPawsExit', 'OS termination watcher unavailable'); }
  }

  drain(): void {
    try {
      const holder = this.holder;
      if (!holder) { return; }
      holder.setSize(65536);
      for (let count = 0; count < 4; count++) {
        const batch = holder.takeNext();
        if (!batch) { break; }
        for (const info of batch.appEventInfos.slice(0, 64)) { this.receive(info); }
      }
    } catch (error) { hilog.warn(0, 'TinyPawsExit', 'OS event backlog read unavailable: %{public}s', String(error).slice(0, 256)); }
  }

  private receive(info: hiAppEvent.AppEventInfo): void {
    const rawReason = this.scalar(info, 'reason') || 'system-reason-not-scalar';
    const subReason = this.scalar(info, 'sub_reason') || this.scalar(info, 'subReason');
    const killId = this.scalar(info, 'kill_id') || this.scalar(info, 'killId');
    const rss = this.scalar(info, 'rss') || this.scalar(info, 'pss_memory');
    const pss = this.scalar(info, 'pss');
    const uid = this.scalar(info, 'uid');
    const importance = this.scalar(info, 'importance') || this.scalar(info, 'adj');
    const desc = this.scalar(info, 'description') || this.scalar(info, 'msg') || this.scalar(info, 'message');
    const reasonSummary = subReason.length > 0 && !rawReason.includes(subReason)
      ? `${rawReason} (${subReason})`
      : rawReason;
    this.append({
      receivedAt: Date.now(),
      eventTime: this.scalar(info, 'time'),
      event: info.name,
      reason: reasonSummary.slice(0, 512),
      subReason: subReason.length > 0 ? subReason : undefined,
      killId: killId.length > 0 ? killId : undefined,
      rss: rss.length > 0 ? rss : undefined,
      pss: pss.length > 0 ? pss : undefined,
      uid: uid.length > 0 ? uid : undefined,
      importance: importance.length > 0 ? importance : undefined,
      desc: desc.length > 0 ? desc.slice(0, 512) : undefined,
      process: this.scalar(info, 'process_name'),
      pid: this.scalar(info, 'pid'),
      foreground: this.scalar(info, 'foreground'),
    });
  }

  private scalar(info: hiAppEvent.AppEventInfo, key: string): string {
    const params = info.params as Record<string, hiAppEvent.ParamType>;
    const value: hiAppEvent.ParamType = params[key];
    if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
      return String(value).slice(0, 256);
    }
    return '';
  }

  private append(record: SystemExitRecord): void {
    if (record.eventTime.length > 0 && this.records.some((item: SystemExitRecord): boolean =>
      item.event === record.event && item.eventTime === record.eventTime && item.pid === record.pid && item.reason === record.reason)) { return; }
    this.records.push(record);
    this.records = this.records.slice(-64);
    let file: fileIo.File | undefined = undefined;
    try {
      file = fileIo.openSync(`${this.path}.tmp`, fileIo.OpenMode.WRITE_ONLY | fileIo.OpenMode.CREATE | fileIo.OpenMode.TRUNC);
      fileIo.writeSync(file.fd, JSON.stringify(this.records));
      fileIo.fsyncSync(file.fd);
      fileIo.closeSync(file); file = undefined;
      fileIo.renameSync(`${this.path}.tmp`, this.path);
      hilog.info(0, 'TinyPawsExit', 'recorded %{public}s: %{public}s', record.event, record.reason);
    } catch (_) { hilog.warn(0, 'TinyPawsExit', 'OS exit record write failed'); }
    finally { if (file !== undefined) { fileIo.closeSync(file); } }
  }
}
