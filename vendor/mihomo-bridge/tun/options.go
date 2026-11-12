package tun

import (
	"fmt"
	"net"
	"net/netip"
	"strings"

	"github.com/metacubex/mihomo/constant"
	LC "github.com/metacubex/mihomo/listener/config"
)

// interfaceOptions is the external platform-TUN configuration consumed by
// Start. MTU is required: the OS and userspace reader must use the same value,
// rather than independently defaulting to different packet capacities.
func interfaceOptions(stack, address, dns string, mtu uint32) (LC.Tun, error) {
	if mtu == 0 || mtu > 65535 {
		return LC.Tun{}, fmt.Errorf("platform TUN MTU must be in 1..65535, got %d", mtu)
	}
	var prefix4 []netip.Prefix
	var prefix6 []netip.Prefix
	tunStack, ok := constant.StackTypeMapping[strings.ToLower(stack)]
	if !ok {
		tunStack = constant.TunMips
	}
	for _, a := range strings.Split(address, ",") {
		a = strings.TrimSpace(a)
		if len(a) == 0 {
			continue
		}
		prefix, err := netip.ParsePrefix(a)
		if err != nil {
			return LC.Tun{}, err
		}
		if prefix.Addr().Is4() {
			prefix4 = append(prefix4, prefix)
		} else {
			prefix6 = append(prefix6, prefix)
		}
	}
	var dnsHijack []string
	for _, d := range strings.Split(dns, ",") {
		d = strings.TrimSpace(d)
		if len(d) == 0 {
			continue
		}
		hijack := net.JoinHostPort(d, "53")
		if _, err := netip.ParseAddrPort(hijack); err != nil {
			return LC.Tun{}, err
		}
		dnsHijack = append(dnsHijack, hijack)
	}
	return LC.Tun{
		Enable:              true,
		Device:              "FlClash",
		Stack:               tunStack,
		DNSHijack:           dnsHijack,
		AutoRoute:           false,
		AutoDetectInterface: false,
		Inet4Address:        prefix4,
		Inet6Address:        prefix6,
		MTU:                 mtu,
	}, nil
}
