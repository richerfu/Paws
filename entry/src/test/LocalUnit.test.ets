import { describe, beforeAll, beforeEach, afterEach, afterAll, it, expect } from '@ohos/hypium';
import {
  blockedApplications,
  buildVpnWant,
  currentHMetaBundleName,
  DEFAULT_OPTIONS_JSON,
  HMetaVpnConfig,
  HMETA_BUNDLE_NAME,
  HMETA_MODULE_NAME,
  HMETA_VPN_ABILITY,
  perAppMode,
  parseLinkAddress,
  parseOptions,
  readOptionsJson,
  setHMetaBundleName,
  trustedApplications,
} from '../main/ets/vpnability/VpnConfig';

export default function localUnitTest() {
  describe('vpnConfigUnitTest', () => {
    beforeAll(() => {
    });
    beforeEach(() => {
      setHMetaBundleName(HMETA_BUNDLE_NAME);
    });
    afterEach(() => {
    });
    afterAll(() => {
    });

    it('buildsVpnWantForExtensionAbility', 0, () => {
      const want = buildVpnWant('{"mtu":1400}');
      expect(want.bundleName).assertEqual(HMETA_BUNDLE_NAME);
      expect(want.moduleName).assertEqual(HMETA_MODULE_NAME);
      expect(want.abilityName).assertEqual(HMETA_VPN_ABILITY);
      expect(want.parameters?.optionsJson).assertEqual('{"mtu":1400}');
    });

    it('usesRuntimeBundleNameForVpnWantAndSelfBypass', 0, () => {
      setHMetaBundleName(' com.example.hmeta.debug ');
      expect(currentHMetaBundleName()).assertEqual('com.example.hmeta.debug');
      expect(buildVpnWant().bundleName).assertEqual('com.example.hmeta.debug');

      const blocked = blockedApplications({ blockedApplications: ['com.demo.app'] });
      expect(blocked[0]).assertEqual('com.example.hmeta.debug');
      expect(blocked[1]).assertEqual('com.demo.app');

      const trusted = trustedApplications({
        perAppMode: 'proxy',
        trustedApplications: ['com.demo.app', 'com.example.hmeta.debug'],
      });
      expect(trusted.length).assertEqual(1);
      expect(trusted[0]).assertEqual('com.demo.app');

      setHMetaBundleName('');
      expect(currentHMetaBundleName()).assertEqual(HMETA_BUNDLE_NAME);
    });

    it('parsesOptionsAndFallsBackToDefaultJson', 0, () => {
      expect(readOptionsJson(undefined)).assertEqual(DEFAULT_OPTIONS_JSON);
      expect(parseOptions('bad-json').mtu).assertEqual(undefined);
      expect(parseOptions('{"mtu":1400,"ipv6":true}').mtu).assertEqual(1400);
      expect(parseOptions('{"mtu":1400,"ipv6":true}').ipv6).assertTrue();
    });

    it('keepsDefaultDnsInSyncWithRustModel', 0, () => {
      const options = parseOptions(DEFAULT_OPTIONS_JSON);
      expect(options.dnsAddresses?.[0]).assertEqual('172.19.0.2');
      expect(options.dnsServers?.[0]).assertEqual('223.5.5.5');
      expect(options.dnsServers?.[1]).assertEqual('119.29.29.29');
      expect(options.dnsFallbacks?.[0]).assertEqual('1.1.1.1');
      expect(options.dnsFallbacks?.[1]).assertEqual('8.8.8.8');
      expect(options.dnsNameserverPolicy?.['geosite:cn']?.[0]).assertEqual('223.5.5.5');
      expect(options.dnsNameserverPolicy?.['geosite:geolocation-!cn']?.[0]).assertEqual('1.1.1.1');
    });

    it('buildsVpnConfigForIpv4AndIpv6', 0, () => {
      const config = new HMetaVpnConfig({
        addresses: ['172.19.0.1/30', 'fdfe:dcba:9876::1/126'],
        routes: ['0.0.0.0/0', '::/0'],
        dnsAddresses: ['172.19.0.2'],
        mtu: 1400,
        ipv6: true,
      }, [], [HMETA_BUNDLE_NAME]);
      expect(config.mtu).assertEqual(1400);
      expect(config.addresses.length).assertEqual(2);
      expect(config.routes.length).assertEqual(2);
      expect(config.routes[0].isDefaultRoute).assertTrue();
      expect(config.routes[1].isDefaultRoute).assertTrue();
      expect(config.isIPv4Accepted).assertTrue();
      expect(config.isIPv6Accepted).assertTrue();
      expect(config.blockedApplications[0]).assertEqual(HMETA_BUNDLE_NAME);
    });

    it('normalizesBlockedApplications', 0, () => {
      const blocked = blockedApplications({ blockedApplications: [' com.demo.app ', 'com.demo.app'] });
      expect(blocked[0]).assertEqual(HMETA_BUNDLE_NAME);
      expect(blocked[1]).assertEqual('com.demo.app');
      expect(blocked.length).assertEqual(2);
      const alreadyBlocked = blockedApplications({ blockedApplications: [HMETA_BUNDLE_NAME] });
      expect(alreadyBlocked.length).assertEqual(1);
    });

    it('mapsPerAppModesToTrustedOrBlockedApplications', 0, () => {
      const proxyOptions = {
        perAppMode: 'proxy',
        trustedApplications: [' com.demo.app ', 'com.demo.app', HMETA_BUNDLE_NAME],
        blockedApplications: ['com.blocked.app'],
      };
      expect(perAppMode(proxyOptions)).assertEqual('proxy');
      expect(trustedApplications(proxyOptions).length).assertEqual(1);
      expect(trustedApplications(proxyOptions)[0]).assertEqual('com.demo.app');
      expect(blockedApplications(proxyOptions).length).assertEqual(0);

      const bypassOptions = { perAppMode: 'bypass', blockedApplications: ['com.demo.app'] };
      expect(perAppMode(bypassOptions)).assertEqual('bypass');
      expect(trustedApplications(bypassOptions).length).assertEqual(0);
      expect(blockedApplications(bypassOptions)[0]).assertEqual(HMETA_BUNDLE_NAME);
      expect(blockedApplications(bypassOptions)[1]).assertEqual('com.demo.app');
    });

    it('parsesLinkAddresses', 0, () => {
      const ipv4 = parseLinkAddress('198.18.0.1/16', 32);
      expect(ipv4.address.family).assertEqual(1);
      expect(ipv4.prefixLength).assertEqual(16);
      const ipv6 = parseLinkAddress('fdfe:dcba:9876::1/126', 128);
      expect(ipv6.address.family).assertEqual(2);
      expect(ipv6.prefixLength).assertEqual(126);
    });
  });
}
