package compat_test

import (
	"bufio"
	"bytes"
	"context"
	"crypto/tls"
	"encoding/hex"
	"encoding/json"
	"io"
	"net/http"
	"os"
	"testing"
	"time"

	recovery "tinypaws.local/obfs-compat/tlsretry"
)

// Optional captures come only from meow_wire_test.rs using local-test-only.
// This fixture never loads a subscription, node file, or a live endpoint.
type meowCapture struct {
	Name        string   `json:"name"`
	HelloHex    string   `json:"helloHex"`
	WritesHex   []string `json:"writesHex"`
	EagerWrites int      `json:"eagerWrites"`
	ChunkSize   int      `json:"chunkSize"`
}

type decodedFrame struct {
	LengthStart  int `json:"lengthStart"`
	PayloadStart int `json:"payloadStart"`
	End          int `json:"end"`
	PlainBytes   int `json:"plainBytes"`
}

// Wire validity and replay eligibility are separate contracts. In particular,
// a real TLS 1.3 compatibility session ID is legal but conservatively ineligible.
func completeHelloRecord(hello []byte) bool {
	parsed := wireTLS(hello)
	return parsed.RecordComplete && parsed.RecordLength == len(hello)-5 &&
		parsed.HandshakeType == 1 && parsed.HandshakeLength == parsed.RecordLength-4 &&
		(parsed.Stage == "client-hello-parsed" || parsed.Stage == "client-hello-without-sni")
}

func completeClientHello(t *testing.T) []byte {
	t.Helper()
	raw := &wireConn{}
	conn := tls.Client(raw, &tls.Config{
		ServerName: "target.example", MinVersion: tls.VersionTLS12,
		MaxVersion: tls.VersionTLS13, CurvePreferences: []tls.CurveID{tls.X25519},
		SessionTicketsDisabled: true, InsecureSkipVerify: true, // Local capture only.
	})
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	if conn.HandshakeContext(ctx) == nil {
		t.Fatal("capture-only connection unexpectedly completed TLS")
	}
	hello := bytes.Join(raw.writes, nil)
	if !completeHelloRecord(hello) {
		t.Fatal("crypto/tls did not emit exactly one complete ClientHello")
	}
	return hello
}

func destinationBytes() []byte {
	destination := append([]byte{3, 14}, []byte("target.example")...)
	return append(destination, 1, 187)
}

func inspectTLSWrites(t *testing.T, name string, writes [][]byte, eagerWrites int, hello []byte, chunkSize int) {
	t.Helper()
	wire := bytes.Join(writes, nil)
	headerEnd := bytes.Index(wire, []byte("\r\n\r\n")) + 4
	if headerEnd < 4 {
		t.Fatal("missing HTTP envelope")
	}
	req, err := http.ReadRequest(bufio.NewReader(bytes.NewReader(wire)))
	if err != nil || req.Method != "GET" || req.Host != "cover.example:443" {
		t.Fatal("invalid HTTP-obfs envelope", err)
	}
	body := wire[headerEnd:]
	if len(body) < 16 {
		t.Fatal("missing AEAD salt")
	}
	ss, err := newIndependentSS(body[:16])
	if err != nil {
		t.Fatal(err)
	}
	r := bytes.NewReader(body[16:])
	var frames []decodedFrame
	var plain [][]byte
	for r.Len() > 0 {
		start := len(body) - r.Len()
		chunk, err := ss.read(r)
		if err != nil {
			t.Fatal("independent AES-GCM authentication failed", err)
		}
		frames = append(frames, decodedFrame{start, start + 18, len(body) - r.Len(), len(chunk)})
		plain = append(plain, chunk)
	}
	expected := [][]byte{destinationBytes()}
	for p := hello; len(p) > 0; {
		n := min(chunkSize, len(p))
		expected = append(expected, p[:n])
		p = p[n:]
	}
	if eagerWrites == 0 { // Explicit historical early control, not production Mihomo.
		expected[1] = append(bytes.Clone(expected[0]), expected[1]...)
		expected = expected[1:]
	} else {
		initial := bytes.Join(writes[:eagerWrites], nil)
		if len(initial)-headerEnd != 68 || req.ContentLength != 68 {
			t.Fatal("eager envelope must contain only salt and the destination record")
		}
	}
	if len(plain) != len(expected) {
		t.Fatalf("record count=%d, want %d", len(plain), len(expected))
	}
	for i := range plain {
		if !bytes.Equal(plain[i], expected[i]) {
			t.Fatalf("plaintext record %d differs from destination/ClientHello chunk", i)
		}
	}
	var sizes []int
	histogram := map[int]int{}
	var boundaries []int
	position := 0
	for _, write := range writes {
		position += len(write)
		sizes = append(sizes, len(write))
		histogram[len(write)]++
		boundaries = append(boundaries, position-headerEnd)
	}
	seams := 0
	for _, frame := range frames {
		for _, boundary := range boundaries {
			if boundary == frame.PayloadStart {
				seams++
			}
		}
	}
	var writeReport any = sizes
	if len(sizes) > 64 {
		writeReport = map[string]any{"count": len(sizes), "histogram": histogram}
	}
	mode := "eager"
	if eagerWrites == 0 {
		mode = "early-control"
	}
	report, _ := json.Marshal(map[string]any{
		"engine": name, "handshakeMode": mode, "helloBytes": len(hello),
		"helloReplayEligible":  recovery.ValidFlight(hello),
		"initialHTTPBodyBytes": req.ContentLength, "rawWriteSizes": writeReport,
		"frames": frames, "writesEndingBetweenLengthAndPayload": seams,
		"authenticated": true, "scope": "local Write calls, not TCP packet boundaries",
	})
	t.Log(string(report))
}

func TestTLSFlightFraming(t *testing.T) {
	hello := completeClientHello(t)
	var captures []meowCapture
	if path := os.Getenv("TINYPAWS_MEOW_WIRE_FIXTURE"); path != "" {
		file, err := os.Open(path)
		if err != nil {
			t.Fatal("cannot open local Meow fixture")
		}
		defer file.Close()
		if err := json.NewDecoder(io.LimitReader(file, 1024*1024)).Decode(&captures); err != nil || len(captures) == 0 {
			t.Fatal("invalid local Meow fixture")
		}
		hello, err = hex.DecodeString(captures[0].HelloHex)
		if err != nil || !completeHelloRecord(hello) {
			t.Fatal("Meow fixture lacks a complete ClientHello record")
		}
	}
	if path := os.Getenv("TINYPAWS_HELLO_FIXTURE_OUT"); path != "" {
		if err := os.WriteFile(path, hello, 0600); err != nil {
			t.Fatal("cannot write local ClientHello fixture")
		}
	}
	for _, candidate := range clients {
		for _, chunkSize := range []int{len(hello), 7} {
			t.Run(candidate.name+"/chunk-"+jsonInt(chunkSize), func(t *testing.T) {
				raw := &wireConn{}
				conn := candidate.encrypted(t, candidate.obfs(raw))
				eagerWrites := len(raw.writes)
				if candidate.name == "mihomo" && eagerWrites == 0 {
					t.Fatal("Mihomo fixture diverged from the real adapter's eager path")
				}
				for p := hello; len(p) > 0; {
					n := min(chunkSize, len(p))
					if got, err := conn.Write(p[:n]); got != n || err != nil {
						t.Fatal("ClientHello write failed", got, err)
					}
					p = p[n:]
				}
				inspectTLSWrites(t, candidate.name, raw.writes, eagerWrites, hello, chunkSize)
			})
		}
	}
	for _, capture := range captures {
		t.Run(capture.Name, func(t *testing.T) {
			if capture.HelloHex != hex.EncodeToString(hello) || capture.ChunkSize < 1 {
				t.Fatal("Meow fixture inputs differ")
			}
			var writes [][]byte
			for _, encoded := range capture.WritesHex {
				write, err := hex.DecodeString(encoded)
				if err != nil {
					t.Fatal("invalid captured write")
				}
				writes = append(writes, write)
			}
			if capture.EagerWrites < 1 || capture.EagerWrites > len(writes) {
				t.Fatal("invalid Meow eager capture")
			}
			inspectTLSWrites(t, capture.Name, writes, capture.EagerWrites, hello, capture.ChunkSize)
		})
	}
}

func jsonInt(n int) string {
	b, _ := json.Marshal(n)
	return string(b)
}
