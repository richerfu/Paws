import { ComponentContent, UIContext } from '@kit.ArkUI';
import { ConfigurationConstant, common } from '@kit.AbilityKit';
import { fileIo, picker } from '@kit.CoreFileKit';
import { util } from '@kit.ArkTS';
import { generateBarcode, scanBarcode, scanCore } from '@kit.ScanKit';
import { image } from '@kit.ImageKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { NshNativeProperty, NshSurface } from 'libnsh_platform_ohos.so';
import pawsUi, { UiCommandSession } from 'libpaws_ui.so';
import window from '@ohos.window';
import { VpnPlugin } from '../plugins/VpnPlugin';
import { ExportPlugin } from '../plugins/ExportPlugin';
import { describeError, describeUserError } from '../common/Errors';
import { createGatewayNode } from './PawsGatewayNode';

interface UiRequest { id: number; command: string; payload: Object; }
interface UiEffect { kind: string; owner: string; configRevision: string; optionsJson: string; }
interface DomainRevision { domain: string; groupKey?: string; revision: number; }
interface StateChanged { generation: number; revision: number; changes: DomainRevision[]; }
interface StateBarrier { generation: number; required: DomainRevision[]; }
interface StateObservation { change: StateChanged; barrier: StateBarrier; }
interface UiResult { id: number; ok: boolean; data?: Object | null; error?: string; effect?: UiEffect; barrier?: StateBarrier; stateRefreshError?: string; }
interface PlatformPayload { url?: string; name?: string; content?: string; text?: string; pngBase64?: string; }
interface UiPreferences { language: string; theme: string; advanced: boolean; systemLocale: string; systemColorMode?: number; }
interface ImportedFile { name: string; content: string; }
interface ScanFailure { code?: number; }
interface QrPayload { source: string; pngBase64: string; }
interface VpnReceiptPayload { requestId: number; }
interface VpnReceipt { requestId: number; status: string; result: boolean; error: string; }
interface UiReceived { id: number; }
interface UiOperation { requestId: number; action: string; }
interface StateQueryPayload { domain: string; groupKey?: string; }

let commandSessionSequence: number = 0;

/** Owns cancellable Rust preparation and the namespace of observable requests. */
export class PawsCommandSession {
  readonly commands: UiCommandSession = new UiCommandSession();
  private readonly receiptScope: string = `nsh-${++commandSessionSequence}`;
  private readonly queryResponseChars: Map<number, number> = new Map();
  onOperation?: (requestId: number, action: string) => void;

  requestKey(id: number): string { return `${this.receiptScope}:${id}`; }
  recordQueryResponse(id: number, chars: number): void { this.queryResponseChars.set(id, chars); }
  takeQueryResponseChars(id: number): number {
    const chars = this.queryResponseChars.get(id) ?? 0;
    this.queryResponseChars.delete(id);
    return chars;
  }
  close(): void { this.queryResponseChars.clear(); this.commands.close(); }
}

/** System capability owner. It is created by the Ability, never by a Rust UI bridge. */
export class PawsHostServices {
  readonly vpn: VpnPlugin = new VpnPlugin();
  private readonly context: common.UIAbilityContext;
  private closed: boolean = false;
  private vpnIntentGeneration: number = 0;
  private mainWindow?: window.Window;
  private requestedBarDark?: boolean;
  private barUpdates: Promise<void> = Promise.resolve();

  constructor(context: common.UIAbilityContext) {
    this.context = context;
    this.vpn.attachHost(context);
  }

  dispose(): void { this.closed = true; this.mainWindow = undefined; this.vpn.detachHost(); }

  attachMainWindow(mainWindow: window.Window): void {
    this.mainWindow = mainWindow;
    this.requestedBarDark = undefined;
  }

  ensureActive(): void {
    if (this.closed) { throw new Error('Paws Ability session is closed'); }
  }

  async dispatch(request: UiRequest, session: PawsCommandSession): Promise<UiResult> {
    this.ensureActive();
    if (request.command === 'vpn.confirm') {
      const originalId = this.receiptRequestId(request);
      const lookup = this.vpn.lookupHostOperation(session.requestKey(originalId));
      const receipt: VpnReceipt = { requestId: originalId,
        status: lookup.status === 'found' ? lookup.operationStatus : 'unavailable',
        result: lookup.result, error: lookup.error };
      return { id: request.id, ok: true, data: receipt };
    }
    const recover = request.command === 'vpn.recover';
    const vpnIntent = recover || request.command === 'vpn.start' || request.command === 'vpn.stop' || request.command === 'vpn.restart';
    const generation = vpnIntent ? ++this.vpnIntentGeneration : this.vpnIntentGeneration;
    if (!vpnIntent) { return this.dispatchCommand(request, session, false, generation); }
    const originalId = recover ? this.receiptRequestId(request) : request.id;
    const coreRequest: UiRequest = recover ? { id: request.id, command: 'vpn.stop', payload: new Object() } : request;
    const operationId = this.vpn.submitHostOperation(session.requestKey(request.id), request.command,
      JSON.stringify(request.payload), async (): Promise<boolean> => {
        const result = await this.dispatchCommand(coreRequest, session, true, generation);
        if (!result.ok) { throw new Error(result.error ?? 'VPN command failed'); }
        return true;
      });
    const terminal = await this.vpn.finishHostOperation(operationId);
    const receipt: VpnReceipt = { requestId: originalId, status: terminal.status,
      result: terminal.result, error: terminal.error };
    return { id: request.id, ok: terminal.status === 'succeeded' && terminal.result,
      data: recover ? receipt : null, error: terminal.error || undefined };
  }

  private receiptRequestId(request: UiRequest): number {
    const id = (request.payload as VpnReceiptPayload).requestId;
    if (!Number.isSafeInteger(id) || id < 1) { throw new Error('A valid original VPN requestId is required'); }
    return id;
  }

  private async dispatchCommand(request: UiRequest, session: PawsCommandSession,
    vpnIntent: boolean, generation: number): Promise<UiResult> {
    if (request.command === 'state.query') {
      const response = await session.commands.queryState(JSON.stringify(request.payload));
      const parsedAt = Date.now();
      const data = JSON.parse(response) as Object;
      const elapsed = Date.now() - parsedAt;
      session.recordQueryResponse(request.id, response.length);
      if (elapsed >= 16 || response.length >= 64 * 1024) {
        const query = request.payload as StateQueryPayload;
        hilog.info(0x0000, 'PawsNsh', 'domain %{public}s group %{public}s query parsed %{public}d chars in %{public}d ms',
          query.domain, query.groupKey ?? '', response.length, elapsed);
      }
      return { id: request.id, ok: true, data };
    }
    const payload = request.payload as PlatformPayload;
    if (request.command === 'platform.scan') {
      try {
        const result = await scanBarcode.startScanForResult(this.context, {
          scanTypes: [scanCore.ScanType.QR_CODE], enableMultiMode: false, enableAlbum: true,
        });
        return { id: request.id, ok: true, data: result.originalValue.trim() };
      } catch (error) {
        if ((error as ScanFailure).code === 1000500002) {
          return { id: request.id, ok: true, data: '' };
        }
        throw new Error(describeUserError(error));
      }
    }
    if (request.command === 'platform.importFile') {
      const imported = await this.importFile();
      return { id: request.id, ok: true, data: imported };
    }
    if (request.command === 'platform.qr') {
      const text = payload.text ?? '';
      if (!text.length || text.length > 4096) { throw new Error('QR content must contain between 1 and 4096 characters'); }
      const options: generateBarcode.CreateOptions = {
        scanType: scanCore.ScanType.QR_CODE, width: 640, height: 640,
        margin: 4, level: generateBarcode.ErrorCorrectionLevel.LEVEL_M,
      };
      const bitmap: image.PixelMap = await generateBarcode.createBarcode(text, options);
      try {
        const packer = image.createImagePacker();
        try {
          const packing: image.PackingOption = { format: 'image/png', quality: 100 };
          const png: ArrayBuffer = await packer.packing(bitmap, packing);
          const pngBase64 = new util.Base64Helper().encodeToStringSync(new Uint8Array(png));
          const data: QrPayload = { source: `data:image/png;base64,${pngBase64}`, pngBase64 };
          const response: UiResult = { id: request.id, ok: true, data };
          return response;
        } finally { await packer.release(); }
      } finally { await bitmap.release(); }
    }
    if (request.command === 'platform.exportQr') {
      await new ExportPlugin().exportImage({ suggestedName: payload.name ?? 'profile', pngBase64: payload.pngBase64 ?? '' });
      return { id: request.id, ok: true, data: null };
    }
    if (request.command === 'platform.openUrl') {
      const install = payload.url?.match(/^clash:\/\/install-config\?url=([^&]+)$/);
      const clashInstall = install !== null && install !== undefined &&
        /^https?:\/\//.test(decodeURIComponent(install[1]));
      if (!payload.url || (!/^https?:\/\//.test(payload.url) && !clashInstall)) {
        throw new Error('An absolute HTTP or HTTPS URL or a Clash install-config link is required');
      }
      await this.context.openLink(payload.url);
      return { id: request.id, ok: true, data: null };
    }
    if (request.command === 'platform.export') {
      const name = payload.name ?? 'Paws';
      const exportKind = /\.ya?ml$/i.test(name) ? 'profile' : /\.log$/i.test(name) ? 'log' : 'backup';
      await new ExportPlugin().exportText({ exportKind, suggestedName: name, content: payload.content ?? '' });
      return { id: request.id, ok: true, data: null };
    }
    const response = await session.commands.dispatch(JSON.stringify(request));
    const parsedAt = Date.now();
    const result = JSON.parse(response) as UiResult;
    const parseDuration = Date.now() - parsedAt;
    if (parseDuration >= 16 || response.length >= 256 * 1024) {
      hilog.info(0x0000, 'PawsNsh', 'command %{public}s parsed %{public}d chars in %{public}d ms', request.command, response.length, parseDuration);
    }
    this.ensureActive();
    if (vpnIntent && generation !== this.vpnIntentGeneration) {
      throw new Error('VPN request was superseded');
    }
    if (result.ok && result.effect) {
      const effect = result.effect;
      if (vpnIntent) {
        session.onOperation?.(request.id, effect.kind);
        await this.applyEffect(effect);
      } else {
        const operationId = this.vpn.submitHostOperation(session.requestKey(request.id),
          `effect:${effect.kind}`, JSON.stringify(effect), (): Promise<void> => this.applyEffect(effect));
        session.onOperation?.(request.id, effect.kind);
        const terminal = await this.vpn.finishHostOperation(operationId);
        if (terminal.status !== 'succeeded' || !terminal.result) {
          throw new Error(terminal.error || 'Configuration saved, but its VPN operation is unconfirmed');
        }
      }
    }
    return result;
  }

  private async applyEffect(effect: UiEffect): Promise<void> {
    if (effect.kind === 'start') {
      await this.startPreparedEffect(effect);
    } else if (effect.kind === 'restart') {
      if (effect.owner.length === 0) { await this.startPreparedEffect(effect); }
      else if (!await this.vpn.requestRestartVpn(effect.owner, effect.configRevision, effect.optionsJson)) {
        throw new Error('Changes were saved, but the VPN session changed before they could be applied.');
      }
    } else if (effect.kind === 'stop') {
      if (effect.owner.length === 0) {
        if (!await this.vpn.requestStopVpn()) { throw new Error('VPN Stop was superseded before completion'); }
      }
      else if (!await this.vpn.requestStopVpnIfCurrent(effect.owner, effect.configRevision)) {
        throw new Error('Changes were saved, but the VPN session changed before it could be stopped.');
      }
    } else {
      throw new Error(`Unsupported VPN effect '${effect.kind}'`);
    }
  }

  private async startPreparedEffect(effect: UiEffect): Promise<void> {
    if (!pawsUi.isRuntimeConfigRevisionCurrent(effect.configRevision)) {
      throw new Error('Configuration changed while preparing VPN. Retry Start.');
    }
    await this.vpn.requestStartVpn(effect.optionsJson);
  }

  applyPreferences(preferences: UiPreferences): void {
    this.ensureActive();
    // NSH owns the app's appearance and explicit control colors. Changing the
    // Ability color mode would also rebuild ArkUI's global resource theme.
    // System pickers retain the actual system appearance.
    const dark = preferences.theme === 'dark' || (preferences.theme === 'system' &&
      preferences.systemColorMode === ConfigurationConstant.ColorMode.COLOR_MODE_DARK);
    const target = this.mainWindow;
    if (!target || this.requestedBarDark === dark) { return; }
    this.requestedBarDark = dark;
    // Coalesce queued transitions and keep the last completed platform write
    // in the same order as preferences. A slow old write cannot win last.
    this.barUpdates = this.barUpdates.then(async (): Promise<void> => {
      if (this.closed || this.mainWindow !== target || this.requestedBarDark !== dark) { return; }
      await target.setWindowBackgroundColor('#00000000');
      if (this.closed || this.mainWindow !== target || this.requestedBarDark !== dark) { return; }
      const content = dark ? '#ffffffff' : '#ff000000';
      await target.setWindowSystemBarProperties({
        statusBarColor: '#00000000', navigationBarColor: '#00000000',
        statusBarContentColor: content, navigationBarContentColor: content,
      });
    }).catch((error: Error): void => {
      if (this.mainWindow === target && this.requestedBarDark === dark) { this.requestedBarDark = undefined; }
      hilog.warn(0x0000, 'PawsNsh', 'system bar appearance failed %{public}s', describeError(error));
    });
  }

  private async importFile(): Promise<ImportedFile | null> {
    const options = new picker.DocumentSelectOptions();
    options.maxSelectNumber = 1;
    const uris = await new picker.DocumentViewPicker(this.context).select(options);
    if (!uris.length) { return null; }
    const file = await fileIo.open(uris[0], fileIo.OpenMode.READ_ONLY);
    try {
      const size = (await fileIo.stat(file.fd)).size;
      if (size < 1 || size > 16 * 1024 * 1024) {
        throw new Error('Import file must contain between 1 byte and 16 MiB');
      }
      const bytes = new ArrayBuffer(size);
      const read = await fileIo.read(file.fd, bytes);
      if (read !== size) { throw new Error('Import file could not be read completely'); }
      const name = decodeURIComponent(uris[0].slice(uris[0].lastIndexOf('/') + 1));
      return { name, content: util.TextDecoder.create('utf-8').decodeToString(new Uint8Array(bytes)) };
    } finally { await fileIo.close(file); }
  }
}

let currentHost: PawsHostServices | undefined;

export function installPawsHost(context: common.UIAbilityContext): PawsHostServices {
  currentHost?.dispose();
  currentHost = new PawsHostServices(context);
  return currentHost;
}

export function pawsHost(): PawsHostServices {
  if (!currentHost) { throw new Error('Paws Ability services are not initialized'); }
  currentHost.ensureActive();
  return currentHost;
}

/** A zero-size NSH native component carrying typed commands and observable state. */
export class PawsCoreGateway {
  node: ComponentContent<Object>;
  private readonly surface: NshSurface;
  private readonly handle: number;
  private readonly host: PawsHostServices;
  private disposed: boolean = false;
  private lastRequest: number = -1;
  private retryTimer: number = -1;
  private readonly receiptTimers: Set<number> = new Set();
  private publishedPreferences: string = '';
  private pendingChanges?: StateChanged;
  private readonly session: PawsCommandSession = new PawsCommandSession();

  constructor(context: UIContext, handle: number, surface: NshSurface, host: PawsHostServices) {
    this.surface = surface;
    this.handle = handle;
    this.host = host;
    this.session.onOperation = (requestId: number, action: string): void => {
      this.emit('operation', { requestId, action } as UiOperation);
    };
    this.node = createGatewayNode(context);
    hilog.info(0x0000, 'PawsNsh', 'core gateway mounted handle %{public}d', handle);
    surface.setComponentSize(handle, 0, 0);
    // The initial native mount must finish before event handlers receive state.
    setTimeout((): void => { void this.observe(); }, 0);
    setTimeout((): void => {
      if (!this.disposed) {
        const bootstrap: UiRequest = { id: 0, command: 'bootstrap', payload: new Object() };
        void this.host.dispatch(bootstrap, this.session)
          .then(async (result: UiResult): Promise<void> => {
            if (!result.ok) { this.emit('result', result); }
            if (!this.disposed) { this.publishChanges(); }
          })
          .catch((error: Error): void => {
            this.emit('result', { id: 0, ok: false, error: describeUserError(error) } as UiResult);
          });
      }
    }, 0);
  }

  updateProps(changes: NshNativeProperty[]): void {
    if (this.disposed || this.surface.isDisposed || this.node.isDisposed()) { return; }
    for (const change of changes) {
      if (change.name !== 'request' || change.removed || change.value === null) { continue; }
      const request = change.value as UiRequest;
      if (!Number.isSafeInteger(request.id) || request.id < 0 || typeof request.command !== 'string') { continue; }
      // Initial native props can precede event-listener installation in the
      // same mount batch. A receipt goes out on the next ArkTS turn, without
      // waiting for command completion or serializing Stop behind Start.
      const receiptTimer = setTimeout((): void => {
        this.receiptTimers.delete(receiptTimer);
        if (this.disposed || this.surface.isDisposed || this.node.isDisposed()) { return; }
        this.emit('received', { id: request.id } as UiReceived);
      }, 0);
      this.receiptTimers.add(receiptTimer);
      if (request.id <= this.lastRequest) { continue; }
      this.lastRequest = request.id;
      // Stop must reach the lifecycle owner while an earlier Start is pending.
      // Rust and the VPN host own ordering through revision and intent fences.
      if (request.command === 'state.subscribe') {
        // Initial props may precede the changed listener in the native mount.
        setTimeout((): void => { void this.runRequest(request); }, 0);
      } else { void this.runRequest(request); }
    }
  }

  private async runRequest(request: UiRequest): Promise<void> {
    if (this.disposed) { return; }
    let result: UiResult;
    const subscribe = request.command === 'state.subscribe';
    const query = request.command === 'state.query';
    try {
      if (subscribe) {
        const change = JSON.parse(this.session.commands.subscribeState(JSON.stringify(request.payload))) as StateChanged;
        this.emitChanged(change);
        result = { id: request.id, ok: true, data: change };
      } else { result = await this.host.dispatch(request, this.session); }
    }
    catch (error) { result = { id: request.id, ok: false, error: describeUserError(error) }; }
    if (this.disposed) { return; }
    if (!subscribe && !query) {
      try {
        this.publishPreferences();
        const critical = request.command === 'vpn.stop' || request.command === 'vpn.recover' || request.command === 'profiles.cancelImport';
        const observation = this.publishChanges(critical);
        if (result.ok) { result.barrier = observation.barrier; }
      } catch (error) {
        // The operation already completed. A failed read cannot turn a saved
        // mutation into an apparent failure which the user might resubmit.
        if (result.ok) { result.stateRefreshError = `Operation completed, but state refresh failed: ${describeUserError(error)}`; }
      }
    }
    const queryChars = this.session.takeQueryResponseChars(request.id);
    const emittedAt = Date.now();
    this.emit('result', result);
    const emitDuration = Date.now() - emittedAt;
    if (query && (emitDuration >= 16 || queryChars >= 64 * 1024)) {
      const payload = request.payload as StateQueryPayload;
      hilog.info(0x0000, 'PawsNsh', 'domain %{public}s group %{public}s result emitted %{public}d chars in %{public}d ms',
        payload.domain, payload.groupKey ?? '', queryChars, emitDuration);
    } else if (emitDuration >= 16) {
      hilog.warn(0x0000, 'PawsNsh', 'command %{public}s result emit blocked host for %{public}d ms', request.command, emitDuration);
    }
  }

  dispose(): void {
    if (this.disposed) { return; }
    this.disposed = true;
    for (const timer of this.receiptTimers) { clearTimeout(timer); }
    this.receiptTimers.clear();
    this.session.close();
    if (this.retryTimer >= 0) { clearTimeout(this.retryTimer); }
    pawsUi.cancelUiRevisionWait();
    if (!this.node.isDisposed()) { this.node.dispose(); }
  }

  private emit(name: string, data: Object): void {
    if (this.disposed || this.surface.isDisposed || this.node.isDisposed()) { return; }
    try {
      this.surface.emitComponentEvent(this.handle, name, data);
    } catch (error) {
      // The native handle can retire before its queued JS dispose callback.
      // Surface flags describe the whole surface, not this component registry.
      if (error instanceof Error && error.message === 'native component is disposed') {
        this.dispose();
        return;
      }
      throw (error as Error);
    }
  }

  private publishPreferences(): void {
    if (this.disposed) { return; }
    const started = Date.now();
    const json = pawsUi.queryUiPreferences();
    const elapsed = Date.now() - started;
    if (elapsed >= 16) { hilog.warn(0x0000, 'PawsNsh', 'preferences query blocked host for %{public}d ms', elapsed); }
    if (json === this.publishedPreferences) { return; }
    const preferences = JSON.parse(json) as UiPreferences;
    this.host.applyPreferences(preferences);
    this.publishedPreferences = json;
    this.emit('preferences', preferences);
  }

  private emitChanged(change: StateChanged): void {
    const previous = this.pendingChanges;
    const versions = new Map<string, DomainRevision>();
    if (previous?.generation === change.generation) {
      for (const item of previous.changes) { versions.set(`${item.domain}\u0000${item.groupKey ?? ''}`, item); }
    }
    for (const item of change.changes) { versions.set(`${item.domain}\u0000${item.groupKey ?? ''}`, item); }
    const pending: StateChanged = { generation: change.generation, revision: change.revision, changes: Array.from(versions.values()) };
    this.pendingChanges = pending;
    if (pending.changes.length) { this.emit('changed', pending); }
    this.pendingChanges = undefined;
  }

  private publishChanges(criticalOnly: boolean = false): StateObservation {
    // Only metadata is copied synchronously. Domain reads are explicit async
    // queries, so a pending rules load cannot hold up Stop or cancellation.
    const observation = JSON.parse(this.session.commands.observeState(criticalOnly)) as StateObservation;
    this.emitChanged(observation.change);
    return observation;
  }

  private async observe(): Promise<void> {
    if (this.disposed) { return; }
    try {
      this.publishPreferences();
      let revision = String(this.publishChanges().change.revision);
      while (!this.disposed) {
        const observed = await pawsUi.waitForUiRevision(revision);
        if (!this.disposed) {
          this.publishPreferences();
          if (observed !== revision) { revision = String(this.publishChanges().change.revision); }
        }
      }
    } catch (error) {
      if (!this.disposed) {
        this.emit('result', { id: 0, ok: false, error: describeUserError(error) } as UiResult);
        this.retryTimer = setTimeout((): void => { void this.observe(); }, 1000);
      }
    }
  }
}
