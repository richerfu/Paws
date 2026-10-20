package main

import (
	"encoding/json"
	"errors"
	"net/netip"
)

// The desktop local DNS transport reads /etc/resolv.conf, which is not
// HarmonyOS's network-scoped DNS configuration. Resolve through the physical
// network's actual DNS address; the platform socket controller binds every
// DNS socket to the same netId before sending traffic.
func withPhysicalDNS(config []byte, servers []string) ([]byte, error) {
	var chosen string
	for _, server := range servers {
		addr, err := netip.ParseAddr(server)
		if err != nil || addr.IsLoopback() || addr.IsUnspecified() {
			continue
		}
		if chosen == "" || addr.Is4() {
			chosen = server
		}
		if addr.Is4() {
			break
		}
	}
	if chosen == "" {
		return nil, errors.New("physical network DNS unavailable")
	}
	var value map[string]any
	if err := json.Unmarshal(config, &value); err != nil {
		return nil, err
	}
	dns, _ := value["dns"].(map[string]any)
	list, _ := dns["servers"].([]any)
	for _, item := range list {
		server, ok := item.(map[string]any)
		if ok && server["tag"] == "dns-local" && server["type"] == "local" {
			server["type"] = "udp"
			server["server"] = chosen
		}
	}
	return json.Marshal(value)
}
