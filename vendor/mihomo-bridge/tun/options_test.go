package tun

import (
	"fmt"
	"net/netip"
	"reflect"
	"testing"

	"github.com/metacubex/mihomo/config"
	"github.com/metacubex/mihomo/constant"
	_ "github.com/metacubex/mihomo/hub/executor" // native parser's temporaryUpdateGeneral linkname
)

const mtuProfile = "mixed-port: 0\nport: 0\nsocks-port: 0\ngeo-auto-update: false\nntp: {enable: false}\ndns: {enable: false}\nproxies: []\nproxy-groups: []\nrules: [\"MATCH,DIRECT\"]\n"

func TestInterfaceOptionsConsumeTheParsedNativeMtu(t *testing.T) {
	constant.SetHomeDir(t.TempDir())
	for _, mtu := range []uint32{1280, 1500, 2816, 9000} {
		t.Run(fmt.Sprint(mtu), func(t *testing.T) {
			// Use the pinned native parser used by quickSetup, then the actual
			// options constructor used by Start. No fake TUN opens a device.
			cfg, err := config.Parse([]byte(mtuProfile + fmt.Sprintf("tun: {enable: false, mtu: %d}\n", mtu)))
			if err != nil {
				t.Fatal(err)
			}
			options, err := interfaceOptions("mixed", "172.19.0.1/30, fdfe::1/126", "0.0.0.0,::", cfg.General.Tun.MTU)
			if err != nil {
				t.Fatal(err)
			}
			if options.MTU != mtu || options.FileDescriptor != 0 {
				t.Fatalf("MTU changed or the pure constructor acquired a descriptor: %#v", options)
			}
			if !options.Enable || options.AutoRoute || options.AutoDetectInterface || options.Stack != constant.TunMixed {
				t.Fatalf("external platform ownership changed: %#v", options)
			}
			if !reflect.DeepEqual(options.Inet4Address, []netip.Prefix{netip.MustParsePrefix("172.19.0.1/30")}) ||
				!reflect.DeepEqual(options.Inet6Address, []netip.Prefix{netip.MustParsePrefix("fdfe::1/126")}) ||
				!reflect.DeepEqual(options.DNSHijack, []string{"0.0.0.0:53", "[::]:53"}) {
				t.Fatalf("non-MTU TUN settings changed: %#v", options)
			}
		})
	}
}

func TestInterfaceOptionsRejectMissingOrUnrepresentableMtu(t *testing.T) {
	constant.SetHomeDir(t.TempDir())
	for _, mtu := range []uint32{0, 65536, ^uint32(0)} {
		options, err := interfaceOptions("mixed", "172.19.0.1/30", "0.0.0.0", mtu)
		if err == nil || options.MTU != 0 || options.FileDescriptor != 0 {
			t.Fatalf("invalid MTU %d silently defaulted: %#v, %v", mtu, options, err)
		}
	}
	cfg, err := config.Parse([]byte(mtuProfile))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := interfaceOptions("mixed", "172.19.0.1/30", "0.0.0.0", cfg.General.Tun.MTU); err == nil {
		t.Fatal("a native config missing the application-provided MTU silently defaulted")
	}
	if _, err := config.Parse([]byte(mtuProfile + "tun: {mtu: -1}\n")); err == nil {
		t.Fatal("native parser accepted a negative uint32 MTU")
	}
}

func TestInterfaceOptionsPreserveAddressValidation(t *testing.T) {
	for _, input := range [][2]string{{"not-a-prefix", "0.0.0.0"}, {"172.19.0.1/30", "not-an-ip"}} {
		options, err := interfaceOptions("mixed", input[0], input[1], 1500)
		if err == nil || options.FileDescriptor != 0 {
			t.Fatalf("invalid address/DNS acquired interface options: %#v, %v", options, err)
		}
	}
}
