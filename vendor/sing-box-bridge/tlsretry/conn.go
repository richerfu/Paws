// Package tlsretry recovers a fresh ClientHello after EOF/reset, before any decrypted reply.
// It never replays HTTP, TLS application data, or an established TLS connection.
package tlsretry

import (
	"context"
	"errors"
	"io"
	"net"
	"os"
	"sync"
	"syscall"
	"time"
)

const capFlight = 16389

// Unknown suites remain transparent but cannot establish replay eligibility.
// TLS 1.2 PSK suites do not require extension 41.
func freshCipher(suite uint16) bool {
	switch suite {
	case 0x000a, 0x002f, 0x0035, 0x003c, 0x003d, 0x009c, 0x009d,
		0xc009, 0xc00a, 0xc012, 0xc013, 0xc014, 0xc023, 0xc024,
		0xc027, 0xc028, 0xc02b, 0xc02c, 0xc02f, 0xc030, 0xcca8, 0xcca9,
		0x1301, 0x1302, 0x1303, 0x1304, 0x1305:
		return true
	}
	return false
}

func exceedsRecord(prefix, next []byte) bool {
	if len(prefix)+len(next) > capFlight {
		return true
	}
	var header [5]byte
	n := copy(header[:], prefix)
	n += copy(header[n:], next)
	return n == len(header) && len(prefix)+len(next) > 5+(int(header[3])<<8|int(header[4]))
}

func ValidFlight(b []byte) bool { return classify(b) == 1 }

// -1 rejected, 0 incomplete, 1 exactly one fresh ClientHello record.
func classify(b []byte) int {
	if len(b) == 0 {
		return 0
	}
	if b[0] != 22 {
		return -1
	}
	if len(b) < 5 {
		return 0
	}
	n := int(b[3])<<8 | int(b[4])
	if b[1] != 3 || b[2] < 1 || b[2] > 3 || n < 45 || n > 16384 || len(b) > n+5 {
		return -1
	}
	if len(b) < n+5 {
		return 0
	}
	if b[5] != 1 || int(b[6])<<16|int(b[7])<<8|int(b[8]) != n-4 || b[9] != 3 || b[10] != 3 {
		return -1
	}
	p := 43
	sid := int(b[p])
	p++
	// A nonempty legacy ID is ambiguous with TLS 1.2 resumption, even when
	// used only for TLS 1.3 compatibility. Do not guess whether it is fresh.
	if sid != 0 || p+sid+2 > len(b) {
		return -1
	}
	p += sid
	c := int(b[p])<<8 | int(b[p+1])
	p += 2
	if c < 2 || c%2 != 0 || p+c+1 > len(b) {
		return -1
	}
	knownCipher := false
	for end := p + c; p < end; p += 2 {
		suite := uint16(b[p])<<8 | uint16(b[p+1])
		if freshCipher(suite) {
			knownCipher = true
		} else if suite != 0x00ff && suite != 0x5600 && !(b[p] == b[p+1] && b[p]&0x0f == 0x0a) {
			return -1
		}
	}
	if !knownCipher {
		return -1
	}
	comp := int(b[p])
	p++
	if comp == 0 || p+comp > len(b) {
		return -1
	}
	p += comp
	if p == len(b) {
		return 1
	}
	if p+2 > len(b) {
		return -1
	}
	ext := int(b[p])<<8 | int(b[p+1])
	p += 2
	if p+ext != len(b) {
		return -1
	}
	for p < len(b) {
		if p+4 > len(b) {
			return -1
		}
		kind := int(b[p])<<8 | int(b[p+1])
		size := int(b[p+2])<<8 | int(b[p+3])
		p += 4
		// ECH conceals inner PSK/early_data, so it cannot satisfy this guard.
		if kind == 35 || kind == 41 || kind == 42 || kind == 0xfe0d || p+size > len(b) {
			return -1
		}
		p += size
	}
	return 1
}

// HandshakeContext binds cancellation before the eager SS destination write.
// The raw socket is private to this handshake; no caller deadline is discarded.
func HandshakeContext(ctx context.Context, raw net.Conn, handshake func() (net.Conn, error)) (net.Conn, error) {
	stop := context.AfterFunc(ctx, func() { _ = raw.Close() })
	defer stop()
	if err := ctx.Err(); err != nil {
		_ = raw.Close()
		return nil, err
	}
	if deadline, ok := ctx.Deadline(); ok {
		_ = raw.SetWriteDeadline(deadline)
	}
	conn, err := handshake()
	if err != nil {
		_ = raw.Close()
		return nil, err
	}
	if !stop() || ctx.Err() != nil {
		_ = raw.Close()
		return nil, ctx.Err()
	}
	if err = conn.SetWriteDeadline(time.Time{}); err != nil {
		_ = conn.Close()
		return nil, err
	}
	return conn, nil
}

type Redial func(context.Context) (net.Conn, error)
type Conn struct {
	mu, readMu, writeMu                  sync.Mutex
	conn                                 net.Conn
	ctx                                  context.Context
	cancel                               context.CancelFunc
	dial                                 Redial
	flight                               []byte
	disabled, committed, retried, closed bool
	recoveryTimeout                      time.Duration
	readDeadline, writeDeadline          time.Time
	local, remote                        net.Addr
	notify                               func(string)
	attemptCancel                        context.CancelFunc
	attemptTimer                         *time.Timer
	attemptDeadline                      time.Time
	attemptGeneration                    uint64
}

func New(ctx context.Context, conn net.Conn, dial Redial, notify func(string)) *Conn {
	ctx, cancel := context.WithCancel(ctx)
	return &Conn{conn: conn, ctx: ctx, cancel: cancel, dial: dial, recoveryTimeout: 4 * time.Second, local: conn.LocalAddr(), remote: conn.RemoteAddr(), notify: notify}
}
func earlier(a, b time.Time) time.Time {
	if a.IsZero() || !b.IsZero() && b.Before(a) {
		return b
	}
	return a
}

// All state transitions AND underlying deadline updates share mu. Computing a
// deadline under mu and applying it later can re-arm a stale caller deadline.
func (c *Conn) effectiveReadDeadlineLocked() time.Time {
	deadline := c.readDeadline
	// There is no internal deadline on the original connection. The bounded
	// recovery starts only after a terminal EOF/reset and includes its reply.
	if c.retried && !c.committed && !c.disabled {
		deadline = earlier(deadline, c.attemptDeadline)
	}
	return deadline
}
func (c *Conn) applyReadDeadlineLocked() error {
	if c.conn != nil {
		return c.conn.SetReadDeadline(c.effectiveReadDeadlineLocked())
	}
	return nil
}

// A caller may change a deadline while dial/eager-write is in progress, when
// conn is nil. A separately resettable timer cancels that whole attempt too.
func (c *Conn) refreshAttemptTimerLocked() {
	c.attemptGeneration++
	if c.attemptTimer != nil {
		c.attemptTimer.Stop()
		c.attemptTimer = nil
	}
	if c.attemptCancel == nil {
		return
	}
	deadline := earlier(earlier(c.attemptDeadline, c.readDeadline), c.writeDeadline)
	generation := c.attemptGeneration
	c.attemptTimer = time.AfterFunc(time.Until(deadline), func() {
		c.mu.Lock()
		defer c.mu.Unlock()
		if c.attemptCancel != nil && c.attemptGeneration == generation {
			c.attemptCancel()
		}
	})
}
func (c *Conn) clearAttemptLocked() {
	c.attemptGeneration++
	if c.attemptTimer != nil {
		c.attemptTimer.Stop()
		c.attemptTimer = nil
	}
	c.attemptCancel = nil
}
func (c *Conn) recoveryError(err error) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.closed {
		return net.ErrClosed
	}
	now := time.Now()
	if (!c.readDeadline.IsZero() && !now.Before(c.readDeadline)) || (!c.writeDeadline.IsZero() && !now.Before(c.writeDeadline)) || (!c.attemptDeadline.IsZero() && !now.Before(c.attemptDeadline)) {
		return os.ErrDeadlineExceeded
	}
	return err
}
func (c *Conn) Read(p []byte) (int, error) {
	if len(p) == 0 {
		return 0, nil
	}
	c.readMu.Lock()
	defer c.readMu.Unlock()
	for {
		c.mu.Lock()
		conn := c.conn
		if c.closed || conn == nil {
			c.mu.Unlock()
			return 0, net.ErrClosed
		}
		_ = c.applyReadDeadlineLocked()
		c.mu.Unlock()
		n, err := conn.Read(p)
		observedAt := time.Now()
		if n > 0 {
			c.mu.Lock()
			c.committed = true
			c.disabled = true
			c.flight = nil
			_ = c.applyReadDeadlineLocked()
			c.mu.Unlock()
			return n, err
		}
		if err == nil {
			return n, err
		}
		recoverable := errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) || errors.Is(err, syscall.ECONNRESET)
		var ne net.Error
		if errors.As(err, &ne) && (ne.Timeout() || ne.Temporary()) {
			recoverable = false
		}
		if !recoverable {
			c.mu.Lock()
			expiredRecovery := c.retried && !c.committed && !c.disabled && !time.Now().Before(c.attemptDeadline)
			c.mu.Unlock()
			if expiredRecovery {
				_ = c.Close()
			}
			return n, err
		}
		retried, retryErr := c.retry(observedAt)
		if retryErr != nil {
			return 0, retryErr
		}
		if !retried {
			return n, err
		}
	}
}
func (c *Conn) retry(observedAt time.Time) (bool, error) {
	// Never hold a reader behind a blocked codec write after peer EOF. A writer
	// in progress makes a fresh-session replay ineligible anyway.
	if !c.writeMu.TryLock() {
		return false, nil
	}
	defer c.writeMu.Unlock()
	c.mu.Lock()
	if c.closed || c.disabled || c.committed || c.retried || !ValidFlight(c.flight) {
		c.mu.Unlock()
		return false, nil
	}
	if err := c.ctx.Err(); err != nil {
		c.mu.Unlock()
		return false, err
	}
	now := time.Now()
	if (!c.readDeadline.IsZero() && !now.Before(c.readDeadline)) || (!c.writeDeadline.IsZero() && !now.Before(c.writeDeadline)) {
		c.mu.Unlock()
		return false, os.ErrDeadlineExceeded
	}
	c.retried = true
	flight := append([]byte(nil), c.flight...)
	c.flight = nil
	old := c.conn
	c.conn = nil
	deadline := observedAt.Add(c.recoveryTimeout)
	attempt, cancel := context.WithDeadline(c.ctx, deadline)
	c.attemptCancel = cancel
	c.attemptDeadline = deadline
	c.refreshAttemptTimerLocked()
	c.mu.Unlock()
	defer func() { c.mu.Lock(); c.clearAttemptLocked(); c.mu.Unlock(); cancel() }()
	_ = old.Close()
	if c.notify != nil {
		c.notify("zero-return-client-hello; fresh-cipher; maximum-retries=1")
	}
	if err := attempt.Err(); err != nil {
		return true, c.recoveryError(err)
	}
	fresh, err := c.dial(attempt)
	if err != nil {
		return true, c.recoveryError(err)
	}
	stop := context.AfterFunc(attempt, func() { _ = fresh.Close() })
	defer stop()
	if err = fresh.SetWriteDeadline(deadline); err != nil {
		_ = fresh.Close()
		return true, c.recoveryError(err)
	}
	for len(flight) > 0 {
		n, e := fresh.Write(flight)
		if n < 0 || n > len(flight) {
			n = 0
			e = io.ErrShortWrite
		}
		flight = flight[n:]
		if e != nil {
			_ = fresh.Close()
			return true, c.recoveryError(e)
		}
		if n == 0 {
			_ = fresh.Close()
			return true, io.ErrShortWrite
		}
	}
	c.mu.Lock()
	stopped := stop()
	if c.closed || !stopped || attempt.Err() != nil {
		c.mu.Unlock()
		_ = fresh.Close()
		if attempt.Err() != nil {
			return true, c.recoveryError(attempt.Err())
		}
		return true, net.ErrClosed
	}
	c.clearAttemptLocked()
	c.conn = fresh
	c.local = fresh.LocalAddr()
	c.remote = fresh.RemoteAddr()
	err = c.applyReadDeadlineLocked()
	if err == nil {
		err = fresh.SetWriteDeadline(c.writeDeadline)
	}
	if err != nil {
		c.conn = nil
	}
	c.mu.Unlock()
	if err != nil {
		_ = fresh.Close()
		return true, c.recoveryError(err)
	}
	return true, nil
}
func (c *Conn) Write(p []byte) (int, error) {
	c.writeMu.Lock()
	defer c.writeMu.Unlock()
	c.mu.Lock()
	conn := c.conn
	if c.closed || conn == nil {
		c.mu.Unlock()
		return 0, net.ErrClosed
	}
	// Disable before an extra write: codecs may emit ciphertext then return an
	// error/partial result. Such data must never migrate to a fresh TLS session.
	if len(p) > 0 && !c.disabled && !c.retried && (ValidFlight(c.flight) || exceedsRecord(c.flight, p)) {
		c.disabled = true
		c.flight = nil
		_ = c.applyReadDeadlineLocked()
	}
	c.mu.Unlock()
	n, err := conn.Write(p)
	if n > 0 {
		c.mu.Lock()
		if !c.disabled && !c.committed && !c.retried {
			if len(c.flight) == 0 && p[0] != 22 || len(c.flight)+n > capFlight {
				c.disabled = true
				c.flight = nil
			} else {
				c.flight = append(c.flight, p[:n]...)
				switch classify(c.flight) {
				case -1:
					c.disabled = true
					c.flight = nil
				}
			}
		}
		_ = c.applyReadDeadlineLocked()
		c.mu.Unlock()
	}
	return n, err
}
func (c *Conn) Close() error {
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return nil
	}
	c.closed = true
	c.flight = nil
	conn := c.conn
	c.clearAttemptLocked()
	c.mu.Unlock()
	c.cancel()
	if conn != nil {
		return conn.Close()
	}
	return nil
}
func (c *Conn) LocalAddr() net.Addr  { c.mu.Lock(); defer c.mu.Unlock(); return c.local }
func (c *Conn) RemoteAddr() net.Addr { c.mu.Lock(); defer c.mu.Unlock(); return c.remote }
func (c *Conn) SetDeadline(t time.Time) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.readDeadline = t
	c.writeDeadline = t
	c.refreshAttemptTimerLocked()
	err := c.applyReadDeadlineLocked()
	if c.conn != nil {
		if e := c.conn.SetWriteDeadline(t); err == nil {
			err = e
		}
	}
	return err
}
func (c *Conn) SetReadDeadline(t time.Time) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.readDeadline = t
	c.refreshAttemptTimerLocked()
	return c.applyReadDeadlineLocked()
}
func (c *Conn) SetWriteDeadline(t time.Time) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.writeDeadline = t
	c.refreshAttemptTimerLocked()
	if c.conn != nil {
		return c.conn.SetWriteDeadline(t)
	}
	return nil
}
