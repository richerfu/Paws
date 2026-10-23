import { fileIo } from '@kit.CoreFileKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { engineParityHome } from './EngineParityPaths';
import { UiRequest as Request, UiResult as Result, DomainRevision as Revision,
  StateBarrier as Barrier, StateChanged as Changed } from '../components/PawsCoreGateway';

interface QueryData {
  domain?: string; generation?: number; revision?: number; configRevision?: number;
  data?: DomainData;
  observation?: ObservationData;
}
interface QueryPayload { domain?: string; groupKey?: string; }
interface ObservationIssue { field: string; }
interface ObservationData { engine?: string; session?: string; issues?: ObservationIssue[]; }
interface StatusData { activeEngine?: string; vpnSessionId?: string; vpnRunning?: boolean; }
interface Counters { uploadBytes?: number; downloadBytes?: number; }
interface DnsCounters { statisticsAvailable?: boolean; handledPackets?: number; cacheHits?: number; cacheMisses?: number; }
interface MemoryCounters { memoryAvailable?: boolean; memoryInUseBytes?: number; }
interface DomainData extends StatusData {
  profiles?: Object[]; proxyGroups?: Object[]; rules?: Object[]; runtimeRules?: Object[];
  providers?: Object[]; geodata?: Object[]; connections?: Object[]; requestHistory?: Object[]; logs?: Object[];
  traffic?: Counters; dns?: DnsCounters; controllerDiagnostics?: MemoryCounters;
}
interface TraceRecord {
  seq: number; run: number; time: number; event: string;
  id?: number; command?: string; ok?: boolean; errorPresent?: boolean;
  stateRefreshError?: boolean; chars?: number; generation?: number;
  revision?: number; configRevision?: number; domain?: string;
  changes?: Revision[]; barrier?: Barrier; grouped?: boolean;
  activeEngine?: string; owner?: string; vpnRunning?: boolean;
  code?: number; bytes?: number; queryType?: number; answerCode?: number;
  elapsedMs?: number; caseId?: string;
  resultId?: number; barrierValid?: boolean; changesTruncated?: boolean;
  observedEngine?: string; observedOwner?: string; issueFields?: string[];
  items?: number; runtimeItems?: number; uploadBytes?: number; downloadBytes?: number;
  statisticsAvailable?: boolean; handledPackets?: number; cacheHits?: number; cacheMisses?: number;
  memoryAvailable?: boolean; memoryBytes?: number;
  answerCount?: number;
  errorUtf8Bytes?: number; firstLineBytes?: number; hasNewline?: boolean;
  knownStaticErrorCategory?: string;
}
const COMMANDS: string[] = __COMMAND_LIST__;
const DOMAINS: string[] = ['config', 'status', 'summary', 'profiles', 'proxies', 'rules', 'providers',
  'geodata', 'traffic', 'connections', 'requests', 'logs', 'dns', 'about'];
let path: string = '';
let sequence: number = 0;
let epoch: number = 0;
let stopped: boolean = false;

function number(value: number | undefined): number | undefined {
  return value !== undefined && Number.isSafeInteger(value) && value >= 0 ? value : undefined;
}
function revisions(values: Revision[]): Revision[] {
  if (!Array.isArray(values)) { return []; }
  return values.slice(0, 64).filter(value => DOMAINS.includes(value.domain) && number(value.revision) !== undefined)
    .map((value: Revision): Revision => ({ domain: value.domain, revision: value.revision }));
}
function append(value: TraceRecord): void {
  if (!path || stopped) { return; }
  let file: fileIo.File | undefined;
  try {
    let size: number = 0;
    try { size = fileIo.statSync(path).size; } catch (_) {}
    if (sequence >= 2048 || size >= 1024 * 1024) {
      value = { seq: sequence + 1, run: epoch, time: Date.now(), event: 'trace-truncated' };
      stopped = true;
    }
    file = fileIo.openSync(path, fileIo.OpenMode.CREATE | fileIo.OpenMode.WRITE_ONLY | fileIo.OpenMode.APPEND);
    fileIo.writeSync(file.fd, JSON.stringify(value) + '\n');
    sequence++;
  } catch (_) {
    stopped = true;
    hilog.error(0, 'PawsEngineQA', 'bounded trace unavailable');
  } finally { if (file !== undefined) { fileIo.closeSync(file); } }
}
function record(event: string): TraceRecord { return { seq: sequence + 1, run: epoch, time: Date.now(), event }; }

function utf8Bytes(text: string): number {
  let length: number = 0;
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i);
    if (code < 128) length++;
    else if (code < 2048) length += 2;
    else if (code >= 0xd800 && code <= 0xdbff && i + 1 < text.length &&
      text.charCodeAt(i + 1) >= 0xdc00 && text.charCodeAt(i + 1) <= 0xdfff) { length += 4; i++; }
    else length += 3;
  }
  return length;
}
function queryMetadata(value: TraceRecord, request: Request): void {
  if (request.command !== 'state.query' || !request.payload) return;
  const payload = request.payload as QueryPayload;
  if (typeof payload.domain === 'string' && DOMAINS.includes(payload.domain)) value.domain = payload.domain;
  value.grouped = typeof payload.groupKey === 'string' && payload.groupKey.length > 0;
}
function errorCategory(text: string): string {
  // Only fixed prefixes and fixed public categories leave this function.
  let message = text;
  for (let i = 0; i < 4; i++) {
    if (message.startsWith('Error: ')) message = message.slice(7);
    else if (message.startsWith('core error: ')) message = message.slice(12);
    else break;
  }
  if (message.startsWith('Public QA multiline failure line ')) return 'qa-injected-multiline';
  if (message.startsWith('公开错误👩‍💻é')) return 'qa-injected-unbroken';
  if (['native management result belongs to an obsolete session or configuration epoch',
    'native management configuration epoch was replaced',
    'runtime domain query belongs to an obsolete engine session',
    'engine observation belongs to an obsolete session'].some(value => message.includes(value))) return 'epoch-obsolete';
  if (['runtime domain query subscription was closed', 'runtime domain query subscription was superseded',
    'Observation subscription was superseded or the domain is not subscribed',
    'Observation owner was closed', 'A query for this observation domain is already in flight']
    .some(value => message.includes(value))) return 'subscription-superseded';
  if (/^stale configuration revision: expected [0-9]+, current [0-9]+$/.test(message)) return 'revision-stale';
  if (['engine observation timed out after 8 seconds', 'runtime domain query exceeded its 8 second total budget',
    'engine observation exceeded the domain query deadline'].some(value => message.includes(value))) return 'deadline';
  if (/^(SingBox|Mihomo) (runtime observation|proxy groups) failed: (controller request failed|controller HTTP [0-9]{3}|invalid observation schema|invalid controller response|response missing proxies|invalid group members|group missing current selection|invalid member name)(:|$)/.test(message)) return 'native-controller';
  if (message.includes('proxy group failed: group identity mismatched')) return 'group-identity';
  return 'unclassified';
}

export function startEngineParityTrace(filesDir: string): void {
  path = `${engineParityHome(filesDir)}/gateway-trace.jsonl`;
  epoch = Date.now(); sequence = 0; stopped = false;
  append(record('trace-start'));
}
export function closeEngineParityTrace(): void {
  append(record('trace-close')); path = '';
}
export function engineParityTraceActive(): boolean { return path.length > 0 && !stopped; }
export function traceParityReceived(request: Request): void {
  // Command identifiers only. Payload, URLs, errors and result objects are never serialized.
  if (!COMMANDS.includes(request.command)) { return; }
  const value = record('received'); value.id = number(request.id); value.command = request.command;
  queryMetadata(value, request);
  append(value);
}
export function traceParityResult(request: Request, result: Result, chars: number): void {
  if (!COMMANDS.includes(request.command)) { return; }
  const value = record('result');
  value.id = number(request.id); value.resultId = number(result.id); value.command = request.command;
  value.ok = result.ok === true; value.errorPresent = typeof result.error === 'string' && result.error.length > 0;
  value.stateRefreshError = typeof result.stateRefreshError === 'string' && result.stateRefreshError.length > 0;
  value.chars = number(chars);
  queryMetadata(value, request);
  const error = typeof result.error === 'string' && result.error.length > 0 ? result.error : result.stateRefreshError;
  if (typeof error === 'string' && error.length > 0) {
    value.errorUtf8Bytes = utf8Bytes(error);
    const newline = error.search(/[\r\n]/);
    value.hasNewline = newline >= 0;
    value.firstLineBytes = utf8Bytes(newline < 0 ? error : error.slice(0, newline));
    value.knownStaticErrorCategory = errorCategory(error);
  }
  if (result.barrier) {
    const generation = number(result.barrier.generation);
    value.barrierValid = generation !== undefined && Array.isArray(result.barrier.required) && result.barrier.required.length <= 64 &&
      result.barrier.required.every(item => DOMAINS.includes(item.domain) && number(item.revision) !== undefined);
    if (value.barrierValid && generation !== undefined) value.barrier = { generation, required: revisions(result.barrier.required) };
  }
  if (request.command === 'state.query' && result.ok && result.data) {
    const query = result.data as QueryData;
    if (query.domain && DOMAINS.includes(query.domain)) {
      value.domain = query.domain; value.generation = number(query.generation);
      value.revision = number(query.revision); value.configRevision = number(query.configRevision);
      const observation = query.observation;
      if (observation) {
        if (['meow', 'mihomo', 'sing-box', 'singbox'].includes(observation.engine ?? '')) value.observedEngine = observation.engine;
        if (observation.session && /^[a-zA-Z0-9_-]{1,96}$/.test(observation.session)) value.observedOwner = observation.session;
        if (Array.isArray(observation.issues)) value.issueFields = observation.issues.slice(0, 16)
          .map(item => item.field).filter(field => ['memory', 'connectionSummary', 'requestHistory', 'logs', 'dnsConfiguration', 'dnsStatistics', 'groups', 'providers', 'runtimeRules'].includes(field));
      }
      if (query.domain === 'status' && query.data) {
        if (['meow', 'mihomo', 'sing-box', 'singbox'].includes(query.data.activeEngine ?? '')) value.activeEngine = query.data.activeEngine;
        const owner = query.data.vpnSessionId;
        if (owner && /^[a-zA-Z0-9_-]{1,96}$/.test(owner)) value.owner = owner;
        if (typeof query.data.vpnRunning === 'boolean') value.vpnRunning = query.data.vpnRunning;
      }
      const data = query.data;
      if (data) {
        let collection: Object[] | undefined;
        switch (query.domain) {
          case 'profiles': collection = data.profiles; break;
          case 'proxies': collection = data.proxyGroups; break;
          case 'rules': collection = data.rules; if (Array.isArray(data.runtimeRules)) value.runtimeItems = number(data.runtimeRules.length); break;
          case 'providers': collection = data.providers; break;
          case 'geodata': collection = data.geodata; break;
          case 'connections': collection = data.connections; break;
          case 'requests': collection = data.requestHistory; break;
          case 'logs': collection = data.logs; break;
        }
        if (Array.isArray(collection)) value.items = number(collection.length);
        if (query.domain === 'traffic') {
          if (data.traffic) { value.uploadBytes = number(data.traffic.uploadBytes); value.downloadBytes = number(data.traffic.downloadBytes); }
          if (data.controllerDiagnostics) {
            value.memoryAvailable = data.controllerDiagnostics.memoryAvailable === true;
            if (value.memoryAvailable) value.memoryBytes = number(data.controllerDiagnostics.memoryInUseBytes);
          }
        }
        if (query.domain === 'dns' && data.dns) {
          value.statisticsAvailable = data.dns.statisticsAvailable === true;
          if (value.statisticsAvailable) {
            value.handledPackets = number(data.dns.handledPackets); value.cacheHits = number(data.dns.cacheHits); value.cacheMisses = number(data.dns.cacheMisses);
          }
        }
      }
    }
  }
  append(value);
}
export function traceParityChanged(change: Changed): void {
  const value = record('changed'); value.generation = number(change.generation);
  value.revision = number(change.revision);
  if (Array.isArray(change.changes)) {
    value.changes = revisions(change.changes); value.changesTruncated = change.changes.length > 64;
  } else value.errorPresent = true;
  append(value);
}
export function traceParityProbe(event: string, caseId: string, ok: boolean, code: number,
  bytes: number, elapsedMs: number, queryType: number = 0, answerCode: number = 0, answerCount: number = 0): void {
  const value = record(event); value.caseId = caseId; value.ok = ok;
  value.code = number(code); value.bytes = number(bytes); value.elapsedMs = number(elapsedMs);
  value.queryType = number(queryType); value.answerCode = number(answerCode);
  if (queryType > 0) value.answerCount = number(answerCount);
  append(value);
}
