import { cleanup, effect, untracked } from "nsh";
import { CoreClient } from "../src/client";
import { domainKey } from "../src/domain-store";
import {
  effectiveGroupLeaf,
  groupMembers,
  primarySelectedGroupLeaf,
  proxyGroupSummary,
  selectedMember,
} from "../src/proxy-groups";
import type {
  CoreRequest,
  CoreResult,
  DomainData,
  DomainInterest,
  DomainName,
  DomainQueryResult,
  StateBarrier,
  StateChanged,
  UiState,
} from "../src/model";

/** Synthetic server data only. All subscription/query reduction is production CoreClient. */
export function fixtureDomain(state: UiState, interest: DomainInterest): DomainData[DomainName] {
  switch (interest.domain) {
    case "config":
      return {
        activeProfile: state.activeProfile,
        mode: state.mode,
        vpnOptions: state.vpnOptions,
        controllerAccess: state.controllerAccess,
        networkPorts: state.networkPorts,
      };
    case "status":
      return {
        vpnSessionId: state.vpnSessionId,
        vpnLifecycle: state.vpnLifecycle,
        engineLoaded: state.engineLoaded,
        activeEngine: state.activeEngine,
        running: state.running,
        vpnRunning: state.vpnRunning,
        networkProtected: state.networkProtected,
        networkProtectError: state.networkProtectError,
        controllerRunning: state.controllerRunning,
        controllerAddr: state.controllerAddr,
        exitLocation: state.exitLocation,
      };
    case "summary": {
      const counts = proxyGroupSummary(state.proxyGroups);
      return {
        appVersion: state.about.appVersion,
        profileCount: state.profiles.length,
        activeProfileName:
          state.profiles.find((row) => row.id === state.activeProfile)?.name || null,
        globalNodeCount: counts.global,
        nodeCount: counts.members,
        groupCount: counts.groups,
        primarySelectedNode: primarySelectedGroupLeaf(state.proxyGroups) || null,
        globalSelectedNode: effectiveGroupLeaf(state.proxyGroups, "GLOBAL") || null,
        latestRuleNode: null,
        groups: state.proxyGroups.map((group) => ({
          name: group.name,
          groupType: group.groupType,
          selected: selectedMember(group) || null,
          fixed: group.fixed ?? null,
          memberCount: groupMembers(state.proxyGroups, group).length,
          effectiveSelectedNode: effectiveGroupLeaf(state.proxyGroups, group.name) || null,
        })),
      };
    }
    case "profiles":
      return { profiles: state.profiles };
    case "proxies":
      return {
        proxyGroups:
          interest.groupKey === undefined
            ? state.proxyGroups
            : state.proxyGroups.filter((group) => group.name === interest.groupKey),
      };
    case "rules":
      return { rules: state.rules };
    case "providers":
      return { providers: state.providers };
    case "geodata":
      return { geodata: state.geodata };
    case "traffic": {
      const profile = state.profiles.find((row) => row.id === state.activeProfile);
      return {
        traffic: state.traffic,
        trafficHistory: state.trafficHistory,
        controllerDiagnostics: state.controllerDiagnostics,
        activeProfileUsage: profile
          ? { uploadBytes: profile.uploadBytes, downloadBytes: profile.downloadBytes }
          : null,
        connectionSummary: {
          count: state.connections.length,
          uploadBytes: state.connections.reduce((sum, row) => sum + row.uploadBytes, 0),
          downloadBytes: state.connections.reduce((sum, row) => sum + row.downloadBytes, 0),
          recent: state.connections.slice(0, 5),
        },
      };
    }
    case "connections":
      return { connections: state.connections };
    case "requests":
      return { requestHistory: state.requestHistory };
    case "logs":
      return { logs: state.logs };
    case "dns":
      return { dns: state.dns };
    case "about":
      return { about: state.about };
  }
}

export class DomainHost {
  readonly seen: CoreRequest[] = [];
  generation = 0;
  interests: DomainInterest[] = [];
  private readonly targets = new Map<string, number>();
  onQuery?: (request: CoreRequest, reply: DomainQueryResult) => boolean;
  onSubscribe?: (request: CoreRequest, reply: StateChanged) => boolean;
  onCommand?: (request: CoreRequest) => void;

  constructor(
    readonly client: CoreClient,
    readonly state: () => UiState,
  ) {
    effect(() => {
      const request = client.request();
      if (!request) return;
      untracked(() => {
        this.seen.push(request);
        client.acceptReceived(request.id);
        if (request.command === "state.subscribe") {
          this.generation = request.payload.generation as number;
          this.interests = request.payload.domains as DomainInterest[];
          this.targets.clear();
          for (const key of this.interests) this.targets.set(domainKey(key), this.state().revision);
          const change: StateChanged = {
            generation: this.generation,
            revision: this.state().revision,
            changes: this.barrier().required,
          };
          if (!this.onSubscribe?.(request, change)) this.reply(request, change);
        } else if (request.command === "state.query") {
          const interest = {
            domain: request.payload.domain as DomainName,
            ...(request.payload.groupKey !== undefined
              ? { groupKey: request.payload.groupKey as string }
              : {}),
          };
          const result: DomainQueryResult = {
            ...interest,
            generation: request.payload.generation as number,
            revision: this.state().revision,
            configRevision: this.state().configRevision,
            data: fixtureDomain(this.state(), interest),
          };
          if (!this.onQuery?.(request, result))
            this.reply(request, JSON.parse(JSON.stringify(result)));
        } else this.onCommand?.(request);
      });
    });
    cleanup(() => client.dispose());
  }
  connect(): void {
    this.client.domains.connect();
  }
  change(domains: DomainName[]): void {
    const changes = this.interests
      .filter((interest) => domains.includes(interest.domain))
      .map((interest) => ({ ...interest, revision: this.state().revision }));
    for (const change of changes) this.targets.set(domainKey(change), change.revision);
    this.client.domains.changed({
      generation: this.generation,
      revision: this.state().revision,
      changes,
    });
  }
  barrier(): StateBarrier {
    return {
      generation: this.generation,
      required: this.interests.map((interest) => ({
        ...interest,
        revision: this.targets.get(domainKey(interest)) ?? this.state().revision,
      })),
    };
  }
  reply(request: CoreRequest, data: unknown, changes?: DomainName[]): void {
    if (changes) this.change(changes);
    this.client.acceptResult({
      id: request.id,
      ok: true,
      data,
      ...(request.command.startsWith("state.") ? {} : { barrier: this.barrier() }),
    });
  }
  result(value: CoreResult): void {
    this.client.acceptResult(value);
  }
}
