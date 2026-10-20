package compat_test

import (
	"bufio"
	"bytes"
	"context"
	"crypto/rand"
	"fmt"
	cipher "github.com/sagernet/sing-shadowsocks2/cipher"
	_ "github.com/sagernet/sing-shadowsocks2/shadowaead"
	M "github.com/sagernet/sing/common/metadata"
	"io"
	"net"
	"net/http"
	"sync/atomic"
	"testing"
	"time"
	compatible "tinypaws.local/obfs-compat/httpobfs"
	recovery "tinypaws.local/obfs-compat/tlsretry"
)

func TestFirstFlightAEADRecovery(t *testing.T) {
	method, err := cipher.CreateMethod(context.Background(), "aes-128-gcm", cipher.MethodOptions{Password: "local-test-only"})
	if err != nil {
		t.Fatal(err)
	}
	body := append([]byte{3, 3}, make([]byte, 32)...)
	body = append(body, 0, 0, 2, 0x13, 1, 1, 0)
	n := len(body) + 4
	hello := append([]byte{22, 3, 1, byte(n >> 8), byte(n), 1, 0, 0, byte(len(body))}, body...)
	salts := make(chan []byte, 2)
	outcomes := make(chan error, 2)
	attempts := &atomic.Int32{}
	dial := func(ctx context.Context) (net.Conn, error) {
		attempt := attempts.Add(1)
		client, server := net.Pipe()
		deadline := time.Now().Add(3 * time.Second)
		_ = client.SetDeadline(deadline)
		_ = server.SetDeadline(deadline)
		go func() {
			defer server.Close()
			outcomes <- func() error {
				r := bufio.NewReader(server)
				request, e := http.ReadRequest(r)
				if e != nil {
					return e
				}
				first, e := io.ReadAll(request.Body)
				if e != nil {
					return e
				}
				wire := io.MultiReader(bytes.NewReader(first), r)
				salt := make([]byte, 16)
				if _, e = io.ReadFull(wire, salt); e != nil {
					return e
				}
				salts <- salt
				decrypt, e := newIndependentSS(salt)
				if e != nil {
					return e
				}
				destination, e := decrypt.read(wire)
				if e != nil {
					return e
				}
				expected := append([]byte{3, 14}, []byte("target.example")...)
				expected = append(expected, 1, 187)
				if !bytes.Equal(destination, expected) {
					return fmt.Errorf("destination was not eager and exact")
				}
				payload, e := decrypt.read(wire)
				if e != nil {
					return e
				}
				if !bytes.Equal(payload, hello) {
					return fmt.Errorf("only ClientHello may be replayed")
				}
				if attempt == 1 {
					return nil
				} // Deliberate zero-response FIN.
				salt = make([]byte, 16)
				if _, e = rand.Read(salt); e != nil {
					return e
				}
				encrypt, e := newIndependentSS(salt)
				if e != nil {
					return e
				}
				reply := append([]byte("HTTP/1.1 101 Switching Protocols\r\n\r\n"), salt...)
				reply = append(reply, encrypt.seal([]byte("TLS"))...)
				_, e = server.Write(reply)
				return e
			}()
		}()
		transport, e := compatible.New(client, "cover.example", "443")
		if e != nil {
			return nil, e
		}
		return recovery.HandshakeContext(ctx, transport, func() (net.Conn, error) {
			return method.DialConn(transport, M.ParseSocksaddr("target.example:443"))
		})
	}
	initial, err := dial(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	conn := recovery.New(context.Background(), initial, dial, nil)
	defer conn.Close()
	if _, err = conn.Write(hello); err != nil {
		t.Fatal(err)
	}
	reply := make([]byte, 3)
	if _, err = io.ReadFull(conn, reply); err != nil {
		t.Fatal(err)
	}
	if string(reply) != "TLS" {
		t.Fatal("wrong authenticated reply")
	}
	if attempts.Load() != 2 {
		t.Fatal("wrong attempt count", attempts.Load())
	}
	if bytes.Equal(<-salts, <-salts) {
		t.Fatal("reconnect reused AEAD salt")
	}
	for i := 0; i < 2; i++ {
		if err = <-outcomes; err != nil {
			t.Fatal(err)
		}
	}
}

// Use the unchanged crypto/tls ClientHello from the framing fixture. Its legacy
// session ID is legal on the wire but cannot prove a fresh, non-resumed session.
func TestRealClientHelloWireValidityDoesNotImplyReplayEligibility(t *testing.T) {
	hello := completeClientHello(t)
	if !completeHelloRecord(hello) || len(hello) < 44 || hello[43] == 0 {
		t.Fatal("fixture lost the real crypto/tls compatibility session ID")
	}
	if recovery.ValidFlight(hello) {
		t.Fatal("ambiguous legacy session ID is replay eligible")
	}
	a, b := net.Pipe()
	var retries atomic.Int32
	conn := recovery.New(context.Background(), a, func(context.Context) (net.Conn, error) {
		retries.Add(1)
		return nil, fmt.Errorf("unexpected replay of real ClientHello")
	}, nil)
	defer conn.Close()
	got := make(chan []byte, 1)
	go func() {
		defer b.Close()
		wire := make([]byte, len(hello))
		_, _ = io.ReadFull(b, wire)
		got <- wire
	}()
	if _, e := conn.Write(hello); e != nil {
		t.Fatal(e)
	}
	if _, e := conn.Read(make([]byte, 1)); e != io.EOF {
		t.Fatal(e)
	}
	if !bytes.Equal(<-got, hello) {
		t.Fatal("wire ClientHello changed")
	}
	if retries.Load() != 0 {
		t.Fatal("real ClientHello was replayed")
	}
}
