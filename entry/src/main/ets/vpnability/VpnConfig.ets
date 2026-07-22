import { Want } from '@kit.AbilityKit';

export const HMETA_BUNDLE_NAME: string = 'com.richerfu.paws';
export const HMETA_MODULE_NAME: string = 'entry';
export const HMETA_ENTRY_ABILITY: string = 'EntryAbility';
export const HMETA_VPN_ABILITY: string = 'HMetaVpnExtensionAbility';

let hmetaBundleName: string = HMETA_BUNDLE_NAME;

export function setHMetaBundleName(bundleName?: string): void {
  const value = bundleName?.trim() ?? '';
  hmetaBundleName = value.length > 0 ? value : HMETA_BUNDLE_NAME;
}

export function currentHMetaBundleName(): string {
  return hmetaBundleName;
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

export class HMetaVpnConfig {
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

export function buildVpnWant(optionsJson?: string): Want {
  return {
    deviceId: '',
    bundleName: currentHMetaBundleName(),
    moduleName: HMETA_MODULE_NAME,
    abilityName: HMETA_VPN_ABILITY,
    parameters: {
      optionsJson: optionsJson ?? '',
    },
  };
}

export function readOptionsJson(value?: string): string {
  if (typeof value === 'string' && value.length > 0) {
    return value;
  }
  return DEFAULT_OPTIONS_JSON;
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
