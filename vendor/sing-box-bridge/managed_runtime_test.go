package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	D "github.com/miekg/dns"
	"github.com/sagernet/sing-box/adapter"
	"github.com/sagernet/sing/service"
	"io"
	"net"
	"net/http"
	"net/netip"
	"sync/atomic"
	"testing"
	"time"

	box "github.com/sagernet/sing-box"
	"github.com/sagernet/sing-box/option"
	SJ "github.com/sagernet/sing/common/json"
	M "github.com/sagernet/sing/common/metadata"
)

func newNativeManagedGroupFixture(t *testing.T, kind string, mutate ...func(map[string]any)) (*pawsSingRuntime, *box.Box, string) {
	t.Helper()
	reservation, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	address := reservation.Addr().String()
	_ = reservation.Close()
	document := map[string]any{
		"inbounds": []any{},
		"log":      map[string]any{"level": "debug"}, "outbounds": []any{map[string]any{"type": "direct", "tag": "A"}, map[string]any{"type": "direct", "tag": "B"}, map[string]any{"type": "paws-group", "tag": "Auto", "kind": kind, "strategy": "round-robin", "outbounds": []string{"A", "B"}, "interval": 0, "timeout": 1000, "lazy": true}},
		"route": map[string]any{"final": "Auto"}, "experimental": map[string]any{"clash_api": map[string]any{"external_controller": address, "secret": "synthetic-sing-owner"}}, "paws_runtime": map[string]any{"ports": map[string]any{}, "access": map[string]any{}, "proxyProviders": map[string]any{}, "ruleProviders": map[string]any{}},
	}
	for _, change := range mutate {
		change(document)
	}
	raw, err := json.Marshal(document)
	if err != nil {
		t.Fatal(err)
	}
	owner, native, err := newPawsSingRuntime(context.Background(), raw, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	ctx := owner.nativeContext()
	options, err := SJ.UnmarshalExtendedContext[option.Options](ctx, native)
	if err != nil {
		t.Fatal(err)
	}
	instance, err := box.New(box.Options{Context: ctx, Options: options})
	if err != nil {
		t.Fatal(err)
	}
	if err = instance.StartWithPreparation(func() error { return owner.initialize(instance) }); err != nil {
		t.Fatal(err)
	}
	if err = owner.started(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = owner.close(); _ = instance.Close() })
	return owner, instance, address
}
func TestPawsNativeManagedGroupUsesControllerAndSessionOwner(t *testing.T) {
	owner, _, address := newNativeManagedGroupFixture(t, "url-test")
	var err error
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: time.Second}
	request := func(method, path string, body []byte) (int, []byte) {
		t.Helper()
		req, _ := http.NewRequest(method, "http://"+address+path, bytes.NewReader(body))
		req.Header.Set("Authorization", "Bearer synthetic-sing-owner")
		req.Header.Set("Content-Type", "application/json")
		response, err := client.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer response.Body.Close()
		data, _ := io.ReadAll(response.Body)
		return response.StatusCode, data
	}
	revisions := func() pawsRevisions {
		t.Helper()
		status, data := request("GET", "/paws/observations/revisions", nil)
		if status != 200 {
			t.Fatal(status, string(data))
		}
		var result pawsRevisions
		if err := json.Unmarshal(data, &result); err != nil {
			t.Fatal(err)
		}
		return result
	}
	before := revisions()
	status, data := request("PUT", "/proxies/Auto", []byte(`{"name":"B"}`))
	if status != 204 {
		t.Fatal(status, string(data))
	}
	after := revisions()
	if after.Groups <= before.Groups {
		t.Fatal("actual Set did not notify")
	}
	status, data = request("GET", "/paws/observations/proxy-groups/Auto", nil)
	var group pawsProxyGroup
	if status != 200 || json.Unmarshal(data, &group) != nil {
		t.Fatal(status, string(data))
	}
	if group.Fixed == nil || *group.Fixed != "B" || len(group.Proxies) != 2 {
		t.Fatalf("bad actual group: %#v", group)
	}
	status, _ = request("PUT", "/proxies/Auto", []byte(`{"name":"B"}`))
	if status != 204 || revisions().Groups != after.Groups {
		t.Fatal("same fixed selection produced false change")
	}
	status, _ = request("DELETE", "/proxies/Auto", nil)
	if status != 204 || revisions().Groups <= after.Groups {
		t.Fatal("actual unfix did not notify")
	}
	revisionsBeforeClose := revisions().Groups
	if err = owner.close(); err != nil {
		t.Fatal(err)
	}
	requestAfterClose, _ := http.NewRequest("PUT", "http://"+address+"/proxies/Auto", bytes.NewReader([]byte(`{"name":"A"}`)))
	requestAfterClose.Header.Set("Authorization", "Bearer synthetic-sing-owner")
	requestAfterClose.Header.Set("Content-Type", "application/json")
	response, err := client.Do(requestAfterClose)
	if err == nil {
		defer response.Body.Close()
		if response.StatusCode < 400 {
			t.Fatal("retired owner accepted a mutation")
		}
	}
	if owner.groups["Auto"].Fixed() != nil || owner.observations.revisions().Groups != revisionsBeforeClose {
		t.Fatal("retired owner changed actual selection or metadata")
	}
}

func TestPawsNativeGroupObservationDoesNotConsumeRoundRobinDials(t *testing.T) {
	owner, _, address := newNativeManagedGroupFixture(t, "load-balance")
	group := owner.groups["Auto"]
	echo, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer echo.Close()
	go func() {
		for {
			connection, err := echo.Accept()
			if err != nil {
				return
			}
			_ = connection.Close()
		}
	}()
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: time.Second}
	observe := func() {
		t.Helper()
		for _, path := range []string{"/paws/observations/groups", "/paws/observations/proxy-groups/Auto", "/proxies/Auto", "/proxies"} {
			request, _ := http.NewRequest("GET", "http://"+address+path, nil)
			request.Header.Set("Authorization", "Bearer synthetic-sing-owner")
			response, err := client.Do(request)
			if err != nil {
				t.Fatal(err)
			}
			_, _ = io.Copy(io.Discard, response.Body)
			_ = response.Body.Close()
			if response.StatusCode != 200 {
				t.Fatal(response.StatusCode)
			}
		}
	}
	for index := 0; index < 8; index++ {
		observe()
	}
	if group.round.Load() != 0 {
		t.Fatal("read-only queries consumed round robin")
	}
	for index, want := range []string{"A", "B"} {
		connection, err := group.DialContext(owner.ctx, "tcp", M.ParseSocksaddr(echo.Addr().String()))
		if err != nil {
			t.Fatal(err)
		}
		_ = connection.Close()
		if got := group.Now(); got != want {
			t.Fatal("wrong actual native dial selection", want, got)
		}
		before := owner.observations.revisions().Groups
		for count := 0; count < 8; count++ {
			observe()
		}
		if group.round.Load() != uint64(index+1) || owner.observations.revisions().Groups != before {
			t.Fatal("observation changed native round robin or metadata")
		}
	}
}

func TestPawsNativeLargeGenerationCachesRosterOutsideObservationAndDial(t *testing.T) {
	owner, _, address := newNativeManagedGroupFixture(t, "load-balance")
	group := owner.groups["Auto"]
	group.options.Use = []string{"large"}
	record := &pawsProviderRecord{}
	for index := 0; index < 4096; index++ {
		tag := fmt.Sprintf("large-%04d", index)
		record.tags = append(record.tags, tag)
		record.options = append(record.options, option.Outbound{Type: "direct", Tag: tag, Options: &option.DirectOutboundOptions{}})
	}
	generation, err := owner.stageProviders(map[string]*pawsProviderRecord{"large": record})
	if err != nil {
		t.Fatal(err)
	}
	if err = owner.providers.publish(generation); err != nil {
		t.Fatal(err)
	}
	roster := generation.groups["Auto"]
	if roster == nil || len(roster.tags) != 4098 || len(roster.index) != 4098 {
		t.Fatal("wrong immutable native roster")
	}
	if allocations := testing.AllocsPerRun(128, func() { _ = group.Now() }); allocations != 0 {
		t.Fatal("read observation rebuilt roster", allocations)
	}
	if allocations := testing.AllocsPerRun(128, func() { _ = group.choose(generation, M.Socksaddr{}, true) }); allocations != 0 {
		t.Fatal("native RR choice rebuilt roster", allocations)
	}
	all := group.All()
	all[0] = "must-not-mutate-owner"
	if roster.tags[0] != "A" {
		t.Fatal("expanded response owns mutable roster")
	}
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: time.Second}
	before := group.round.Load()
	for index := 0; index < 16; index++ {
		request, _ := http.NewRequest("GET", "http://"+address+"/paws/observations/groups", nil)
		request.Header.Set("Authorization", "Bearer synthetic-sing-owner")
		response, err := client.Do(request)
		if err != nil {
			t.Fatal(err)
		}
		_, _ = io.Copy(io.Discard, response.Body)
		_ = response.Body.Close()
		if response.StatusCode != 200 {
			t.Fatal(response.StatusCode)
		}
	}
	if generation.groups["Auto"] != roster || group.round.Load() != before {
		t.Fatal("summary rebuilt roster or consumed RR")
	}
	echo, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer echo.Close()
	go func() {
		for {
			connection, err := echo.Accept()
			if err != nil {
				return
			}
			_ = connection.Close()
		}
	}()
	for index := 0; index < 8; index++ {
		connection, err := group.DialContext(owner.ctx, "tcp", M.ParseSocksaddr(echo.Addr().String()))
		if err != nil {
			t.Fatal(err)
		}
		_ = connection.Close()
	}
	if group.round.Load() != before+8 || generation.groups["Auto"] != roster {
		t.Fatal("actual native dial changed generation index")
	}
}

func TestPawsNativeAppliedServicesAcknowledgeOnlyOwnedListeners(t *testing.T) {
	freePort := func() int {
		socket, err := net.Listen("tcp", "127.0.0.1:0")
		if err != nil {
			t.Fatal(err)
		}
		port := socket.Addr().(*net.TCPAddr).Port
		_ = socket.Close()
		return port
	}
	benchmark, mixed, public := freePort(), freePort(), freePort()
	owner, _, address := newNativeManagedGroupFixture(t, "select", func(document map[string]any) {
		document["inbounds"] = []any{map[string]any{"type": "mixed", "tag": "sing-box-benchmark", "listen": "127.0.0.1", "listen_port": benchmark, "users": []any{map[string]any{"username": "paws-app", "password": "synthetic-sing-owner"}}}, map[string]any{"type": "mixed", "tag": "mixed-in", "listen": "127.0.0.1", "listen_port": mixed}}
		metadata := document["paws_runtime"].(map[string]any)
		metadata["ports"] = map[string]any{"controllerEnabled": true, "controllerPort": public, "mixedEnabled": true, "mixedPort": mixed}
		metadata["access"] = map[string]any{"secret": "independent-public-credential"}
	})
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: time.Second}
	request, _ := http.NewRequest("GET", "http://"+address+"/paws/observations/applied-services", nil)
	request.Header.Set("Authorization", "Bearer synthetic-sing-owner")
	response, err := client.Do(request)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	var applied pawsAppliedServices
	if response.StatusCode != 200 || json.NewDecoder(response.Body).Decode(&applied) != nil {
		t.Fatal("native services acknowledgment failed", response.StatusCode)
	}
	if int(applied.BenchmarkPort) != benchmark || !applied.MixedReady || applied.PublicControllerAddress == nil || *applied.PublicControllerAddress != fmt.Sprintf("127.0.0.1:%d", public) {
		t.Fatal("acknowledgment differs from actual native listener handles", applied)
	}
	connection, err := net.DialTimeout("tcp", fmt.Sprintf("127.0.0.1:%d", benchmark), time.Second)
	if err != nil {
		t.Fatal(err)
	}
	_ = connection.SetDeadline(time.Now().Add(time.Second))
	_, _ = connection.Write([]byte{5, 1, 0})
	var greeting [2]byte
	if _, err = io.ReadFull(connection, greeting[:]); err != nil || greeting != [2]byte{5, 255} {
		t.Fatal("private native proxy accepted unauthenticated SOCKS", greeting, err)
	}
	_ = connection.Close()
	if err = owner.close(); err != nil {
		t.Fatal(err)
	}
	if _, err = owner.appliedServices(); err == nil {
		t.Fatal("retired native owner acknowledged listeners")
	}
}

func TestPawsNativeTrimPreservesActualOwnerSelectionAndGeneration(t *testing.T) {
	owner, _, _ := newNativeManagedGroupFixture(t, "url-test")
	group := owner.groups["Auto"]
	if !group.SelectOutbound("B") {
		t.Fatal("native selection failed")
	}
	generation := owner.providers.current.Load()
	before := owner.observations.revisions()
	if err := trimPawsSingRuntimeMemory(owner); err != nil {
		t.Fatal(err)
	}
	if owner.providers.current.Load() != generation || group.Fixed() == nil || *group.Fixed() != "B" || owner.observations.revisions() != before {
		t.Fatal("memory trim changed actual runtime state")
	}
	if err := owner.close(); err != nil {
		t.Fatal(err)
	}
	if err := trimPawsSingRuntimeMemory(owner); err == nil {
		t.Fatal("memory trim succeeded for retired owner")
	}
}

func TestPawsNativeFakeIPPolicyPreservesRealLookupAndCacheIsolation(t *testing.T) {
	udp, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	var requests atomic.Uint64
	started := make(chan struct{})
	server := &D.Server{PacketConn: udp, NotifyStartedFunc: func() { close(started) }, Handler: D.HandlerFunc(func(w D.ResponseWriter, q *D.Msg) {
		requests.Add(1)
		r := new(D.Msg)
		r.SetReply(q)
		if q.Question[0].Qtype == D.TypeA {
			r.Answer = []D.RR{&D.A{Hdr: D.RR_Header{Name: q.Question[0].Name, Rrtype: D.TypeA, Class: D.ClassINET, Ttl: 60}, A: net.ParseIP("192.0.2.9")}}
		}
		_ = w.WriteMsg(r)
	})}
	go func() { _ = server.ActivateAndServe() }()
	<-started
	t.Cleanup(func() { _ = server.Shutdown(); _ = udp.Close() })
	port := udp.LocalAddr().(*net.UDPAddr).Port
	owner, _, _ := newNativeManagedGroupFixture(t, "select", func(document map[string]any) {
		document["dns"] = map[string]any{"servers": []any{map[string]any{"type": "udp", "tag": "real", "server": "127.0.0.1", "server_port": port}, map[string]any{"type": "fakeip", "tag": "fake", "inet4_range": "198.18.0.1/16", "inet6_range": "fc00::/64"}, map[string]any{"type": "paws-dns", "tag": "main", "servers": []string{"real"}, "fakeIP": "fake"}}, "final": "main", "strategy": "ipv4_only", "reverse_mapping": true, "rules": []any{map[string]any{"domain_suffix": []string{"rule.test"}, "server": "main"}}}
	})
	router := service.FromContext[adapter.DNSRouter](owner.ctx)
	manager := service.FromContext[adapter.DNSTransportManager](owner.ctx)
	main, _ := manager.Transport("main")
	fakeRange := netip.MustParsePrefix("198.18.0.0/16")
	for _, domain := range []string{"default.test", "x.rule.test"} {
		for _, explicit := range []bool{false, true} {
			options := adapter.DNSQueryOptions{}
			if explicit {
				options.Transport = main
			}
			query := new(D.Msg)
			query.SetQuestion(domain+".", D.TypeA)
			answer, err := router.Exchange(owner.ctx, query, options)
			if err != nil || len(answer.Answer) != 1 {
				t.Fatal("native fake exchange", err, answer)
			}
			ip := M.AddrFromIP(answer.Answer[0].(*D.A).A)
			if !fakeRange.Contains(ip) {
				t.Fatal("expected actual native fake-IP", ip)
			}
			if _, ok := router.LookupReverseMapping(ip); ok {
				t.Fatal("fake-IP leaked into ordinary DNS reverse cache")
			}
			addresses, err := router.Lookup(owner.ctx, domain, options)
			if err != nil || len(addresses) == 0 {
				t.Fatal("native real lookup", err)
			}
			for _, address := range addresses {
				if address != netip.MustParseAddr("192.0.2.9") {
					t.Fatal("real Lookup received synthetic response", address)
				}
			}
			// A real cached Lookup must never change the next synthesized answer.
			answer, err = router.Exchange(owner.ctx, query, options)
			if err != nil || !fakeRange.Contains(M.AddrFromIP(answer.Answer[0].(*D.A).A)) {
				t.Fatal("response-cache mixed real and fake DNS", err)
			}
		}
	}
	if requests.Load() == 0 {
		t.Fatal("real native Lookup never reached the controlled resolver")
	}
}
