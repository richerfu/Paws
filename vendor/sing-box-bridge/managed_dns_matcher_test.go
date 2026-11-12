package main

import (
	"strings"
	"testing"

	D "github.com/miekg/dns"
	"github.com/sagernet/sing-box/adapter"
	R "github.com/sagernet/sing-box/route/rule"
)

// A DNS policy matcher uses the actual domain predicate, which has always
// rejected empty metadata before running its regex. The internal all-domain
// expression therefore needs no empty-string match. Test the public wire
// representation as well: control bytes are escaped by the real DNS decoder.
func TestPawsDNSCanonicalWildcardPreservesNativeWireDomainPredicate(t *testing.T) {
	previous, err := R.NewDomainRegexItem([]string{".*"})
	if err != nil {
		t.Fatal(err)
	}
	current, err := R.NewDomainRegexItem([]string{"(?i).+"})
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{".", "example.test.", "EXAMPLE.TEST.", "é.example.", "\n.", `a\000b.example.`, `\255.example.`} {
		t.Run(name, func(t *testing.T) {
			message := new(D.Msg)
			message.SetQuestion(name, D.TypeA)
			wire, err := message.Pack()
			if err != nil {
				t.Fatal(err)
			}
			decoded := new(D.Msg)
			if err = decoded.Unpack(wire); err != nil {
				t.Fatal(err)
			}
			domain := strings.TrimSuffix(strings.ToLower(decoded.Question[0].Name), ".")
			metadata := &adapter.InboundContext{Domain: domain, QueryType: D.TypeA}
			before, after := previous.Match(metadata), current.Match(metadata)
			if before != after || after != (domain != "") {
				t.Fatal("canonical expression changed actual DNS matcher behavior", name, decoded.Question[0].Name, before, after)
			}
		})
	}
}
