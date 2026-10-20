package compat_test

import (
	"bytes"
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/md5"
	"crypto/sha1"
	"encoding/binary"
	"encoding/json"
	"io"
	"net"
	"strings"
	"testing"
	"time"

	compatibleObfs "tinypaws.local/obfs-compat/httpobfs"

	mihomoObfs "github.com/metacubex/mihomo/transport/simple-obfs"
	mihomoCipher "github.com/metacubex/sing-shadowsocks2/cipher"
	_ "github.com/metacubex/sing-shadowsocks2/shadowaead"
	mihomoMetadata "github.com/metacubex/sing/common/metadata"
	singObfs "github.com/sagernet/sing-box/transport/simple-obfs"
	singCipher "github.com/sagernet/sing-shadowsocks2/cipher"
	_ "github.com/sagernet/sing-shadowsocks2/shadowaead"
	singMetadata "github.com/sagernet/sing/common/metadata"
	"golang.org/x/crypto/hkdf"
)

// These connections test byte-stream behavior deterministically. Write call
// boundaries below are raw transport writes, not claimed TCP packet boundaries.
type wireConn struct {
	writes [][]byte
	reads  [][]byte
}

func (c *wireConn) Read(p []byte) (int, error) {
	if len(c.reads) == 0 {
		return 0, io.EOF
	}
	n := copy(p, c.reads[0])
	c.reads[0] = c.reads[0][n:]
	if len(c.reads[0]) == 0 {
		c.reads = c.reads[1:]
	}
	return n, nil
}
func (c *wireConn) Write(p []byte) (int, error) {
	c.writes = append(c.writes, bytes.Clone(p))
	return len(p), nil
}
func (*wireConn) Close() error                     { return nil }
func (*wireConn) LocalAddr() net.Addr              { return &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 1} }
func (*wireConn) RemoteAddr() net.Addr             { return &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 2} }
func (*wireConn) SetDeadline(time.Time) error      { return nil }
func (*wireConn) SetReadDeadline(time.Time) error  { return nil }
func (*wireConn) SetWriteDeadline(time.Time) error { return nil }

type client struct {
	name      string
	obfs      func(net.Conn) net.Conn
	encrypted func(*testing.T, net.Conn) net.Conn
}

var clients = []client{
	{"mihomo", func(c net.Conn) net.Conn { return mihomoObfs.NewHTTPObfs(c, "cover.example", "443") }, func(t *testing.T, c net.Conn) net.Conn {
		method, err := mihomoCipher.CreateMethod("aes-128-gcm", mihomoCipher.MethodOptions{Password: "local-test-only"})
		if err != nil {
			t.Fatal(err)
		}
		// The real HTTP-obfs adapter has useEarly=false and NeedHandshake=false.
		conn, err := method.DialConn(c, mihomoMetadata.ParseSocksaddr("target.example:443"))
		if err != nil {
			t.Fatal(err)
		}
		return conn
	}},
	{"sing-box", func(c net.Conn) net.Conn { return singObfs.NewHTTPObfs(c, "cover.example", "443") }, func(t *testing.T, c net.Conn) net.Conn {
		method, err := singCipher.CreateMethod(context.Background(), "aes-128-gcm", singCipher.MethodOptions{Password: "local-test-only"})
		if err != nil {
			t.Fatal(err)
		}
		return method.DialEarlyConn(c, singMetadata.ParseSocksaddr("target.example:443"))
	}},
	{"sing-box-compatible", func(c net.Conn) net.Conn {
		wrapped, err := compatibleObfs.New(c, "cover.example", "443")
		if err != nil {
			panic(err)
		}
		return wrapped
	}, func(t *testing.T, c net.Conn) net.Conn {
		method, err := singCipher.CreateMethod(context.Background(), "aes-128-gcm", singCipher.MethodOptions{Password: "local-test-only"})
		if err != nil {
			t.Fatal(err)
		}
		conn, err := method.DialConn(c, singMetadata.ParseSocksaddr("target.example:443"))
		if err != nil {
			t.Fatal(err)
		}
		return conn
	}},
}

// Independent AEAD decoding verifies actual encryption/framing rather than
// accepting a write to a listening socket as a successful protocol session.
func decrypt(t *testing.T, wire []byte) []byte {
	t.Helper()
	if len(wire) < 16 {
		t.Fatal("missing salt")
	}
	hash := md5.Sum([]byte("local-test-only"))
	subkey := make([]byte, 16)
	if _, err := io.ReadFull(hkdf.New(sha1.New, hash[:], wire[:16], []byte("ss-subkey")), subkey); err != nil {
		t.Fatal(err)
	}
	block, err := aes.NewCipher(subkey)
	if err != nil {
		t.Fatal(err)
	}
	aead, err := cipher.NewGCM(block)
	if err != nil {
		t.Fatal(err)
	}
	nonce := make([]byte, aead.NonceSize())
	wire = wire[16:]
	next := func() {
		for i := range nonce {
			nonce[i]++
			if nonce[i] != 0 {
				break
			}
		}
	}
	var plain []byte
	for len(wire) > 0 {
		if len(wire) < 2+aead.Overhead() {
			t.Fatal("truncated encrypted length")
		}
		length, err := aead.Open(nil, nonce, wire[:2+aead.Overhead()], nil)
		if err != nil {
			t.Fatal(err)
		}
		next()
		size := int(binary.BigEndian.Uint16(length))
		wire = wire[2+aead.Overhead():]
		if len(wire) < size+aead.Overhead() {
			t.Fatal("truncated encrypted payload")
		}
		payload, err := aead.Open(nil, nonce, wire[:size+aead.Overhead()], nil)
		if err != nil {
			t.Fatal(err)
		}
		next()
		plain = append(plain, payload...)
		wire = wire[size+aead.Overhead():]
	}
	return plain
}

func TestFirstRequest(t *testing.T) {
	for _, client := range clients {
		t.Run(client.name, func(t *testing.T) {
			raw := &wireConn{}
			conn := client.encrypted(t, client.obfs(raw))
			payload := []byte("fixed-client-first-payload")
			if n, err := conn.Write(payload); err != nil || n != len(payload) {
				t.Fatalf("write n=%d error=%v", n, err)
			}
			wire := bytes.Join(raw.writes, nil)
			split := bytes.Index(wire, []byte("\r\n\r\n"))
			if split < 0 {
				t.Fatal("missing header terminator")
			}
			lines := strings.Split(string(wire[:split]), "\r\n")
			if lines[0] != "GET / HTTP/1.1" {
				t.Fatal(lines[0])
			}
			var headers []string
			for _, line := range lines[1:] {
				name, value, found := strings.Cut(line, ": ")
				if !found {
					t.Fatalf("bad header %q", line)
				}
				if name == "User-Agent" {
					value = "curl/7.<random>.<random>"
				}
				if strings.EqualFold(name, "Sec-WebSocket-Key") {
					value = "<random>"
				}
				headers = append(headers, name+": "+value)
			}
			body := wire[split+4:]
			plain := decrypt(t, body)
			expected := append([]byte{3, 14}, []byte("target.example")...)
			expected = append(expected, 1, 187)
			expected = append(expected, payload...)
			if !bytes.Equal(plain, expected) {
				t.Fatalf("decrypted request mismatch: got %x", plain)
			}
			sizes := make([]int, len(raw.writes))
			for i, w := range raw.writes {
				sizes[i] = len(w)
			}
			report, _ := json.Marshal(map[string]any{"engine": client.name, "methodPath": lines[0], "headers": headers, "cipherBytes": len(body), "plaintextVerified": true, "rawWriteSizes": sizes})
			t.Log(string(report))
		})
	}
}

func TestFragmentedResponse(t *testing.T) {
	header := []byte("HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n")
	body := []byte("encrypted-response-placeholder")
	for _, client := range clients {
		for _, scenario := range []string{"coalesced", "split-header", "header-only-first-read"} {
			t.Run(client.name+"/"+scenario, func(t *testing.T) {
				raw := &wireConn{}
				switch scenario {
				case "coalesced":
					raw.reads = [][]byte{append(bytes.Clone(header), body...)}
				case "split-header":
					raw.reads = [][]byte{bytes.Clone(header[:17]), append(bytes.Clone(header[17:]), body...)}
				case "header-only-first-read":
					raw.reads = [][]byte{bytes.Clone(header), bytes.Clone(body)}
				}
				conn := client.obfs(raw)
				got, err := io.ReadAll(conn)
				if err != nil || !bytes.Equal(got, body) {
					t.Fatalf("valid fragmented response rejected: bytes=%d error=%v", len(got), err)
				}
			})
		}
	}
}
