import { Want } from '@kit.AbilityKit';

export const PAWS_BUNDLE_NAME: string = 'com.richerfu.paws';
export const PAWS_MODULE_NAME: string = 'entry';
export const PAWS_ENTRY_ABILITY: string = 'EntryAbility';
export const PAWS_VPN_ABILITY: string = 'PawsVpnExtensionAbility';

let pawsBundleName: string = PAWS_BUNDLE_NAME;

export function setPawsBundleName(bundleName?: string): void {
  const value = bundleName?.trim() ?? '';
  pawsBundleName = value.length > 0 ? value : PAWS_BUNDLE_NAME;
}

export function currentPawsBundleName(): string {
  return pawsBundleName;
}

export class NetAddress {
  public address: string;
  public family: number;
  public port: number;

  constructor(address: string, family: number, port: number = 0) {
    this.address = address;
    this.family = family;
    this.port = port;
  }
}

export class LinkAddress {
  public address: NetAddress;
  public prefixLength: number;

  constructor(address: NetAddress, prefixLength: number) {
    this.address = address;
    this.prefixLength = prefixLength;
  }
}

export class RouteInfo {
  public interface: string;
  public destination: LinkAddress;
  public gateway: NetAddress;
  public hasGateway: boolean;
  public isDefaultRoute: boolean;

  constructor(destination: LinkAddress, isDefaultRoute: boolean) {
    this.interface = 'vpn-tun';
    this.destination = destination;
    this.gateway = new NetAddress(destination.address.family === 2 ? '::' : '0.0.0.0', destination.address.family);
    this.hasGateway = false;
    this.isDefaultRoute = isDefaultRoute;
  }
}

export interface VpnOptions {
  addresses?: string[];
  mtu?: number;
  ipv6?: boolean;
  systemProxy?: boolean;
  dnsHijacking?: boolean;
  allowBypass?: boolean;
  stack?: string;
  routes?: string[];
  dnsAddresses?: string[];
  dnsServers?: string[];
  dnsFallbacks?: string[];
  dnsNameserverPolicy?: Record<string, string[]>;
}

const DEFAULT_DNS_NAMESERVER_POLICY: Record<string, string[]> = {
  'geosite:cn': ['223.5.5.5', '119.29.29.29'],
  'geosite:geolocation-!cn': ['1.1.1.1', '8.8.8.8'],
};

const DEFAULT_OPTIONS: VpnOptions = {
  addresses: ['172.19.0.1/30'],
  mtu: 1500,
  ipv6: false,
  systemProxy: false,
  dnsHijacking: true,
  allowBypass: false,
  stack: 'netstack-smoltcp',
  routes: ['0.0.0.0/0'],
  dnsAddresses: ['172.19.0.2'],
  dnsServers: ['223.5.5.5', '119.29.29.29'],
  dnsFallbacks: ['1.1.1.1', '8.8.8.8'],
  dnsNameserverPolicy: DEFAULT_DNS_NAMESERVER_POLICY,
};

export const DEFAULT_OPTIONS_JSON: string = JSON.stringify(DEFAULT_OPTIONS);

export class PawsVpnConfig {
  public addresses: LinkAddress[];
  public routes: RouteInfo[];
  public dnsAddresses: string[];
  public mtu: number;
  public isIPv4Accepted: boolean;
  public isIPv6Accepted: boolean;
  public isBlocking: boolean;

  constructor(options: VpnOptions) {
    const addresses = options.addresses?.length ? options.addresses : ['172.19.0.1/30'];
    const routes = options.routes?.length ? options.routes : ['0.0.0.0/0'];
    this.addresses = addresses.map((value: string) => parseLinkAddress(value, addressDefaultPrefix(value)));
    this.routes = routes.map((value: string) => {
      const route = parseLinkAddress(value, routeDefaultPrefix(value));
      return new RouteInfo(route, route.prefixLength === 0);
    });
    this.dnsAddresses = options.dnsAddresses?.length ? options.dnsAddresses : ['172.19.0.2'];
    this.mtu = (options.mtu ?? 1500) > 0 ? (options.mtu ?? 1500) : 1500;
    this.isIPv4Accepted = this.addresses.some((address: LinkAddress) => address.address.family === 1) ||
      this.routes.some((route: RouteInfo) => route.destination.address.family === 1);
    this.isIPv6Accepted = (options.ipv6 ?? false) ||
      this.addresses.some((address: LinkAddress) => address.address.family === 2) ||
      this.routes.some((route: RouteInfo) => route.destination.address.family === 2);
    this.isBlocking = false;
  }
}

const PAWS_ASHMEM_FD_KEY = 'pawsPlatformAshmem';
const PAWS_NOTIFICATION_FD_KEY = 'pawsPlatformNotification';
const PAWS_START_ATTEMPT_ID_KEY = 'pawsStartAttemptId';
const PAWS_SYSTEM_PARAMETERS_KEY = 'myParams';

export interface PawsPlatformSharedMemoryFds {
  ashmemFd: number;
  notificationFd: number;
}

class PawsWantFileDescriptor {
  public type: string = 'FD';
  public value: number;

  constructor(value: number) {
    this.value = value;
  }
}

interface PawsVpnWantParameters extends Record<string, Object> {
  optionsJson: string;
}

export function buildVpnWant(
  optionsJson?: string,
  sharedMemory?: PawsPlatformSharedMemoryFds,
  startAttemptId?: string,
): Want {
  const parameters: PawsVpnWantParameters = {
    optionsJson: optionsJson ?? '',
  };
  if (sharedMemory) {
    parameters[PAWS_ASHMEM_FD_KEY] = new PawsWantFileDescriptor(sharedMemory.ashmemFd);
    parameters[PAWS_NOTIFICATION_FD_KEY] = new PawsWantFileDescriptor(sharedMemory.notificationFd);
  }
  if (startAttemptId && startAttemptId.length > 0) {
    parameters[PAWS_START_ATTEMPT_ID_KEY] = startAttemptId;
  }
  return {
    deviceId: '',
    bundleName: currentPawsBundleName(),
    moduleName: PAWS_MODULE_NAME,
    abilityName: PAWS_VPN_ABILITY,
    parameters,
  };
}

/**
 * The first VPN authorization flow preserves the original WantParams under
 * `myParams`. Later starts deliver them directly, so both layouts must be
 * accepted without retrying the extension ability.
 */
function readVpnParameters(want?: Want): Record<string, Object> | undefined {
  const parameters = want?.parameters as Record<string, Object> | undefined;
  if (!parameters) {
    return undefined;
  }
  if (typeof parameters['optionsJson'] === 'string') {
    return parameters;
  }
  const nested = parameters[PAWS_SYSTEM_PARAMETERS_KEY];
  if (typeof nested === 'object' && nested !== null && !Array.isArray(nested)) {
    return nested as Record<string, Object>;
  }
  return parameters;
}

export function readPlatformSharedMemoryFds(want: Want): PawsPlatformSharedMemoryFds | undefined {
  let ashmemFd = want.fds?.[PAWS_ASHMEM_FD_KEY];
  let notificationFd = want.fds?.[PAWS_NOTIFICATION_FD_KEY];
  if (!isValidFileDescriptor(ashmemFd) || !isValidFileDescriptor(notificationFd)) {
    const parameters = readVpnParameters(want);
    ashmemFd = readFileDescriptorParameter(parameters, PAWS_ASHMEM_FD_KEY);
    notificationFd = readFileDescriptorParameter(parameters, PAWS_NOTIFICATION_FD_KEY);
  }
  if (!isValidFileDescriptor(ashmemFd) || !isValidFileDescriptor(notificationFd)) {
    return undefined;
  }
  return {
    ashmemFd: ashmemFd as number,
    notificationFd: notificationFd as number,
  };
}

export function readPlatformStartAttemptId(want?: Want): string {
  const value = readVpnParameters(want)?.[PAWS_START_ATTEMPT_ID_KEY];
  if (typeof value === 'string' && value.length > 0) {
    return value;
  }
  throw new Error('VPN extension request is missing start attempt id');
}

export function readOptionsJson(want?: Want): string {
  const value = readVpnParameters(want)?.['optionsJson'];
  if (typeof value === 'string' && value.length > 0) {
    return value;
  }
  return DEFAULT_OPTIONS_JSON;
}

function readFileDescriptorParameter(
  parameters: Record<string, Object> | undefined,
  key: string,
): number | undefined {
  const descriptor = parameters?.[key] as PawsWantFileDescriptor | undefined;
  if (!descriptor || descriptor.type !== 'FD' || !isValidFileDescriptor(descriptor.value)) {
    return undefined;
  }
  return descriptor.value;
}

function isValidFileDescriptor(value?: number): boolean {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0;
}

export function parseOptions(optionsJson: string): VpnOptions {
  try {
    return JSON.parse(optionsJson) as VpnOptions;
  } catch (_err) {
    return {};
  }
}

export function parseLinkAddress(value: string, defaultPrefix: number): LinkAddress {
  const parts = value.split('/');
  const address = parts[0];
  const prefixLength = parts.length > 1 ? Number.parseInt(parts[1], 10) : defaultPrefix;
  const family = address.includes(':') ? 2 : 1;
  return new LinkAddress(new NetAddress(address, family), Number.isFinite(prefixLength) ? prefixLength : defaultPrefix);
}

export function addressDefaultPrefix(value: string): number {
  return value.includes(':') ? 128 : 32;
}

export function routeDefaultPrefix(value: string): number {
  return value.includes(':') ? 128 : 32;
}
