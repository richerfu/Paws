import { describe, beforeAll, beforeEach, afterEach, afterAll, it, expect } from '@ohos/hypium';
import { Want } from '@kit.AbilityKit';
import {
  buildVpnWant,
  currentPawsBundleName,
  DEFAULT_OPTIONS_JSON,
  PawsVpnConfig,
  PAWS_BUNDLE_NAME,
  PAWS_MODULE_NAME,
  PAWS_VPN_ABILITY,
  parseLinkAddress,
  parseOptions,
  readOptionsJson,
  readPlatformSharedMemoryFds,
  readPlatformStartAttemptId,
  setPawsBundleName,
} from '../main/ets/vpnability/VpnConfig';

export default function localUnitTest() {
  describe('vpnConfigUnitTest', () => {
    beforeAll(() => {
    });
    beforeEach(() => {
      setPawsBundleName(PAWS_BUNDLE_NAME);
    });
    afterEach(() => {
    });
    afterAll(() => {
    });

    it('buildsVpnWantForExtensionAbility', 0, () => {
      const want = buildVpnWant('{"mtu":1400}');
      expect(want.bundleName).assertEqual(PAWS_BUNDLE_NAME);
      expect(want.moduleName).assertEqual(PAWS_MODULE_NAME);
      expect(want.abilityName).assertEqual(PAWS_VPN_ABILITY);
      expect(want.parameters?.optionsJson).assertEqual('{"mtu":1400}');
    });

    it('usesRuntimeBundleNameForVpnWant', 0, () => {
      setPawsBundleName(' com.example.paws.debug ');
      expect(currentPawsBundleName()).assertEqual('com.example.paws.debug');
      expect(buildVpnWant().bundleName).assertEqual('com.example.paws.debug');

      setPawsBundleName('');
      expect(currentPawsBundleName()).assertEqual(PAWS_BUNDLE_NAME);
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
      const config = new PawsVpnConfig({
        addresses: ['172.19.0.1/30', 'fdfe:dcba:9876::1/126'],
        routes: ['0.0.0.0/0', '::/0'],
        dnsAddresses: ['172.19.0.2'],
        mtu: 1400,
        ipv6: true,
      });
      expect(config.mtu).assertEqual(1400);
      expect(config.addresses.length).assertEqual(2);
      expect(config.routes.length).assertEqual(2);
      expect(config.routes[0].isDefaultRoute).assertTrue();
      expect(config.routes[1].isDefaultRoute).assertTrue();
      expect(config.isIPv4Accepted).assertTrue();
      expect(config.isIPv6Accepted).assertTrue();
    });

    it('parsesLinkAddresses', 0, () => {
      const ipv4 = parseLinkAddress('198.18.0.1/16', 32);
      expect(ipv4.address.family).assertEqual(1);
      expect(ipv4.prefixLength).assertEqual(16);
      const ipv6 = parseLinkAddress('fdfe:dcba:9876::1/126', 128);
      expect(ipv6.address.family).assertEqual(2);
      expect(ipv6.prefixLength).assertEqual(126);
    });

    it('readsDirectVpnExtensionWant', 0, () => {
      const want = {
        parameters: {
          optionsJson: '{"addresses":["172.19.0.1/30"]}',
          pawsStartAttemptId: 'attempt-direct',
        },
        fds: {
          pawsPlatformAshmem: 21,
          pawsPlatformNotification: 22,
        },
      } as Want;

      const sharedMemory = readPlatformSharedMemoryFds(want);
      expect(readOptionsJson(want)).assertEqual('{"addresses":["172.19.0.1/30"]}');
      expect(readPlatformStartAttemptId(want)).assertEqual('attempt-direct');
      expect(sharedMemory?.ashmemFd).assertEqual(21);
      expect(sharedMemory?.notificationFd).assertEqual(22);
    });

    it('readsFirstAuthorizationVpnExtensionWant', 0, () => {
      const want = {
        parameters: {
          callingUid: 20010048,
          callingPid: 1234,
          myParams: {
            optionsJson: '{"addresses":["172.19.0.1/30"]}',
            pawsStartAttemptId: 'attempt-first-authorization',
            pawsPlatformAshmem: {
              type: 'FD',
              value: 31,
            },
            pawsPlatformNotification: {
              type: 'FD',
              value: 32,
            },
          },
        },
      } as Want;

      const sharedMemory = readPlatformSharedMemoryFds(want);
      expect(readOptionsJson(want)).assertEqual('{"addresses":["172.19.0.1/30"]}');
      expect(readPlatformStartAttemptId(want)).assertEqual('attempt-first-authorization');
      expect(sharedMemory?.ashmemFd).assertEqual(31);
      expect(sharedMemory?.notificationFd).assertEqual(32);
    });

    it('rejectsMalformedNestedFileDescriptors', 0, () => {
      const want = {
        parameters: {
          myParams: {
            optionsJson: '{"addresses":["172.19.0.1/30"]}',
            pawsPlatformAshmem: {
              type: 'not-an-fd',
              value: 31,
            },
            pawsPlatformNotification: {
              type: 'FD',
              value: -1,
            },
          },
        },
      } as Want;

      expect(readPlatformSharedMemoryFds(want) === undefined).assertEqual(true);
    });
  });
}
