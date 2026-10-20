package main

import (
	"strings"
	"testing"
)

func TestPhysicalDNS(t *testing.T) {
	config := []byte(`{"dns":{"servers":[{"tag":"dns-local","type":"local"},{"tag":"custom","type":"https","server":"dns.example"}]}}`)
	out, err := withPhysicalDNS(config, []string{"::1", "2001:db8::53", "192.0.2.53"})
	if err != nil || !strings.Contains(string(out), `"server":"192.0.2.53"`) || !strings.Contains(string(out), `"type":"https"`) {
		t.Fatalf("physical DNS conversion failed: %s %v", out, err)
	}
	if _, err = withPhysicalDNS(config, []string{"127.0.0.1", "::"}); err == nil {
		t.Fatal("unusable DNS must fail closed")
	}
}
