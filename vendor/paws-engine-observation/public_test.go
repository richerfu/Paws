package main

import (
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"
)

func TestPublicControllerUsesSeparateCredentialAndRedactsPrivateConfig(t *testing.T) {
	private := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer private-owner" {
			t.Error("private credential not replaced")
			w.WriteHeader(401)
			return
		}
		if r.URL.Query().Get("token") != "" {
			t.Error("public query token leaked")
		}
		if r.URL.Path == "/configs" && r.Header.Get("Accept-Encoding") != "identity" {
			t.Error("config redaction did not request identity encoding")
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"mode":"rule","secret":"private-owner","external-controller":"127.0.0.1:19090","listeners":[{"port":18490}],"paws-runtime":{"secret":"private-owner"}}`)
	}))
	defer private.Close()
	reservation, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	port := reservation.Addr().(*net.TCPAddr).Port
	_ = reservation.Close()
	var services pawsNetworkServices
	services.Ports.ControllerEnabled = true
	services.Ports.ControllerPort = uint16(port)
	secret := "user-public"
	services.Access.Secret = &secret
	public, err := startPawsPublicServer(services, strings.TrimPrefix(private.URL, "http://"), "private-owner")
	if err != nil {
		t.Fatal(err)
	}
	defer public.close()
	client := &http.Client{Transport: &http.Transport{Proxy: nil}}
	base := "http://127.0.0.1:" + strconv.Itoa(port)
	for _, path := range []string{"/configs/private", "/paws/observations/logs", "/%70aws/observations/logs", "/proxies/../paws/observations/logs", "/proxies/%252e%252e/paws/observations/logs"} {
		req, _ := http.NewRequest("GET", base+path, nil)
		req.Header.Set("Authorization", "Bearer user-public")
		response, err := client.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		_ = response.Body.Close()
		if response.StatusCode < 400 {
			t.Fatalf("private path allowed: %s status %d", path, response.StatusCode)
		}
	}
	configRequest, _ := http.NewRequest("GET", base+"/configs?token=user-public", nil)
	configRequest.Header.Set("Accept-Encoding", "gzip")
	response, err := client.Do(configRequest)
	if err != nil {
		t.Fatal(err)
	}
	data, _ := io.ReadAll(response.Body)
	_ = response.Body.Close()
	if response.StatusCode != 200 || strings.Contains(string(data), "private-owner") || strings.Contains(string(data), "18490") || strings.Contains(string(data), "19090") || strings.Contains(string(data), "paws-runtime") {
		t.Fatalf("config not sanitized: status%d %s", response.StatusCode, data)
	}
	response, err = client.Get(base + "/configs")
	if err != nil {
		t.Fatal(err)
	}
	_ = response.Body.Close()
	if response.StatusCode != 401 {
		t.Fatal(response.StatusCode)
	}
	if err := public.close(); err != nil {
		t.Fatal(err)
	}
	response, err = client.Get(base + "/configs?token=user-public")
	if err == nil {
		_ = response.Body.Close()
		t.Fatal("stopped public controller remained reachable")
	}
}
