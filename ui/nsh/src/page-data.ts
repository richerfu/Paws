import type { Page } from "./components";
import type { DomainInput } from "./model";

/** Only the foreground route owns these detail subscriptions. Stack's retained
 * pages keep their drafts but cannot keep background query streams alive. */
export function pageDomains(page: Page): DomainInput[] {
  switch (page) {
    case "dashboard":
      return ["summary"];
    case "profiles":
    case "profile-detail":
    case "editor":
      return ["profiles"];
    case "proxies":
    case "proxy-group":
      return ["proxies"];
    case "traffic":
    case "dns-diagnostics":
      return ["traffic", "dns"];
    case "resources":
      return ["rules", "providers", "geodata", "summary"];
    case "connections":
      return ["connections"];
    case "requests":
      return ["requests"];
    case "activity-detail":
      return ["connections", "requests"];
    case "logs":
      return ["logs"];
    case "diagnostics":
      return ["summary", "dns"];
    case "advanced-diagnostics":
    case "settings":
      return ["summary"];
    case "about":
    case "privacy":
      return ["about"];
    default:
      return [];
  }
}
