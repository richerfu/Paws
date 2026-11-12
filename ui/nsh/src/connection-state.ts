import type { UiState } from "./model";

export function connectionState(
  state: Pick<UiState, "vpnLifecycle" | "vpnRunning" | "networkProtected">,
  operation?: "start" | "stop" | "restart",
) {
  if (operation === "stop") return "stopping";
  if (state.vpnLifecycle === "starting" || operation === "start" || operation === "restart")
    return "connecting";
  if (state.vpnLifecycle === "failed" || state.vpnLifecycle === "protect-failed") return "failed";
  if (state.vpnRunning && state.vpnLifecycle === "connected" && state.networkProtected)
    return "protected";
  return state.vpnRunning ? "unprotected" : "idle";
}
