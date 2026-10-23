package main

import (
	"context"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"github.com/sagernet/sing-box/option"
	M "github.com/sagernet/sing/common/metadata"
)

func TestPawsNativeProviderHealthTimerUsesNativeOutboundAndStops(t *testing.T) {
	var requests atomic.Uint64
	var fail, hold atomic.Bool
	entered, cancelled := make(chan struct{}, 1), make(chan struct{}, 1)
	release := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		if hold.Load() {
			select {
			case entered <- struct{}{}:
			default:
			}
			select {
			case <-r.Context().Done():
				cancelled <- struct{}{}
			case <-release:
			}
			return
		}
		if fail.Load() {
			w.WriteHeader(http.StatusServiceUnavailable)
		} else {
			w.WriteHeader(http.StatusNoContent)
		}
	}))
	defer func() { close(release); server.Close() }()
	owner, _, _ := newNativeManagedGroupFixture(t, "select")
	definition := pawsProviderDefinition{Type: "inline", Health: pawsProviderHealth{Enable: true, URL: server.URL, Interval: 1, Lazy: true, Timeout: 2000, ExpectedStatus: "204"}}
	// Payload compilation already has an independent C ABI acceptance suite.
	// This fixture uses the actual native staging/start/publication path and a
	// real Direct outbound so this case isolates scheduling and retirement.
	record := &pawsProviderRecord{definition: definition, tags: []string{"health-native"}, options: []option.Outbound{{Type: "direct", Tag: "health-native", Options: &option.DirectOutboundOptions{}}}}
	owner.groups["Auto"].options.Outbounds = []string{"health-native"}
	generation, err := owner.stageProviders(map[string]*pawsProviderRecord{"health": record})
	if err != nil {
		t.Fatal(err)
	}
	if err = owner.providers.publish(generation); err != nil {
		t.Fatal(err)
	}
	state, err := owner.providerState("proxies", "health")
	if err != nil || state.Members == nil || len(*state.Members) != 1 || (*state.Members)[0].Alive != nil {
		t.Fatal("untested member must be unknown", state, err)
	}
	wait := func(description string, condition func() bool) {
		t.Helper()
		deadline := time.NewTimer(4 * time.Second)
		defer deadline.Stop()
		tick := time.NewTicker(10 * time.Millisecond)
		defer tick.Stop()
		for !condition() {
			select {
			case <-deadline.C:
				t.Fatal(description)
			case <-tick.C:
			}
		}
	}
	alive := func(expected bool) bool {
		state, err := owner.providerState("proxies", "health")
		if err != nil || state.Members == nil {
			return false
		}
		member := (*state.Members)[0]
		return member.Alive != nil && *member.Alive == expected && ((member.Delay != nil) == expected)
	}
	owner.startProviderHealth("health", definition)
	wait("initial real provider check did not publish alive", func() bool { return alive(true) })
	initial := requests.Load()
	time.Sleep(1200 * time.Millisecond)
	if requests.Load() != initial {
		t.Fatal("an idle lazy provider was probed again")
	}
	// Merely asking for a roster does not count as traffic or wake lazy checks.
	for range 8 {
		_, _ = owner.providerState("proxies", "health")
	}
	touch := func() {
		t.Helper()
		connection, err := owner.groups["Auto"].DialContext(context.Background(), "tcp", M.ParseSocksaddr(server.Listener.Addr().String()))
		if err != nil {
			t.Fatal(err)
		}
		_ = connection.Close()
	}
	touch()
	wait("a real native dial did not wake periodic health", func() bool { return requests.Load() > initial && alive(true) })
	fail.Store(true)
	touch()
	wait("failed real URL test did not publish false", func() bool { return alive(false) })
	hold.Store(true)
	touch()
	select {
	case <-entered:
	case <-time.After(4 * time.Second):
		t.Fatal("no real in-flight periodic probe to retire")
	}
	before := requests.Load()
	for range 8 {
		owner.scheduleProviderHealth("health")
	}
	if requests.Load() != before {
		t.Fatal("pending scheduled checks duplicated native HTTP work")
	}
	closed := make(chan error, 1)
	go func() { closed <- owner.close() }()
	select {
	case err := <-closed:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(time.Second):
		t.Fatal("owner close did not cancel and join actual provider health")
	}
	select {
	case <-cancelled:
	case <-time.After(time.Second):
		t.Fatal("real HTTP probe was not cancelled")
	}
	time.Sleep(1200 * time.Millisecond)
	if requests.Load() != before {
		t.Fatal("retired provider scheduler initiated another native request")
	}
}
