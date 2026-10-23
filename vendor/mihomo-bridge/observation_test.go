package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"strconv"
	"testing"
	"time"

	"github.com/metacubex/mihomo/config"
	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/hub"
	"github.com/metacubex/mihomo/hub/executor"
	"github.com/metacubex/mihomo/listener"
	"github.com/metacubex/mihomo/tunnel"
)

// This runs the actual pinned native config, private controller, group Set /
// ForceSet handlers, scoped credentials and owner revisions. No fake controller
// returns a canned projection; no platform TUN or user profile is involved.
func TestPawsNativeControllerSelectionAndRetirementHaveTheSameOwner(t *testing.T) {
	C.SetHomeDir(t.TempDir())
	reservation, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	address := reservation.Addr().String()
	_ = reservation.Close()
	raw := []byte("external-controller: " + address + "\nsecret: synthetic-selection-owner\nmixed-port: 0\nport: 0\nsocks-port: 0\ngeo-auto-update: false\nntp: {enable: false}\ndns: {enable: false}\nproxies:\n  - {name: A, type: direct}\n  - {name: B, type: direct}\nproxy-groups:\n  - {name: Auto, type: url-test, proxies: [A,B], interval: 0}\nrules: [\"MATCH,Auto\"]\n")
	cfg, err := config.Parse(raw)
	if err != nil {
		t.Fatal(err)
	}
	owner, err := preparePawsMihomo(cfg, raw)
	if err != nil {
		t.Fatal(err)
	}
	defer executor.Shutdown()
	defer owner.close()
	hub.ApplyConfig(cfg)
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: time.Second}
	request := func(method, path string, body []byte) (int, []byte, error) {
		req, _ := http.NewRequestWithContext(context.Background(), method, "http://"+address+path, bytes.NewReader(body))
		req.Header.Set("Authorization", "Bearer synthetic-selection-owner")
		req.Header.Set("Content-Type", "application/json")
		response, err := client.Do(req)
		if err != nil {
			return 0, nil, err
		}
		defer response.Body.Close()
		data, err := io.ReadAll(response.Body)
		return response.StatusCode, data, err
	}
	deadline := time.Now().Add(2 * time.Second)
	for {
		status, _, err := request("GET", "/paws/observations/revisions", nil)
		if err == nil && status == 200 {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("actual private controller did not become ready", status, err)
		}
		time.Sleep(10 * time.Millisecond)
	}
	revisions := func() uint64 {
		status, data, err := request("GET", "/paws/observations/revisions", nil)
		if err != nil || status != 200 {
			t.Fatal(status, err)
		}
		var value pawsRevisions
		if err := json.Unmarshal(data, &value); err != nil {
			t.Fatal(err)
		}
		return value.Groups
	}
	before := revisions()
	status, _, err := request("PUT", "/proxies/Auto", []byte(`{"name":"B"}`))
	if err != nil || status != 204 {
		t.Fatal(status, err)
	}
	after := revisions()
	if after <= before {
		t.Fatal("native selection did not advance the actual owner revision", before, after)
	}
	status, data, err := request("GET", "/paws/observations/proxy-groups/Auto", nil)
	if err != nil || status != 200 {
		t.Fatal(status, err)
	}
	var group pawsProxyGroup
	if err := json.Unmarshal(data, &group); err != nil {
		t.Fatal(err)
	}
	if group.Fixed == nil || *group.Fixed != "B" {
		t.Fatalf("actual group not fixed: %#v", group)
	}
	status, _, err = request("PUT", "/proxies/Auto", []byte(`{"name":"B"}`))
	if err != nil || status != 204 {
		t.Fatal(status, err)
	}
	if got := revisions(); got != after {
		t.Fatal("same native pin produced a false revision", after, got)
	}
	status, _, err = request("DELETE", "/proxies/Auto", nil)
	if err != nil || status != 204 {
		t.Fatal(status, err)
	}
	unfixed := revisions()
	if unfixed <= after {
		t.Fatal("actual unfix did not advance group revision")
	}
	status, _, err = request("PUT", "/proxies/Auto", []byte(`{"name":"not-present"}`))
	if err != nil || status != 400 {
		t.Fatal(status, err)
	}
	if got := revisions(); got != unfixed {
		t.Fatal("failed native mutation advanced revision")
	}
	if err := owner.close(); err != nil {
		t.Fatal(err)
	}
	status, _, err = request("PUT", "/proxies/Auto", []byte(`{"name":"A"}`))
	if err == nil && status < 400 {
		t.Fatal("retired owner accepted native mutation", strconv.Itoa(status))
	}
	if owner.observations.revisions().Groups != unfixed {
		t.Fatal("retired owner mutated its group revision")
	}
}

func TestPawsNativeRequiredListenerFailureAndStopPreserveActualOwnership(t *testing.T) {
	C.SetHomeDir(t.TempDir())
	occupied, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	port := occupied.Addr().(*net.TCPAddr).Port
	raw := []byte(fmt.Sprintf("external-controller: 127.0.0.1:19090\nsecret: synthetic-listener-owner\ngeo-auto-update: false\nntp: {enable: false}\ndns: {enable: false}\nlisteners:\n  - {name: tinypaws-mihomo-benchmark, type: mixed, listen: 127.0.0.1, port: %d, users: [{username: paws-app, password: synthetic-listener-owner}]}\nproxies: [{name: A, type: direct}]\nrules: [\"MATCH,A\"]\n", port))
	cfg, err := config.Parse(raw)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = listener.PawsStopInboundListeners(); _ = occupied.Close() })
	if err = listener.PatchInboundListeners(cfg.Listeners, tunnel.Tunnel, true); err == nil {
		t.Fatal("occupied foreign socket was reported applied")
	}
	if _, ok := listener.PawsAppliedInboundAddress(cfg.Listeners["tinypaws-mihomo-benchmark"]); ok {
		t.Fatal("failed native listener acknowledged a foreign port")
	}
	_ = occupied.Close()
	if err = listener.PatchInboundListeners(cfg.Listeners, tunnel.Tunnel, true); err != nil {
		t.Fatal(err)
	}
	if _, ok := listener.PawsAppliedInboundAddress(cfg.Listeners["tinypaws-mihomo-benchmark"]); !ok {
		t.Fatal("actual native listener was not acknowledged")
	}
	authenticate := func(password string, want byte) {
		connection, err := net.DialTimeout("tcp", fmt.Sprintf("127.0.0.1:%d", port), time.Second)
		if err != nil {
			t.Fatal(err)
		}
		defer connection.Close()
		_ = connection.SetDeadline(time.Now().Add(time.Second))
		_, _ = connection.Write([]byte{5, 1, 2})
		var greeting [2]byte
		if _, err = io.ReadFull(connection, greeting[:]); err != nil || greeting != [2]byte{5, 2} {
			t.Fatal("private listener did not require username/password", greeting, err)
		}
		user := "paws-app"
		payload := append([]byte{1, byte(len(user))}, []byte(user)...)
		payload = append(payload, byte(len(password)))
		payload = append(payload, []byte(password)...)
		_, _ = connection.Write(payload)
		var result [2]byte
		if _, err = io.ReadFull(connection, result[:]); err != nil || result[0] != 1 || (want == 0 && result[1] != 0) || (want != 0 && result[1] == 0) {
			t.Fatal("native authentication result", result, err)
		}
	}
	authenticate("wrong-session-credential", 1)
	authenticate("synthetic-listener-owner", 0)
	if err = listener.PawsStopInboundListeners(); err != nil {
		t.Fatal(err)
	}
	if _, ok := listener.PawsAppliedInboundAddress(cfg.Listeners["tinypaws-mihomo-benchmark"]); ok {
		t.Fatal("stopped custom listener still acknowledged")
	}
	rebound, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
	if err != nil {
		t.Fatal("Stop did not release actual native listener", err)
	}
	_ = rebound.Close()
}

func TestPawsNativeAppliedServicesUsesPrivateOwnerAndStopsActualListeners(t *testing.T) {
	C.SetHomeDir(t.TempDir())
	freePort := func() int {
		socket, err := net.Listen("tcp", "127.0.0.1:0")
		if err != nil {
			t.Fatal(err)
		}
		port := socket.Addr().(*net.TCPAddr).Port
		_ = socket.Close()
		return port
	}
	private, benchmark, mixed, public := freePort(), freePort(), freePort(), freePort()
	raw := []byte(fmt.Sprintf("external-controller: 127.0.0.1:%d\nsecret: synthetic-applied-owner\nmixed-port: %d\nallow-lan: false\nbind-address: 127.0.0.1\ngeo-auto-update: false\nntp: {enable: false}\ndns: {enable: false}\npaws-runtime:\n  ports: {mixedPort: %d, mixedEnabled: true, controllerPort: %d, controllerEnabled: true}\n  access: {allowLan: false, secret: public-applied-credential}\nlisteners:\n  - {name: tinypaws-mihomo-benchmark, type: mixed, listen: 127.0.0.1, port: %d, users: [{username: paws-app, password: synthetic-applied-owner}]}\nproxies: [{name: A, type: direct}]\nrules: [\"MATCH,A\"]\n", private, mixed, mixed, public, benchmark))
	cfg, err := config.Parse(raw)
	if err != nil {
		t.Fatal(err)
	}
	owner, err := preparePawsMihomo(cfg, raw)
	if err != nil {
		t.Fatal(err)
	}
	defer executor.Shutdown()
	t.Cleanup(func() { _ = owner.close(); _ = handleStopListener() })
	hub.ApplyConfig(cfg)
	configMu.Lock()
	currentConfig = cfg
	configMu.Unlock()
	if !handleStartListener() {
		t.Fatal("required actual native listeners did not start")
	}
	client := &http.Client{Transport: &http.Transport{Proxy: nil}, Timeout: time.Second}
	var response *http.Response
	deadline := time.Now().Add(2 * time.Second)
	for {
		request, _ := http.NewRequest("GET", fmt.Sprintf("http://127.0.0.1:%d/paws/observations/applied-services", private), nil)
		request.Header.Set("Authorization", "Bearer synthetic-applied-owner")
		response, err = client.Do(request)
		if err == nil && response.StatusCode == 200 {
			break
		}
		if response != nil {
			_ = response.Body.Close()
		}
		if time.Now().After(deadline) {
			t.Fatal("actual acknowledgment unavailable", err)
		}
		time.Sleep(5 * time.Millisecond)
	}
	var applied pawsAppliedServices
	if json.NewDecoder(response.Body).Decode(&applied) != nil {
		t.Fatal("bad actual acknowledgment")
	}
	_ = response.Body.Close()
	if int(applied.BenchmarkPort) != benchmark || !applied.MixedReady || applied.PublicControllerAddress == nil || *applied.PublicControllerAddress != fmt.Sprintf("127.0.0.1:%d", public) {
		t.Fatal("ack differs from actual owner handles", applied)
	}
	if err = owner.close(); err != nil {
		t.Fatal(err)
	}
	if !handleStopListener() {
		t.Fatal("actual listener stop failed")
	}
	if _, err = owner.appliedServices(); err == nil {
		t.Fatal("retired owner still acknowledged listeners")
	}
	for _, port := range []int{benchmark, mixed, public} {
		listener, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
		if err != nil {
			t.Fatal("owned public/private-proxy listener not released", port, err)
		}
		_ = listener.Close()
	}
}
