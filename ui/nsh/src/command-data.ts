import type { DomainInput } from "./model";

/** Writes require the data their controls were based on. Stop, recovery,
 * cancellation and independent read/platform operations remain available. */
export function commandDomains(command: string): DomainInput[] {
  if (["vpn.start", "vpn.restart", "mode.set"].includes(command)) return ["config", "status"];
  if (
    [
      "profiles.activate",
      "profiles.delete",
      "profiles.refresh",
      "profiles.refreshAll",
      "profiles.restore",
      "profiles.subscription",
      "profiles.save",
    ].includes(command)
  )
    return ["config", "profiles"];
  if (
    command === "profiles.importUrl" ||
    command === "profiles.importContent" ||
    command.startsWith("settings.")
  )
    return ["config"];
  if (command.startsWith("rules.") && !["rules.lookup"].includes(command))
    return ["config", "rules"];
  if (["providers.refresh", "providers.healthcheck", "providers.testProxy"].includes(command))
    return ["config", "providers"];
  if (command === "diagnostics.serviceDns") return ["config", "summary"];
  if (command === "requests.clear") return ["requests"];
  if (command === "dns.flush" || command === "dns.fakeIpFlush") return ["status", "dns"];
  return [];
}
