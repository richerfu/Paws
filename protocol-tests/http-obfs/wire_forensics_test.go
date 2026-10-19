package compat_test

import (
	"bufio"
	"bytes"
	"crypto/aes"
	"crypto/cipher"
	"crypto/hmac"
	"crypto/md5"
	"crypto/sha1"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"testing"
)

const (
	wireLogLimit    = 16 << 20
	wireSocketLimit = 2048
	wireMaxSockets  = 4096
)

// This report deliberately has no raw events, flow labels, endpoint, password,
// salt, keys, HTTP values, SNI, or decrypted bytes. Hashes cover captured bytes,
// not a claim about bytes outside the capture window or TCP packet boundaries.
type wireSocketID struct {
	PID int64 `json:"pid"`
	ID  int64 `json:"id"`
}
type wireRange struct {
	Start int `json:"start"`
	End   int `json:"end"`
}
type wireHTTPReport struct {
	Complete                        bool     `json:"complete"`
	GETRootHTTP11                   bool     `json:"getRootHTTP11"`
	HeaderOrder                     []string `json:"headerOrder"`
	HostSHA256                      string   `json:"hostSHA256,omitempty"`
	UserAgent                       string   `json:"normalizedUserAgent,omitempty"`
	ContentLength                   *int64   `json:"contentLength,omitempty"`
	UpgradeCorrect                  bool     `json:"upgradeCorrect"`
	ConnectionCorrect               bool     `json:"connectionCorrect"`
	WebsocketKeyValidURLSafe16B     bool     `json:"websocketKeyValidURLSafe16B"`
	WebsocketKeyValidStandardBase64 bool     `json:"websocketKeyValidStandardBase64"`
	WebsocketKeyHasURLOnlyChars     bool     `json:"websocketKeyHasUrlOnlyChars"`
	Issues                          []string `json:"issues,omitempty"`
}
type wireTLSReport struct {
	Stage              string `json:"stage"`
	RecordType         int    `json:"recordType,omitempty"`
	RecordLength       int    `json:"recordLength,omitempty"`
	RecordComplete     bool   `json:"recordComplete"`
	HandshakeType      int    `json:"handshakeType,omitempty"`
	HandshakeLength    int    `json:"handshakeLength,omitempty"`
	SNIPresent         bool   `json:"sniPresent"`
	SNIMatchesExpected bool   `json:"sniMatchesCpCloudflareCom"`
}
type wireSocketReport struct {
	wireSocketID
	Samples                int            `json:"samples"`
	SampledBytes           int            `json:"sampledBytes"`
	ObservedEnd            int64          `json:"observedEnd"`
	UniqueBytes            int            `json:"uniqueBytes"`
	ContiguousBytes        int            `json:"contiguousBytes"`
	DuplicateSamples       int            `json:"duplicateSamples"`
	OverlapBytes           int            `json:"overlapBytes"`
	ConflictingBytes       int            `json:"conflictingBytes"`
	OutOfOrderSamples      int            `json:"outOfOrderSamples"`
	MissingRanges          []wireRange    `json:"missingRanges,omitempty"`
	CaptureTruncated       bool           `json:"captureTruncated"`
	CaptureAtLimit         bool           `json:"captureAtLimit"`
	WireSHA256             string         `json:"wireSHA256"`
	WireHashScope          string         `json:"wireHashScope"`
	HTTP                   wireHTTPReport `json:"http"`
	SaltSHA256             string         `json:"saltSHA256,omitempty"`
	Authentication         string         `json:"authentication"`
	Stage                  string         `json:"stage"`
	FailureRecord          *int           `json:"failureRecord,omitempty"`
	RecordPlaintextLengths []int          `json:"recordPlaintextLengths"`
	DestinationSHA256      string         `json:"destinationSHA256,omitempty"`
	DestinationIsExpected  bool           `json:"destinationIsCpCloudflareCom443"`
	TLS                    wireTLSReport  `json:"tls"`
}
type wireSummary struct {
	Sockets                       int `json:"sockets"`
	Samples                       int `json:"samples"`
	MalformedLines                int `json:"malformedLines"`
	MalformedSamples              int `json:"malformedSamples"`
	MissingOrInvalidOffsetSamples int `json:"missingOrInvalidOffsetSamples"`
	SocketLimitDrops              int `json:"socketLimitDrops"`
	AuthenticatedSockets          int `json:"authenticatedSockets"`
	PartialSockets                int `json:"partialSockets"`
	MACFailureSockets             int `json:"macFailureSockets"`
	GapSockets                    int `json:"gapSockets"`
	OverlapSockets                int `json:"overlapSockets"`
	ConflictingSockets            int `json:"conflictingSockets"`
	TruncatedSockets              int `json:"truncatedSockets"`
	AtLimitSockets                int `json:"atLimitSockets"`
	DuplicateSaltGroups           int `json:"duplicateSaltGroups"`
	ExpectedDestinationSockets    int `json:"expectedDestinationSockets"`
	ExpectedSNISockets            int `json:"expectedSNISockets"`
}
type wireReport struct {
	Format                string             `json:"format"`
	CaptureLimitPerSocket int                `json:"captureLimitPerSocket"`
	Summary               wireSummary        `json:"summary"`
	Sockets               []wireSocketReport `json:"sockets"`
	DuplicateSalt         [][]wireSocketID   `json:"duplicateSalt"`
}
type wireCapture struct {
	wireSocketReport
	data       [wireSocketLimit]byte
	present    [wireSocketLimit]bool
	lastOffset int64
}
type wirePrivateNode struct {
	Cipher   string          `json:"cipher"`
	Password string          `json:"password"`
	Server   string          `json:"server"`
	Port     json.RawMessage `json:"port"`
}

var wireIDPattern = regexp.MustCompile(`(?:^|[\s,])id=([0-9]+)(?:[\s,]|$)`)
var wireOffsetPattern = regexp.MustCompile(`(?:^|[\s,])offset=([0-9]+)(?:[\s,]|$)`)
var wireHexPattern = regexp.MustCompile(`(?:^|[\s,])hex=([0-9a-fA-F]*)(?:[\s,]|$)`)
var wireCurlPattern = regexp.MustCompile(`^curl/7\.[0-9]+\.[0-9]+$`)

func wireHash(p []byte) string { sum := sha256.Sum256(p); return hex.EncodeToString(sum[:]) }
func wireMatchInt(re *regexp.Regexp, event string) (int64, bool) {
	m := re.FindStringSubmatch(event)
	if len(m) != 2 {
		return 0, false
	}
	n, err := strconv.ParseInt(m[1], 10, 64)
	return n, err == nil
}

func wireReadCaptures(raw []byte) (map[wireSocketID]*wireCapture, wireSummary) {
	captures := make(map[wireSocketID]*wireCapture)
	var summary wireSummary
	scanner := bufio.NewScanner(bytes.NewReader(raw))
	scanner.Buffer(make([]byte, 4096), wireLogLimit+1)
	for scanner.Scan() {
		var line struct {
			AtMs    json.RawMessage `json:"atMs"`
			PID     int64           `json:"pid"`
			Flow    json.RawMessage `json:"flow"`
			Event   string          `json:"event"`
			Message string          `json:"message"`
		}
		if json.Unmarshal(scanner.Bytes(), &line) != nil {
			summary.MalformedLines++
			continue
		}
		if line.Event == "" {
			line.Event = line.Message
		}
		if !strings.Contains(line.Event, "socket-trace") || !strings.Contains(line.Event, "event=wire-write-sample") {
			continue
		}
		id, okID := wireMatchInt(wireIDPattern, line.Event)
		offset, okOffset := wireMatchInt(wireOffsetPattern, line.Event)
		m := wireHexPattern.FindStringSubmatch(line.Event)
		if !okOffset {
			summary.MissingOrInvalidOffsetSamples++
		}
		if !okID || !okOffset || line.PID <= 0 || len(m) != 2 || len(m[1])%2 != 0 {
			summary.MalformedSamples++
			continue
		}
		key := wireSocketID{PID: line.PID, ID: id}
		c := captures[key]
		if c == nil {
			if len(captures) >= wireMaxSockets {
				summary.SocketLimitDrops++
				continue
			}
			c = &wireCapture{wireSocketReport: wireSocketReport{wireSocketID: key, RecordPlaintextLengths: []int{}}, lastOffset: -1}
			captures[key] = c
		}
		count := len(m[1]) / 2
		// A valid int64 offset plus a bounded log length cannot wrap silently.
		if offset > int64(^uint64(0)>>1)-int64(count) {
			summary.MalformedSamples++
			continue
		}
		summary.Samples++
		c.Samples++
		c.SampledBytes += count
		if c.lastOffset >= 0 && offset < c.lastOffset {
			c.OutOfOrderSamples++
		}
		c.lastOffset = offset
		end := offset + int64(count)
		if end > c.ObservedEnd {
			c.ObservedEnd = end
		}
		if end >= wireSocketLimit {
			c.CaptureAtLimit = true
		}
		if end > wireSocketLimit {
			c.CaptureTruncated = true
		}
		kept := count
		if offset >= wireSocketLimit {
			kept = 0
		} else if kept > wireSocketLimit-int(offset) {
			kept = wireSocketLimit - int(offset)
		}
		part, err := hex.DecodeString(m[1][:kept*2])
		if err != nil {
			summary.MalformedSamples++
			continue
		}
		duplicate := kept > 0 && kept == count
		for i, b := range part {
			pos := int(offset) + i
			if c.present[pos] {
				c.OverlapBytes++
				if c.data[pos] != b {
					c.ConflictingBytes++
					duplicate = false
				}
			} else {
				duplicate = false
				c.present[pos] = true
				c.data[pos] = b
				c.UniqueBytes++
			}
		}
		if duplicate {
			c.DuplicateSamples++
		}
	}
	if scanner.Err() != nil {
		summary.MalformedLines++
	}
	return captures, summary
}

func wireHTTP(data []byte) (wireHTTPReport, int) {
	r := wireHTTPReport{HeaderOrder: []string{}}
	end := bytes.Index(data, []byte("\r\n\r\n"))
	if end < 0 {
		r.Issues = append(r.Issues, "incomplete-header")
		return r, -1
	}
	r.Complete = true
	lines := strings.Split(string(data[:end]), "\r\n")
	r.GETRootHTTP11 = lines[0] == "GET / HTTP/1.1"
	if !r.GETRootHTTP11 {
		r.Issues = append(r.Issues, "unexpected-request-line")
	}
	seen := map[string]bool{}
	for _, line := range lines[1:] {
		name, value, found := strings.Cut(line, ":")
		if !found || name == "" || strings.ContainsAny(name, " \t") {
			r.Issues = append(r.Issues, "malformed-header")
			continue
		}
		name = strings.ToLower(name)
		value = strings.TrimSpace(value)
		if seen[name] {
			r.Issues = append(r.Issues, "duplicate-header")
		}
		seen[name] = true
		safeName := map[string]string{"host": "Host", "user-agent": "User-Agent", "content-length": "Content-Length", "connection": "Connection", "upgrade": "Upgrade", "sec-websocket-key": "Sec-WebSocket-Key", "accept": "Accept", "sec-websocket-version": "Sec-WebSocket-Version"}[name]
		if safeName == "" {
			safeName = "<other>"
		}
		r.HeaderOrder = append(r.HeaderOrder, safeName)
		switch name {
		case "host":
			r.HostSHA256 = wireHash([]byte(value))
		case "user-agent":
			if wireCurlPattern.MatchString(value) {
				r.UserAgent = "curl/7.<random>.<random>"
			} else {
				r.UserAgent = "<unrecognized>"
			}
		case "content-length":
			n, err := strconv.ParseInt(value, 10, 64)
			if err == nil && n >= 0 {
				r.ContentLength = &n
			} else {
				r.Issues = append(r.Issues, "invalid-content-length")
			}
		case "upgrade":
			r.UpgradeCorrect = strings.EqualFold(value, "websocket")
		case "connection":
			for _, token := range strings.Split(value, ",") {
				if strings.EqualFold(strings.TrimSpace(token), "upgrade") {
					r.ConnectionCorrect = true
				}
			}
		case "sec-websocket-key":
			p, err := base64.URLEncoding.Strict().DecodeString(value)
			if err == nil && len(p) == 16 && base64.URLEncoding.EncodeToString(p) == value {
				r.WebsocketKeyValidURLSafe16B = true
			}
			p, err = base64.RawURLEncoding.Strict().DecodeString(value)
			if err == nil && len(p) == 16 && base64.RawURLEncoding.EncodeToString(p) == value {
				r.WebsocketKeyValidURLSafe16B = true
			}
			p, err = base64.StdEncoding.Strict().DecodeString(value)
			r.WebsocketKeyValidStandardBase64 = err == nil && len(p) == 16 && base64.StdEncoding.EncodeToString(p) == value
			r.WebsocketKeyHasURLOnlyChars = strings.ContainsAny(value, "-_")
		}
	}
	return r, end + 4
}

// AES-128 uses the first EVP_BytesToKey MD5 block. HKDF-SHA1 needs only one
// expansion block for its 16-byte subkey. No Shadowsocks client decoder is used.
func wireAEAD(password string, salt []byte) (cipher.AEAD, error) {
	master := md5.Sum([]byte(password))
	extract := hmac.New(sha1.New, salt)
	extract.Write(master[:])
	expand := hmac.New(sha1.New, extract.Sum(nil))
	expand.Write([]byte("ss-subkey"))
	expand.Write([]byte{1})
	block, err := aes.NewCipher(expand.Sum(nil)[:16])
	if err != nil {
		return nil, err
	}
	return cipher.NewGCM(block)
}
func wireNextNonce(nonce []byte) {
	for i := range nonce {
		nonce[i]++
		if nonce[i] != 0 {
			return
		}
	}
}
func wireDecrypt(r *wireSocketReport, body []byte, password string) []byte {
	r.Authentication = "unverified"
	if len(body) < 16 {
		r.Stage = "salt-truncated"
		return nil
	}
	r.SaltSHA256 = wireHash(body[:16])
	aead, err := wireAEAD(password, body[:16])
	if err != nil {
		r.Stage = "cipher-init-failed"
		return nil
	}
	nonce := make([]byte, 12)
	body = body[16:]
	var plain []byte
	for len(body) > 0 {
		i := len(r.RecordPlaintextLengths)
		fail := func(stage string, mac bool) {
			r.Stage = stage
			r.FailureRecord = &i
			if mac {
				r.Authentication = "failed"
			} else if i > 0 {
				r.Authentication = "partial"
			}
		}
		if len(body) < 18 {
			fail("length-truncated", false)
			return plain
		}
		length, err := aead.Open(nil, nonce, body[:18], nil)
		if err != nil {
			fail("length-mac-failed", true)
			return plain
		}
		wireNextNonce(nonce)
		size := int(binary.BigEndian.Uint16(length))
		body = body[18:]
		if size > 0x3fff {
			fail("invalid-record-length", false)
			return plain
		}
		if len(body) < size+16 {
			fail("payload-truncated", false)
			return plain
		}
		payload, err := aead.Open(nil, nonce, body[:size+16], nil)
		if err != nil {
			fail("payload-mac-failed", true)
			return plain
		}
		wireNextNonce(nonce)
		r.RecordPlaintextLengths = append(r.RecordPlaintextLengths, len(payload))
		plain = append(plain, payload...)
		body = body[size+16:]
	}
	if len(r.RecordPlaintextLengths) == 0 {
		r.Stage = "no-records"
		return nil
	}
	r.Authentication = "passed"
	r.Stage = "verified-captured-records"
	return plain
}

func wireDestination(r *wireSocketReport, plain []byte) []byte {
	if len(plain) < 1 {
		return nil
	}
	end := 0
	expected := false
	switch plain[0] {
	case 1:
		end = 7
	case 4:
		end = 19
	case 3:
		if len(plain) < 2 {
			return nil
		}
		end = 2 + int(plain[1]) + 2
		if len(plain) >= end {
			expected = strings.EqualFold(string(plain[2:end-2]), "cp.cloudflare.com")
		}
	default:
		return nil
	}
	if len(plain) < end {
		return nil
	}
	r.DestinationSHA256 = wireHash(plain[:end])
	r.DestinationIsExpected = expected && binary.BigEndian.Uint16(plain[end-2:end]) == 443
	return plain[end:]
}

func wireTLS(data []byte) wireTLSReport {
	r := wireTLSReport{Stage: "no-tls-data"}
	if len(data) == 0 {
		return r
	}
	r.Stage = "record-truncated"
	if len(data) < 5 {
		return r
	}
	r.RecordType = int(data[0])
	r.RecordLength = int(binary.BigEndian.Uint16(data[3:5]))
	if len(data)-5 < r.RecordLength {
		return r
	}
	r.RecordComplete = true
	if r.RecordType != 22 || data[1] != 3 {
		r.Stage = "unexpected-record"
		return r
	}
	p := data[5 : 5+r.RecordLength]
	if len(p) < 4 {
		r.Stage = "handshake-truncated"
		return r
	}
	r.HandshakeType = int(p[0])
	r.HandshakeLength = int(p[1])<<16 | int(p[2])<<8 | int(p[3])
	if r.HandshakeType != 1 {
		r.Stage = "not-client-hello"
		return r
	}
	if len(p)-4 < r.HandshakeLength {
		r.Stage = "handshake-truncated"
		return r
	}
	p = p[4 : 4+r.HandshakeLength]
	r.Stage = "malformed-client-hello"
	// ClientHello: version/random, session ID, cipher suites, compression,
	// then a bounded extension vector. Every length is checked before slicing.
	if len(p) < 35 {
		return r
	}
	p = p[34:]
	n := int(p[0])
	p = p[1:]
	if n > len(p) {
		return r
	}
	p = p[n:]
	if len(p) < 2 {
		return r
	}
	n = int(binary.BigEndian.Uint16(p))
	p = p[2:]
	if n < 2 || n%2 != 0 || n > len(p) {
		return r
	}
	p = p[n:]
	if len(p) < 1 {
		return r
	}
	n = int(p[0])
	p = p[1:]
	if n < 1 || n > len(p) {
		return r
	}
	p = p[n:]
	if len(p) == 0 {
		r.Stage = "client-hello-without-sni"
		return r
	}
	if len(p) < 2 {
		return r
	}
	n = int(binary.BigEndian.Uint16(p))
	p = p[2:]
	if n != len(p) {
		return r
	}
	for len(p) > 0 {
		if len(p) < 4 {
			return r
		}
		kind := binary.BigEndian.Uint16(p)
		n = int(binary.BigEndian.Uint16(p[2:4]))
		p = p[4:]
		if n > len(p) {
			return r
		}
		ext := p[:n]
		p = p[n:]
		if kind != 0 {
			continue
		}
		if r.SNIPresent || len(ext) < 2 || int(binary.BigEndian.Uint16(ext)) != len(ext)-2 {
			return r
		}
		ext = ext[2:]
		names := 0
		for len(ext) > 0 {
			if len(ext) < 3 {
				return r
			}
			nameType := ext[0]
			n = int(binary.BigEndian.Uint16(ext[1:3]))
			ext = ext[3:]
			if n == 0 || n > len(ext) {
				return r
			}
			if nameType == 0 {
				names++
				r.SNIPresent = true
				r.SNIMatchesExpected = strings.EqualFold(string(ext[:n]), "cp.cloudflare.com")
			}
			ext = ext[n:]
		}
		if names != 1 {
			r.SNIMatchesExpected = false
			return r
		}
	}
	if r.SNIPresent {
		r.Stage = "client-hello-parsed"
	} else {
		r.Stage = "client-hello-without-sni"
	}
	return r
}

func wireAnalyze(captures map[wireSocketID]*wireCapture, summary wireSummary, password string) wireReport {
	report := wireReport{Format: "TinyPaws-private-wire-forensics-v1", CaptureLimitPerSocket: wireSocketLimit, Summary: summary, Sockets: []wireSocketReport{}, DuplicateSalt: [][]wireSocketID{}}
	keys := make([]wireSocketID, 0, len(captures))
	for key := range captures {
		keys = append(keys, key)
	}
	sort.Slice(keys, func(i, j int) bool {
		if keys[i].PID != keys[j].PID {
			return keys[i].PID < keys[j].PID
		}
		return keys[i].ID < keys[j].ID
	})
	salts := map[string][]wireSocketID{}
	for _, key := range keys {
		c := captures[key]
		r := c.wireSocketReport
		limit := wireSocketLimit
		if c.ObservedEnd < int64(limit) {
			limit = int(c.ObservedEnd)
		}
		for i := 0; i < limit; {
			if c.present[i] {
				i++
				continue
			}
			start := i
			for i < limit && !c.present[i] {
				i++
			}
			r.MissingRanges = append(r.MissingRanges, wireRange{start, i})
		}
		for r.ContiguousBytes < limit && c.present[r.ContiguousBytes] {
			r.ContiguousBytes++
		}
		wire := c.data[:r.ContiguousBytes]
		r.WireSHA256 = wireHash(wire)
		r.WireHashScope = "reconstructed-contiguous-prefix"
		r.HTTP, limit = wireHTTP(wire)
		r.Authentication = "unverified"
		r.Stage = "http-header-incomplete"
		r.TLS = wireTLS(nil)
		if c.ConflictingBytes > 0 {
			r.Stage = "capture-conflict"
		} else if limit >= 0 && len(r.HTTP.Issues) == 0 {
			plain := wireDecrypt(&r, wire[limit:], password)
			r.TLS = wireTLS(wireDestination(&r, plain))
			if r.Authentication == "passed" && (r.CaptureAtLimit || r.CaptureTruncated || len(r.MissingRanges) > 0) {
				r.Authentication = "partial"
				r.Stage = "verified-prefix-capture-incomplete"
			}
		} else if limit >= 0 {
			r.Stage = "http-header-invalid"
		}
		if r.SaltSHA256 != "" {
			salts[r.SaltSHA256] = append(salts[r.SaltSHA256], key)
		}
		s := &report.Summary
		s.Sockets++
		switch r.Authentication {
		case "passed":
			s.AuthenticatedSockets++
		case "partial":
			s.PartialSockets++
		case "failed":
			s.MACFailureSockets++
		}
		if len(r.MissingRanges) > 0 {
			s.GapSockets++
		}
		if r.OverlapBytes > 0 {
			s.OverlapSockets++
		}
		if r.ConflictingBytes > 0 {
			s.ConflictingSockets++
		}
		if r.CaptureTruncated {
			s.TruncatedSockets++
		}
		if r.CaptureAtLimit {
			s.AtLimitSockets++
		}
		if r.DestinationIsExpected {
			s.ExpectedDestinationSockets++
		}
		if r.TLS.Stage == "client-hello-parsed" && r.TLS.SNIMatchesExpected {
			s.ExpectedSNISockets++
		}
		report.Sockets = append(report.Sockets, r)
	}
	// Only socket ID groups leave this function; raw salts never do.
	for _, group := range salts {
		if len(group) > 1 {
			report.DuplicateSalt = append(report.DuplicateSalt, group)
		}
	}
	sort.Slice(report.DuplicateSalt, func(i, j int) bool {
		a, b := report.DuplicateSalt[i][0], report.DuplicateSalt[j][0]
		if a.PID != b.PID {
			return a.PID < b.PID
		}
		return a.ID < b.ID
	})
	report.Summary.DuplicateSaltGroups = len(report.DuplicateSalt)
	return report
}

func wireReadPrivate(path string, limit int64) ([]byte, bool) {
	f, err := os.Open(path)
	if err != nil {
		return nil, false
	}
	defer f.Close()
	info, err := f.Stat()
	if err != nil || !info.Mode().IsRegular() || info.Size() > limit {
		return nil, false
	}
	p, err := io.ReadAll(io.LimitReader(f, limit+1))
	return p, err == nil && int64(len(p)) <= limit
}
func wireWritePrivate(path string, report wireReport) bool {
	f, err := os.CreateTemp(filepath.Dir(path), ".tinypaws-wire-*.json")
	if err != nil {
		return false
	}
	temp := f.Name()
	defer os.Remove(temp)
	if f.Chmod(0600) != nil {
		f.Close()
		return false
	}
	enc := json.NewEncoder(f)
	enc.SetIndent("", "  ")
	if enc.Encode(report) != nil {
		f.Close()
		return false
	}
	if f.Close() != nil {
		return false
	}
	return os.Rename(temp, path) == nil
}

func TestDecodeCapturedSocketWireFromPrivateFiles(t *testing.T) {
	logPath, nodePath, resultPath := os.Getenv("TINYPAWS_WIRE_LOG_PATH"), os.Getenv("TINYPAWS_WIRE_NODE_PATH"), os.Getenv("TINYPAWS_WIRE_RESULT_PATH")
	if logPath == "" || nodePath == "" || resultPath == "" {
		t.Skip("private wire input/output environment is not fully configured")
	}
	logAbs, e1 := filepath.Abs(logPath)
	nodeAbs, e2 := filepath.Abs(nodePath)
	resultAbs, e3 := filepath.Abs(resultPath)
	if e1 != nil || e2 != nil || e3 != nil || resultAbs == logAbs || resultAbs == nodeAbs {
		t.Fatal("invalid private output destination")
	}
	nodeData, ok := wireReadPrivate(nodePath, 64<<10)
	if !ok {
		t.Fatal("private node file unreadable or exceeds 64 KiB")
	}
	var node wirePrivateNode
	if json.Unmarshal(nodeData, &node) != nil {
		t.Fatal("invalid private node JSON")
	}
	var port int
	if json.Unmarshal(node.Port, &port) != nil {
		var p string
		if json.Unmarshal(node.Port, &p) != nil {
			t.Fatal("invalid private node port")
		}
		port, _ = strconv.Atoi(p)
	}
	if node.Cipher != "aes-128-gcm" || node.Password == "" || node.Server == "" || port < 1 || port > 65535 {
		t.Fatal("private node must supply aes-128-gcm, password, server and valid port")
	}
	logData, ok := wireReadPrivate(logPath, wireLogLimit)
	if !ok {
		t.Fatal("private wire log unreadable or exceeds 16 MiB")
	}
	captures, summary := wireReadCaptures(logData)
	report := wireAnalyze(captures, summary, node.Password)
	if !wireWritePrivate(resultPath, report) {
		t.Fatal("cannot write private wire report")
	}
	// The log contains counts only; details remain in the mode-0600 report.
	safe, _ := json.Marshal(report.Summary)
	t.Log(string(safe))
	if report.Summary.Samples == 0 {
		t.Error("no valid socket wire samples found")
	}
}

func wireFixtureHello() []byte {
	host := []byte("cp.cloudflare.com")
	names := append([]byte{0, byte(len(host) >> 8), byte(len(host))}, host...)
	sni := append([]byte{byte(len(names) >> 8), byte(len(names))}, names...)
	ext := append([]byte{0, 0, byte(len(sni) >> 8), byte(len(sni))}, sni...)
	body := append([]byte{3, 3}, make([]byte, 32)...)
	body = append(body, 0, 0, 2, 0x13, 1, 1, 0, byte(len(ext)>>8), byte(len(ext)))
	body = append(body, ext...)
	handshake := append([]byte{1, byte(len(body) >> 16), byte(len(body) >> 8), byte(len(body))}, body...)
	return append([]byte{22, 3, 1, byte(len(handshake) >> 8), byte(len(handshake))}, handshake...)
}
func wireFixtureCapture(t *testing.T, id int64, salt []byte, damage bool) *wireCapture {
	t.Helper()
	// Reuse only the existing independent synthetic *encoder* as a cross-check;
	// captured traffic above always uses wireAEAD and wireDecrypt, not a library.
	encoder, err := newIndependentSS(salt)
	if err != nil {
		t.Fatal("synthetic encoder failed")
	}
	dest := append([]byte{3, 17}, []byte("cp.cloudflare.com")...)
	dest = append(dest, 1, 187)
	body := append(bytes.Clone(salt), encoder.seal(dest)...)
	body = append(body, encoder.seal(wireFixtureHello())...)
	if damage {
		body[len(body)-1] ^= 1
	}
	header := "GET / HTTP/1.1\r\nHost: synthetic.invalid:443\r\nUser-Agent: curl/7.1.0\r\nContent-Length: " + strconv.Itoa(len(body)) + "\r\nConnection: Upgrade\r\nSec-WebSocket-Key: _____________________w==\r\nUpgrade: websocket\r\n\r\n"
	data := append([]byte(header), body...)
	if len(data) > wireSocketLimit {
		t.Fatal("synthetic capture exceeds cap")
	}
	c := &wireCapture{wireSocketReport: wireSocketReport{wireSocketID: wireSocketID{PID: 1, ID: id}, Samples: 1, SampledBytes: len(data), ObservedEnd: int64(len(data)), UniqueBytes: len(data), RecordPlaintextLengths: []int{}}}
	copy(c.data[:], data)
	for i := range data {
		c.present[i] = true
	}
	return c
}
func TestCapturedWireForensicsSynthetic(t *testing.T) {
	salt := bytes.Repeat([]byte{0x42}, 16)
	good := wireFixtureCapture(t, 1, salt, false)
	duplicate := wireFixtureCapture(t, 2, salt, false)
	bad := wireFixtureCapture(t, 3, bytes.Repeat([]byte{0x24}, 16), true)
	report := wireAnalyze(map[wireSocketID]*wireCapture{good.wireSocketID: good, duplicate.wireSocketID: duplicate, bad.wireSocketID: bad}, wireSummary{}, "local-test-only")
	if len(report.Sockets) != 3 || report.Summary.AuthenticatedSockets != 2 || report.Summary.MACFailureSockets != 1 || report.Summary.DuplicateSaltGroups != 1 {
		t.Fatal("synthetic authentication or salt grouping failed")
	}
	if len(report.DuplicateSalt[0]) != 2 || report.DuplicateSalt[0][0].ID != 1 || report.DuplicateSalt[0][1].ID != 2 {
		t.Fatal("synthetic duplicate salt IDs incorrect")
	}
	r := report.Sockets[0]
	if !r.DestinationIsExpected || !r.TLS.SNIMatchesExpected || r.TLS.Stage != "client-hello-parsed" || len(r.RecordPlaintextLengths) != 2 {
		t.Fatal("synthetic destination or ClientHello failed")
	}
	if !r.HTTP.WebsocketKeyValidURLSafe16B || r.HTTP.WebsocketKeyValidStandardBase64 || !r.HTTP.WebsocketKeyHasURLOnlyChars {
		t.Fatal("synthetic WebSocket key classification failed")
	}
	if report.Sockets[2].Stage != "payload-mac-failed" {
		t.Fatal("synthetic damaged tag was not rejected at payload")
	}
	wrong := wireAnalyze(map[wireSocketID]*wireCapture{good.wireSocketID: good}, wireSummary{}, "deliberately-wrong-test-password")
	if wrong.Sockets[0].Stage != "length-mac-failed" || wrong.Sockets[0].Authentication != "failed" {
		t.Fatal("synthetic wrong password accepted")
	}
	// Exercise offset sorting, holes, exact duplicates, conflicting overlap and
	// the hard 2-KiB cap using only local public fixture bytes.
	line := func(offset int, data string) string {
		p, _ := json.Marshal(map[string]any{"atMs": 1, "pid": 7, "flow": "synthetic", "event": "socket-trace event=wire-write-sample · id=4, offset=" + strconv.Itoa(offset) + ", n=" + strconv.Itoa(len(data)) + ", hex=" + hex.EncodeToString([]byte(data))})
		return string(p) + "\n"
	}
	captures, stats := wireReadCaptures([]byte(line(2, "cd") + line(0, "ab") + line(0, "ab") + line(1, "X") + line(6, "g") + line(2047, "zz")))
	offsets := wireAnalyze(captures, stats, "local-test-only").Sockets[0]
	if offsets.ContiguousBytes != 4 || offsets.DuplicateSamples != 1 || offsets.ConflictingBytes != 1 || offsets.OverlapBytes != 3 || offsets.OutOfOrderSamples != 1 || !offsets.CaptureTruncated || !offsets.CaptureAtLimit || len(offsets.MissingRanges) != 2 || offsets.Stage != "capture-conflict" {
		t.Fatal("synthetic offset/cap accounting failed")
	}
	invalid, invalidStats := wireReadCaptures([]byte(`{"pid":7,"event":"socket-trace event=wire-write-sample id=4 hex=00"}` + "\n"))
	if len(invalid) != 0 || invalidStats.MissingOrInvalidOffsetSamples != 1 {
		t.Fatal("synthetic missing offset was not reported")
	}
	encoded, _ := json.Marshal(report)
	for _, secret := range []string{"local-test-only", "synthetic.invalid", "cp.cloudflare.com", "_____________________w=="} {
		if bytes.Contains(encoded, []byte(secret)) {
			t.Fatal("synthetic report leaked a raw private field")
		}
	}
	path := filepath.Join(t.TempDir(), "private.json")
	if !wireWritePrivate(path, report) {
		t.Fatal("synthetic private report write failed")
	}
	info, err := os.Stat(path)
	if err != nil || info.Mode().Perm() != 0600 {
		t.Fatal("synthetic private report permissions incorrect")
	}
	// Run the complete environment/file entry point with generated fixtures;
	// no ambient node or capture file is read by this subtest.
	t.Run("private-files-synthetic", func(t *testing.T) {
		dir := t.TempDir()
		logPath, nodePath, resultPath := filepath.Join(dir, "wire.jsonl"), filepath.Join(dir, "node.json"), filepath.Join(dir, "result.json")
		if os.WriteFile(logPath, []byte(line(0, string(good.data[:good.ObservedEnd]))), 0600) != nil {
			t.Fatal("synthetic capture write failed")
		}
		node := []byte(`{"cipher":"aes-128-gcm","password":"local-test-only","server":"synthetic.invalid","port":443}`)
		if os.WriteFile(nodePath, node, 0600) != nil {
			t.Fatal("synthetic node write failed")
		}
		t.Setenv("TINYPAWS_WIRE_LOG_PATH", logPath)
		t.Setenv("TINYPAWS_WIRE_NODE_PATH", nodePath)
		t.Setenv("TINYPAWS_WIRE_RESULT_PATH", resultPath)
		TestDecodeCapturedSocketWireFromPrivateFiles(t)
		got, ok := wireReadPrivate(resultPath, 64<<10)
		var parsed wireReport
		if !ok || json.Unmarshal(got, &parsed) != nil || parsed.Summary.AuthenticatedSockets != 1 || parsed.Summary.ExpectedSNISockets != 1 {
			t.Fatal("synthetic private entry point failed")
		}
	})
}
