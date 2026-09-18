import '@ohos.net.vpnExtension';

declare module '@ohos.net.vpnExtension' {
  export function updateVpnAuthorizedState(bundleName: string): boolean;
}
