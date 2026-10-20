import { fileIo } from '@kit.CoreFileKit';
import { hilog } from '@kit.PerformanceAnalysisKit';

interface LifecycleEvent {
  time: number;
  event: string;
}
interface LifecycleState {
  active: boolean;
  events: LifecycleEvent[];
}

/** Single writer: VPN extension. Never records URLs, config or error payloads. */
export class LifecycleJournal {
  private path: string = '';
  private state: LifecycleState = { active: false, events: [] };

  open(filesDir: string): void {
    this.path = `${filesDir}/paws/lifecycle.json`;
    try {
      const stat = fileIo.statSync(this.path);
      if (stat.size <= 32768) {
        const previous = JSON.parse(fileIo.readTextSync(this.path)) as LifecycleState;
        if (Array.isArray(previous.events)) {
          this.state.events = previous.events.slice(-30);
        }
        if (previous.active === true) {
          this.state.events.push({ time: Date.now(), event: 'previous-session-ended-without-cleanup-cause-unknown' });
        }
      }
    } catch (_) {}
    this.record('extension-created', false);
  }

  record(event: string, active: boolean): void {
    if (this.path.length === 0) { return; }
    this.state.active = active;
    this.state.events.push({ time: Date.now(), event });
    this.state.events = this.state.events.slice(-32);
    let file: fileIo.File | undefined = undefined;
    try {
      const temp = `${this.path}.tmp`;
      file = fileIo.openSync(temp, fileIo.OpenMode.WRITE_ONLY | fileIo.OpenMode.CREATE | fileIo.OpenMode.TRUNC);
      fileIo.writeSync(file.fd, JSON.stringify(this.state));
      fileIo.fsyncSync(file.fd);
      fileIo.closeSync(file); file = undefined;
      fileIo.renameSync(temp, this.path);
    } catch (_) {
      hilog.warn(0, 'TinyPawsJournal', 'lifecycle journal write failed');
    } finally {
      if (file !== undefined) { fileIo.closeSync(file); }
    }
  }
}
