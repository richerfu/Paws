export const startTUN: (
  protect: (fd: number) => Promise<void>,
  resolveUid: (protocol: number, source: string, target: string) => number,
  resolvePackage: (uid: number) => string,
  fd: number,
  stack: string,
  addresses: string,
  dnsAddresses: string,
) => boolean;
export const quickSetup: (
  onResult: (data: string) => void,
  initParams: string,
  setupParams: string,
) => void;
export const getTotalTraffic: (onlyStatisticsProxy: boolean) => string;
export const getTraffic: (onlyStatisticsProxy: boolean) => string;
export const stopTun: () => void;
export const forceGC: () => void;
