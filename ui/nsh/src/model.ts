// Query payloads carry one requested domain. Domain decisions stay in Rust.
export type Mode = "rule" | "global" | "direct";
export type Engine = "auto" | "meow" | "mihomo" | "singbox" | "sing-box";
export type Preferences = {
  language: "system" | "zh_cn" | "en";
  theme: "system" | "light" | "dark";
  advanced: boolean;
  systemLocale: string;
};
export type Profile = {
  id: string;
  name: string;
  source: string;
  active: boolean;
  updatedAt?: string;
  lastRefreshAt?: string;
  lastRefreshError?: string;
  subscriptionUrl?: string;
  ruleCount: number;
  hasBackup: boolean;
  uploadBytes: number;
  downloadBytes: number;
  subscriptionUserInfo?: {
    uploadBytes: number;
    downloadBytes: number;
    totalBytes?: number;
    expireAt?: string;
  };
  subscriptionMetadata?: { title?: string; webPageUrl?: string; supportUrl?: string };
  nextRefreshAt?: string;
  refreshDue: boolean;
  rawYamlPath?: string;
  runtimeYamlPath?: string;
};
export type Proxy = { name: string; proxyType: string; delayMs?: number; selected: boolean };
export type ProxyGroup = {
  name: string;
  groupType: string;
  selected?: string | null;
  fixed?: string | null;
  proxies: Proxy[];
};
export type Rule = {
  id: string;
  profileId: string;
  line: string;
  enabled: boolean;
  order: number;
  source: string;
};
export type Provider = {
  name: string;
  providerType: string;
  path?: string;
  url?: string;
  cacheExists: boolean;
  cacheBytes?: number;
  lastRefreshAt?: string;
  lastRefreshError?: string;
  vehicleType?: string;
  intervalSeconds?: number;
  filter?: string;
  excludeFilter?: string;
  behavior?: string;
  format?: string;
  healthCheckEnabled?: boolean;
  healthCheckUrl?: string;
  healthCheckIntervalSeconds?: number;
  staleCacheAvailable?: boolean;
  members: { name: string; proxyType: string; alive?: boolean | null; delayMs?: number }[];
};
export type Connection = {
  id: string;
  host: string;
  domain: string;
  destinationIp: string;
  destinationPort: number;
  network: string;
  domainSource?: string;
  rule: string;
  proxy: string;
  chains?: string[];
  startedAt?: string;
  rulePayload?: string;
  uploadBytes: number;
  downloadBytes: number;
  active?: boolean;
  updatedAt?: string;
};
export type VpnOptions = {
  engine: Engine;
  stack: string;
  mtu: number;
  ipv6: boolean;
  systemProxy: boolean;
  dnsHijacking: boolean;
  allowBypass: boolean;
  trustedApplications?: string[];
  blockedApplications?: string[];
  addresses: string[];
  routes: string[];
  dnsAddresses: string[];
  dnsServers: string[];
  dnsFallbacks?: string[];
  dnsNameserverPolicy?: Record<string, string[]>;
};
export type NetworkPorts = {
  mixedPort: number;
  controllerPort: number;
  mixedEnabled: boolean;
  controllerEnabled: boolean;
};
export type ControllerAccess = { allowLan: boolean; secret?: string };
export type UiState = {
  revision: number;
  configRevision: number;
  vpnSessionId?: string | null;
  vpnLifecycle:
    | "stopped"
    | "engine-loaded"
    | "starting"
    | "connected"
    | "protect-failed"
    | "failed";
  engineLoaded: boolean;
  activeEngine?: Engine | null;
  running: boolean;
  vpnRunning: boolean;
  networkProtected: boolean;
  networkProtectError?: string | null;
  controllerRunning: boolean;
  controllerAddr?: string | null;
  controllerDiagnostics: {
    memoryAvailable?: boolean;
    memoryLimitAvailable?: boolean;
    memoryScope?: "processResident" | "engineHeap" | null;
    memoryError?: string | null;
    memoryInUseBytes: number;
    memoryLimitBytes: number;
    configSyncCount: number;
    lastConfigSyncAt?: string;
    lastConfigSyncError?: string;
  };
  controllerAccess: ControllerAccess;
  networkPorts: NetworkPorts;
  activeProfile?: string | null;
  mode: Mode;
  traffic: {
    uploadBytes: number;
    downloadBytes: number;
    uploadSpeed: number;
    downloadSpeed: number;
    tunUploadBytes: number;
    tunDownloadBytes: number;
  };
  trafficHistory: { downloadSpeed: number; uploadSpeed: number }[];
  dns: {
    hijacking?: boolean;
    nameserverPolicy?: Record<string, string[]>;
    tunAddresses?: string[];
    model: string;
    listen: string;
    upstreams: string[];
    fallbacks: string[];
    statisticsAvailable: boolean;
    statisticsError?: string | null;
    configurationError?: string | null;
    handledPackets: number;
    cacheHits: number;
    cacheMisses: number;
    recentQueries: { name: string; recordType: string; count: number }[];
  };
  vpnOptions: VpnOptions;
  exitLocation: {
    ip: string;
    country: string;
    countryCode: string;
    provider?: string;
    error?: string;
  };
  proxyGroups: ProxyGroup[];
  profiles: Profile[];
  rules: Rule[];
  providers: Provider[];
  geodata: { name: string; path: string; exists: boolean; bytes?: number; updatedAt?: string }[];
  logs: { level: string; message: string; timestamp: string }[];
  connections: Connection[];
  requestHistory: Connection[];
  about: {
    appVersion: string;
    coreVersion: string;
    meowRsVersion: string;
    mihomoVersion: string;
    singBoxVersion: string;
    rustVersion: string;
    uiRuntime: string;
    uiRuntimeVersion: string;
    privacySummary: string[];
    exitIpServices: { name: string; documentationUrl: string }[];
  };
};
export type ProxyGroupHeader = Omit<ProxyGroup, "proxies"> & {
  memberCount: number;
  effectiveSelectedNode: string | null;
};
export type HomeSummary = {
  appVersion: string;
  profileCount: number;
  activeProfileName: string | null;
  globalNodeCount: number;
  nodeCount: number;
  groupCount: number;
  primarySelectedNode: string | null;
  globalSelectedNode: string | null;
  latestRuleNode: string | null;
  groups: ProxyGroupHeader[];
};
export type TrafficDetail = Pick<
  UiState,
  "traffic" | "trafficHistory" | "controllerDiagnostics"
> & {
  activeProfileUsage: { uploadBytes: number; downloadBytes: number } | null;
  connectionSummaryAvailable?: boolean;
  connectionSummary: {
    count: number;
    uploadBytes: number;
    downloadBytes: number;
    recent: Connection[];
  };
};
export type DomainData = {
  config: Pick<
    UiState,
    "activeProfile" | "mode" | "vpnOptions" | "controllerAccess" | "networkPorts"
  >;
  status: Pick<
    UiState,
    | "vpnSessionId"
    | "vpnLifecycle"
    | "engineLoaded"
    | "activeEngine"
    | "running"
    | "vpnRunning"
    | "networkProtected"
    | "networkProtectError"
    | "controllerRunning"
    | "controllerAddr"
    | "exitLocation"
  >;
  summary: HomeSummary;
  profiles: Pick<UiState, "profiles">;
  proxies: Pick<UiState, "proxyGroups">;
  rules: Pick<UiState, "rules"> & {
    runtimeRules?: { ruleType: string; payload: string; target: string }[] | null;
  };
  providers: Pick<UiState, "providers"> & { runtimeAvailability?: { name: string; providerType: string; available: boolean; reason: string | null; ruleCount: number | null; updatedAt: string | null }[] };
  geodata: Pick<UiState, "geodata">;
  traffic: TrafficDetail;
  connections: Pick<UiState, "connections">;
  requests: Pick<UiState, "requestHistory">;
  logs: Pick<UiState, "logs"> & { recordingError?: string | null };
  dns: Pick<UiState, "dns">;
  about: Pick<UiState, "about">;
};
export type DomainName = keyof DomainData;
export type DomainInterest = { domain: DomainName; groupKey?: string };
export type DomainInput = DomainInterest | DomainName;
export type DomainRevision = DomainInterest & { revision: number };
export type StateChanged = { generation: number; revision: number; changes: DomainRevision[] };
export type StateBarrier = { generation: number; required: DomainRevision[] };
export type DomainQueryResult<D extends DomainName = DomainName> = DomainInterest & {
  domain: D;
  generation: number;
  revision: number;
  configRevision: number;
  data: DomainData[D];
  observation?: { engine: Engine | null; session: string | null; issues: { field: string; reason: string }[] };
};
export type CoreRequest = { id: number; command: string; payload: Record<string, unknown> };
export type CoreResult = {
  id: number;
  ok: boolean;
  error?: string;
  data?: unknown;
  barrier?: StateBarrier;
  stateRefreshError?: string;
};
export type LogRecording = {
  enabled: boolean;
  archives?: {
    fileName: string;
    bytes: number;
    date?: string;
    active?: boolean;
    updatedAt?: string;
  }[];
  lastError?: string;
};

export type VpnConfirmation = {
  requestId: number;
  status: "pending" | "succeeded" | "failed" | "unavailable";
  result: boolean;
  error: string;
};
