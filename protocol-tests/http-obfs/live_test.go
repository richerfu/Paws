package compat_test

import (
	"bufio"
	"context"
	"crypto/tls"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"os"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	mihomoObfs "github.com/metacubex/mihomo/transport/simple-obfs"
	mihomoCipher "github.com/metacubex/sing-shadowsocks2/cipher"
	mihomoMetadata "github.com/metacubex/sing/common/metadata"
	singObfs "github.com/sagernet/sing-box/transport/simple-obfs"
	singCipher "github.com/sagernet/sing-shadowsocks2/cipher"
	singMetadata "github.com/sagernet/sing/common/metadata"
)

type traceConn struct {
	net.Conn
	mu                sync.Mutex
	sent, received    int
	writes, reads     []int
	readEnd, writeEnd string
}

func endKind(err error) string {
	switch {
	case err == nil:
		return ""
	case errors.Is(err, io.EOF):
		return "peer-fin"
	case errors.Is(err, syscall.ECONNRESET):
		return "peer-reset"
	case errors.Is(err, syscall.EPIPE):
		return "write-broken-pipe"
	default:
		if e, ok := err.(net.Error); ok && e.Timeout() {
			return "timeout"
		}
		return "io-error"
	}
}
func (c *traceConn) Read(p []byte) (int, error) {
	n, e := c.Conn.Read(p)
	c.mu.Lock()
	defer c.mu.Unlock()
	c.received += n
	c.reads = append(c.reads, n)
	if e != nil {
		c.readEnd = endKind(e)
	}
	return n, e
}
func (c *traceConn) Write(p []byte) (int, error) {
	n, e := c.Conn.Write(p)
	c.mu.Lock()
	defer c.mu.Unlock()
	c.sent += n
	c.writes = append(c.writes, n)
	if e != nil {
		c.writeEnd = endKind(e)
	}
	return n, e
}

func TestProviderSession(t *testing.T) {
	path := os.Getenv("TINYPAWS_LIVE_NODE_PATH")
	if path == "" {
		t.Skip("live provider test is opt-in")
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal("cannot read private node file")
	}
	var node struct{ Server, Port, Method, Password, Host, Mode string }
	if json.Unmarshal(data, &node) != nil {
		t.Fatal("invalid private node file")
	}
	if node.Mode != "http" {
		t.Fatal("this test isolates HTTP-obfs")
	}
	target := "cp.cloudflare.com"
	for round := 0; round < 5; round++ {
		for _, engine := range []string{"mihomo", "sing-box", "sing-box-eager"} {
			t.Run(engine+"/"+string(rune('0'+round)), func(t *testing.T) {
				started := time.Now()
				ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
				defer cancel()
				raw, err := (&net.Dialer{}).DialContext(ctx, "tcp", net.JoinHostPort(node.Server, node.Port))
				if err != nil {
					t.Fatal("node dial failed: " + endKind(err))
				}
				defer raw.Close()
				_ = raw.SetDeadline(time.Now().Add(10 * time.Second))
				trace := &traceConn{Conn: raw}
				var encrypted net.Conn
				if engine == "mihomo" {
					m, e := mihomoCipher.CreateMethod(node.Method, mihomoCipher.MethodOptions{Password: node.Password})
					if e != nil {
						t.Fatal("cipher creation failed")
					}
					encrypted, err = m.DialConn(mihomoObfs.NewHTTPObfs(trace, node.Host, node.Port), mihomoMetadata.ParseSocksaddr(target+":443"))
					if err != nil {
						t.Fatal("eager handshake failed: " + endKind(err))
					}
				} else {
					m, e := singCipher.CreateMethod(ctx, node.Method, singCipher.MethodOptions{Password: node.Password})
					if e != nil {
						t.Fatal("cipher creation failed")
					}
					encrypted = m.DialEarlyConn(singObfs.NewHTTPObfs(trace, node.Host, node.Port), singMetadata.ParseSocksaddr(target+":443"))
				}
				if engine == "sing-box-eager" {
					if _, err = encrypted.Write(nil); err != nil {
						t.Fatal("eager handshake failed")
					}
				}
				tlsConn := tls.Client(encrypted, &tls.Config{ServerName: target, MinVersion: tls.VersionTLS12, CurvePreferences: []tls.CurveID{tls.X25519, tls.CurveP256}})
				stage := "tls"
				err = tlsConn.HandshakeContext(ctx)
				status := 0
				bodyBytes := 0
				if err == nil {
					stage = "https"
					req, _ := http.NewRequest("GET", "https://"+target+"/generate_204", nil)
					req.Close = true
					err = req.Write(tlsConn)
					if err == nil {
						var response *http.Response
						response, err = http.ReadResponse(bufio.NewReader(tlsConn), req)
						if err == nil {
							status = response.StatusCode
							body, _ := io.ReadAll(io.LimitReader(response.Body, 65536))
							bodyBytes = len(body)
							_ = response.Body.Close()
						}
					}
				}
				trace.mu.Lock()
				report := map[string]any{"engine": engine, "round": round, "stage": stage, "success": err == nil && status >= 200 && status < 400, "status": status, "durationMs": time.Since(started).Milliseconds(), "sentBytes": trace.sent, "receivedBytes": trace.received, "writeSizes": trace.writes, "readSizes": trace.reads, "readEnd": trace.readEnd, "writeEnd": trace.writeEnd, "errorClass": endKind(err), "bodyBytes": bodyBytes, "closeInitiator": map[bool]string{true: "local-after-result", false: "see-readEnd"}[trace.readEnd == ""], "scope": "host TCP syscall observation; not packet capture"}
				trace.mu.Unlock()
				encoded, _ := json.Marshal(report)
				t.Log(string(encoded))
				if err != nil || status < 200 || status >= 400 {
					t.Errorf("provider session failed at %s (%s)", stage, strings.TrimSpace(endKind(err)))
				}
			})
		}
	}
}
