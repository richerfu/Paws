package tlsretry

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"os"
	"sync"
	"sync/atomic"
	"syscall"
	"testing"
	"time"
)

func hello() []byte {
	body := append([]byte{3, 3}, make([]byte, 32)...)
	body = append(body, 0, 0, 2, 0x13, 1, 1, 0)
	n := len(body) + 4
	return append([]byte{22, 3, 1, byte(n >> 8), byte(n), 1, 0, 0, byte(len(body))}, body...)
}
func setup(t *testing.T) (*Conn, net.Conn, *atomic.Int32) {
	t.Helper()
	client, server := net.Pipe()
	calls := &atomic.Int32{}
	redial := func(context.Context) (net.Conn, error) {
		calls.Add(1)
		a, b := net.Pipe()
		go func() {
			defer b.Close()
			got := make([]byte, len(hello()))
			if _, e := io.ReadFull(b, got); e == nil && bytes.Equal(got, hello()) {
				_, _ = b.Write([]byte("TLS"))
			}
		}()
		return a, nil
	}
	c := New(context.Background(), client, redial, nil)
	t.Cleanup(func() { c.Close(); server.Close() })
	return c, server, calls
}
func TestStrictFlight(t *testing.T) {
	b := hello()
	if !ValidFlight(b) {
		t.Fatal("valid hello rejected")
	}
	for n := 0; n < len(b); n++ {
		if classify(b[:n]) != 0 {
			t.Fatal("partial hello rejected", n)
		}
	}
	if ValidFlight([]byte("POST / HTTP/1.1\r\n")) {
		t.Fatal("plaintext admitted")
	}
	if ValidFlight(append(b, 23, 3, 3, 0, 1, 0)) {
		t.Fatal("application data admitted")
	}
	b = append(hello(), 0, 4, 0, 42, 0, 0)
	n := len(b) - 5
	b[3] = byte(n >> 8)
	b[4] = byte(n)
	b[8] = byte(n - 4)
	if ValidFlight(b) {
		t.Fatal("early-data admitted")
	}
}
func TestZeroReturnEOFRedialsOnce(t *testing.T) {
	c, s, calls := setup(t)
	go func() { defer s.Close(); _, _ = io.CopyN(io.Discard, s, int64(len(hello()))) }()
	if _, e := c.Write(hello()); e != nil {
		t.Fatal(e)
	}
	b := make([]byte, 3)
	if _, e := io.ReadFull(c, b); e != nil || string(b) != "TLS" {
		t.Fatal(string(b), e)
	}
	if calls.Load() != 1 {
		t.Fatal("retry count", calls.Load())
	}
}
func TestPlaintextPostIsNeverCachedOrReplayed(t *testing.T) {
	c, s, calls := setup(t)
	go func() { defer s.Close(); _, _ = io.CopyN(io.Discard, s, 4) }()
	_, _ = c.Write([]byte("POST"))
	if len(c.flight) != 0 {
		t.Fatal("plaintext was cached")
	}
	b := make([]byte, 1)
	_, _ = c.Read(b)
	if calls.Load() != 0 {
		t.Fatal("POST replayed")
	}
}
func TestAnyResponsePreventsReplay(t *testing.T) {
	c, s, calls := setup(t)
	go func() {
		defer s.Close()
		_, _ = io.CopyN(io.Discard, s, int64(len(hello())))
		_, _ = s.Write([]byte("S"))
	}()
	_, _ = c.Write(hello())
	b := make([]byte, 1)
	if _, e := c.Read(b); e != nil {
		t.Fatal(e)
	}
	_, _ = c.Read(b)
	if calls.Load() != 0 {
		t.Fatal("established TLS replayed")
	}
}
func TestExtraApplicationRecordPreventsReplay(t *testing.T) {
	c, s, calls := setup(t)
	wire := append(hello(), 23, 3, 3, 0, 1, 0)
	go func() { defer s.Close(); _, _ = io.CopyN(io.Discard, s, int64(len(wire))) }()
	_, _ = c.Write(wire)
	_, _ = c.Read(make([]byte, 1))
	if calls.Load() != 0 {
		t.Fatal("application data replayed")
	}
}
func TestDelayedOriginalReplyDoesNotStartRecoveryBudget(t *testing.T) {
	c, s, calls := setup(t)
	c.recoveryTimeout = 10 * time.Millisecond
	go func() {
		_, _ = io.CopyN(io.Discard, s, int64(len(hello())))
		time.Sleep(50 * time.Millisecond)
		_, _ = s.Write([]byte("TLS"))
	}()
	_, _ = c.Write(hello())
	b := make([]byte, 3)
	if _, e := io.ReadFull(c, b); e != nil || string(b) != "TLS" {
		t.Fatal(e)
	}
	if calls.Load() != 0 {
		t.Fatal("silent original was replaced")
	}
}
func TestReadBeforeWriteRespectsOnlyCallerDeadline(t *testing.T) {
	c, s, calls := setup(t)
	c.recoveryTimeout = 5 * time.Millisecond
	result := make(chan error, 1)
	go func() { b := make([]byte, 3); _, e := io.ReadFull(c, b); result <- e }()
	go func() { _, _ = io.CopyN(io.Discard, s, int64(len(hello()))) }()
	_, _ = c.Write(hello())
	_ = c.SetReadDeadline(time.Now().Add(25 * time.Millisecond))
	select {
	case e := <-result:
		if !errors.Is(e, os.ErrDeadlineExceeded) {
			t.Fatal(e)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("early read ignored caller deadline")
	}
	if calls.Load() != 0 {
		t.Fatal("caller timeout retried")
	}
}
func TestCallerDeadlineTakesPrecedence(t *testing.T) {
	c, s, calls := setup(t)
	go func() { _, _ = io.CopyN(io.Discard, s, int64(len(hello()))) }()
	_, _ = c.Write(hello())
	_ = c.SetReadDeadline(time.Now().Add(10 * time.Millisecond))
	_, err := c.Read(make([]byte, 1))
	var timeout net.Error
	if !errors.As(err, &timeout) || !timeout.Timeout() {
		t.Fatal(err)
	}
	if calls.Load() != 0 {
		t.Fatal("caller deadline ignored")
	}
}
func TestRecoveryCloseCancelsRedial(t *testing.T) {
	c, s, _ := setup(t)
	entered := make(chan struct{})
	c.dial = func(ctx context.Context) (net.Conn, error) { close(entered); <-ctx.Done(); return nil, ctx.Err() }
	go func() { defer s.Close(); _, _ = io.CopyN(io.Discard, s, int64(len(hello()))) }()
	_, _ = c.Write(hello())
	done := make(chan error, 1)
	go func() { _, e := c.Read(make([]byte, 1)); done <- e }()
	<-entered
	c.Close()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("close did not cancel redial")
	}
}

func TestClearingCallerDeadlineKeepsSilentOriginalOpen(t *testing.T) {
	for _, all := range []bool{false, true} {
		t.Run(map[bool]string{false: "read", true: "both"}[all], func(t *testing.T) {
			c, s, calls := setup(t)
			c.recoveryTimeout = 5 * time.Millisecond
			go func() { _, _ = io.CopyN(io.Discard, s, int64(len(hello()))) }()
			_, _ = c.Write(hello())
			_ = c.SetReadDeadline(time.Now().Add(time.Second))
			done := make(chan error, 1)
			go func() { _, e := io.ReadFull(c, make([]byte, 3)); done <- e }()
			time.Sleep(5 * time.Millisecond)
			if all {
				_ = c.SetDeadline(time.Time{})
			} else {
				_ = c.SetReadDeadline(time.Time{})
			}
			select {
			case e := <-done:
				t.Fatal("silent connection ended without a caller deadline", e)
			case <-time.After(35 * time.Millisecond):
			}
			if calls.Load() != 0 {
				t.Fatal("recovery count", calls.Load())
			}
			if _, e := s.Write([]byte("TLS")); e != nil {
				t.Fatal("original was closed", e)
			}
			select {
			case e := <-done:
				if e != nil {
					t.Fatal(e)
				}
			case <-time.After(time.Second):
				t.Fatal("original reply was lost")
			}
		})
	}
}
func TestCallerDeadlineInterruptsRecovery(t *testing.T) {
	for _, all := range []bool{false, true} {
		t.Run(map[bool]string{false: "read", true: "both"}[all], func(t *testing.T) {
			c, s, _ := setup(t)
			entered := make(chan struct{})
			c.dial = func(ctx context.Context) (net.Conn, error) { close(entered); <-ctx.Done(); return nil, ctx.Err() }
			go func() { defer s.Close(); _, _ = io.CopyN(io.Discard, s, int64(len(hello()))) }()
			_, _ = c.Write(hello())
			done := make(chan error, 1)
			go func() { _, e := c.Read(make([]byte, 1)); done <- e }()
			<-entered
			deadline := time.Now().Add(20 * time.Millisecond)
			if all {
				_ = c.SetDeadline(deadline)
			} else {
				_ = c.SetReadDeadline(deadline)
			}
			select {
			case e := <-done:
				var timeout net.Error
				if !errors.As(e, &timeout) || !timeout.Timeout() {
					t.Fatal(e)
				}
			case <-time.After(time.Second):
				t.Fatal("deadline did not interrupt redial")
			}
		})
	}
}
func TestExtendingDeadlineDuringRecoveryCancelsOldTimer(t *testing.T) {
	c, s, _ := setup(t)
	entered := make(chan struct{})
	finish := make(chan struct{})
	c.dial = func(ctx context.Context) (net.Conn, error) {
		close(entered)
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-finish:
			return nil, io.EOF
		}
	}
	go func() { defer s.Close(); _, _ = io.CopyN(io.Discard, s, int64(len(hello()))) }()
	_, _ = c.Write(hello())
	done := make(chan error, 1)
	go func() { _, e := c.Read(make([]byte, 1)); done <- e }()
	<-entered
	_ = c.SetReadDeadline(time.Now().Add(25 * time.Millisecond))
	_ = c.SetReadDeadline(time.Time{})
	select {
	case e := <-done:
		t.Fatal("old caller timer survived clear", e)
	case <-time.After(60 * time.Millisecond):
	}
	close(finish)
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("read did not finish")
	}
}
func TestEagerHandshakeContextCancelsBlockedWrite(t *testing.T) {
	for _, cancelOnly := range []bool{false, true} {
		t.Run(map[bool]string{false: "deadline", true: "cancel"}[cancelOnly], func(t *testing.T) {
			a, b := net.Pipe()
			defer b.Close()
			ctx, cancel := context.WithTimeout(context.Background(), 30*time.Millisecond)
			defer cancel()
			entered := make(chan struct{})
			done := make(chan error, 1)
			go func() {
				_, e := HandshakeContext(ctx, a, func() (net.Conn, error) {
					close(entered)
					_, e := a.Write([]byte("encrypted destination"))
					return a, e
				})
				done <- e
			}()
			<-entered
			if cancelOnly {
				cancel()
			}
			select {
			case e := <-done:
				if e == nil {
					t.Fatal("blocked handshake succeeded")
				}
			case <-time.After(time.Second):
				t.Fatal("eager write ignored cancellation")
			}
		})
	}
}

// A blocked underlying setter forces caller deadline changes to overlap.
type deadlineOverlapConn struct {
	net.Conn
	first            atomic.Bool
	entered, release chan struct{}
	deadlineMu       sync.Mutex
	deadline         time.Time
}

func (d *deadlineOverlapConn) Write(p []byte) (int, error) { return len(p), nil }
func (d *deadlineOverlapConn) Read(p []byte) (int, error)  { p[0] = 'S'; return 1, nil }
func (d *deadlineOverlapConn) SetReadDeadline(v time.Time) error {
	if !v.IsZero() && d.first.CompareAndSwap(false, true) {
		close(d.entered)
		<-d.release
	}
	d.deadlineMu.Lock()
	d.deadline = v
	d.deadlineMu.Unlock()
	return nil
}
func TestConcurrentDeadlineSetAndClearCannotLeaveStaleDeadline(t *testing.T) {
	a, b := net.Pipe()
	defer b.Close()
	d := &deadlineOverlapConn{Conn: a, entered: make(chan struct{}), release: make(chan struct{})}
	c := New(context.Background(), d, nil, nil)
	defer c.Close()
	set := make(chan struct{})
	go func() { _ = c.SetReadDeadline(time.Now().Add(time.Second)); close(set) }()
	<-d.entered
	cleared := make(chan struct{})
	go func() { _ = c.SetReadDeadline(time.Time{}); close(cleared) }()
	// Clear must serialize after the blocked first setter, not race ahead.
	time.Sleep(15 * time.Millisecond)
	close(d.release)
	select {
	case <-set:
	case <-time.After(time.Second):
		t.Fatal("set deadline stuck")
	}
	select {
	case <-cleared:
	case <-time.After(time.Second):
		t.Fatal("clear deadline stuck")
	}
	d.deadlineMu.Lock()
	defer d.deadlineMu.Unlock()
	if !d.deadline.IsZero() {
		t.Fatal("caller clear lost to a stale deadline update")
	}
}
func TestFailedExtraWritePreventsReplay(t *testing.T) {
	c, s, calls := setup(t)
	go func() { _, _ = io.CopyN(io.Discard, s, int64(len(hello()))); s.Close() }()
	_, _ = c.Write(hello())
	_, _ = c.Write([]byte("extra application bytes"))
	_, _ = c.Read(make([]byte, 1))
	if calls.Load() != 0 {
		t.Fatal("failed extra write allowed replay")
	}
}

func TestEOFDoesNotWaitBehindAnExtraBlockedWrite(t *testing.T) {
	c, s, _ := setup(t)
	go func() { _, _ = io.CopyN(io.Discard, s, int64(len(hello()))) }()
	_, _ = c.Write(hello())
	// An extra writer has already disabled recovery but has not completed.
	c.writeMu.Lock()
	c.mu.Lock()
	c.disabled = true
	c.flight = nil
	c.mu.Unlock()
	done := make(chan error, 1)
	go func() { _, e := c.Read(make([]byte, 1)); done <- e }()
	s.Close()
	select {
	case <-done:
	case <-time.After(200 * time.Millisecond):
		c.writeMu.Unlock()
		t.Fatal("EOF waits behind writer")
	}
	c.writeMu.Unlock()
}

func TestECHAndPSKAreIneligibleForReplay(t *testing.T) {
	for _, ext := range []uint16{35, 41, 42, 0xfe0d} {
		b := append(hello(), 0, 4, byte(ext>>8), byte(ext), 0, 0)
		n := len(b) - 5
		b[3] = byte(n >> 8)
		b[4] = byte(n)
		b[8] = byte(n - 4)
		if ValidFlight(b) {
			t.Fatalf("opaque or resumable hello admitted: %x", ext)
		}
	}
}

func TestInitialConnectionNeverGetsAnInternalDeadline(t *testing.T) {
	a, b := net.Pipe()
	defer b.Close()
	d := &deadlineOverlapConn{Conn: a, entered: make(chan struct{}), release: make(chan struct{})}
	close(d.release)
	c := New(context.Background(), d, nil, nil)
	defer c.Close()
	_, _ = c.Write(hello())
	_, _ = c.Read(make([]byte, 1))
	if d.first.Load() {
		t.Fatal("a nonzero internal deadline was installed on the original")
	}
}

type readErrorConn struct {
	net.Conn
	err error
}

func (c *readErrorConn) Read([]byte) (int, error)    { return 0, c.err }
func (c *readErrorConn) Write(p []byte) (int, error) { return len(p), nil }

type temporaryReset struct{}

func (temporaryReset) Error() string   { return "local timeout wrapping reset" }
func (temporaryReset) Unwrap() error   { return syscall.ECONNRESET }
func (temporaryReset) Timeout() bool   { return true }
func (temporaryReset) Temporary() bool { return true }

func TestOnlyTerminalPeerReadErrorsStartRecovery(t *testing.T) {
	for _, tc := range []struct {
		name    string
		err     error
		retries int32
	}{
		{"EOF", io.EOF, 1}, {"truncated-EOF", io.ErrUnexpectedEOF, 1}, {"RST", syscall.ECONNRESET, 1},
		{"caller-timeout", os.ErrDeadlineExceeded, 0}, {"would-block", syscall.EAGAIN, 0},
		{"interrupted", syscall.EINTR, 0}, {"codec-failure", io.ErrNoProgress, 0},
		{"timeout-wrapping-reset", temporaryReset{}, 0},
	} {
		t.Run(tc.name, func(t *testing.T) {
			a, b := net.Pipe()
			defer b.Close()
			var calls atomic.Int32
			c := New(context.Background(), &readErrorConn{a, tc.err}, func(context.Context) (net.Conn, error) {
				calls.Add(1)
				return nil, errors.New("controlled redial failure")
			}, nil)
			defer c.Close()
			_, _ = c.Write(hello())
			_, _ = c.Read(make([]byte, 1))
			if calls.Load() != tc.retries {
				t.Fatal("retry count", calls.Load())
			}
		})
	}
}

func TestRecoveryBudgetStartsAtEOF(t *testing.T) {
	c, s, calls := setup(t)
	c.recoveryTimeout = 20 * time.Millisecond
	read := make(chan struct{})
	go func() { _, _ = io.CopyN(io.Discard, s, int64(len(hello()))); close(read) }()
	_, _ = c.Write(hello())
	<-read
	time.Sleep(55 * time.Millisecond) // Original lifetime must not consume recovery time.
	s.Close()
	b := make([]byte, 3)
	if _, e := io.ReadFull(c, b); e != nil || string(b) != "TLS" {
		t.Fatal(e, string(b))
	}
	if calls.Load() != 1 {
		t.Fatal("retry count", calls.Load())
	}
}

func TestRecoveryBudgetBoundsDialAndCannotBeExtendedByCallerClear(t *testing.T) {
	c, s, _ := setup(t)
	c.recoveryTimeout = 25 * time.Millisecond
	entered := make(chan struct{})
	c.dial = func(ctx context.Context) (net.Conn, error) { close(entered); <-ctx.Done(); return nil, ctx.Err() }
	go func() { defer s.Close(); _, _ = io.CopyN(io.Discard, s, int64(len(hello()))) }()
	_, _ = c.Write(hello())
	done := make(chan error, 1)
	go func() { _, e := c.Read(make([]byte, 1)); done <- e }()
	<-entered
	_ = c.SetDeadline(time.Now().Add(time.Hour))
	_ = c.SetDeadline(time.Time{})
	select {
	case e := <-done:
		if !errors.Is(e, os.ErrDeadlineExceeded) {
			t.Fatal(e)
		}
	case <-time.After(time.Second):
		t.Fatal("caller clear extended fixed recovery budget")
	}
}

func TestRecoveryBudgetIncludesFreshReplyAndClosesFreshSocket(t *testing.T) {
	c, s, _ := setup(t)
	c.recoveryTimeout = 25 * time.Millisecond
	freshClosed := make(chan error, 1)
	c.dial = func(context.Context) (net.Conn, error) {
		a, b := net.Pipe()
		go func() {
			defer b.Close()
			_, _ = io.CopyN(io.Discard, b, int64(len(hello())))
			_, e := b.Read(make([]byte, 1))
			freshClosed <- e
		}()
		return a, nil
	}
	go func() { defer s.Close(); _, _ = io.CopyN(io.Discard, s, int64(len(hello()))) }()
	_, _ = c.Write(hello())
	_, e := c.Read(make([]byte, 1))
	if !errors.Is(e, os.ErrDeadlineExceeded) {
		t.Fatal(e)
	}
	select {
	case e := <-freshClosed:
		if !errors.Is(e, io.EOF) {
			t.Fatal(e)
		}
	case <-time.After(time.Second):
		t.Fatal("expired recovery socket was not closed")
	}
}

func ineligibleFlights() [][]byte {
	var flights [][]byte
	for _, suite := range []uint16{0x002c, 0x008c, 0x00a8, 0xc035, 0xccab, 0xd001, 0xffff} {
		b := hello()
		b[46] = byte(suite >> 8)
		b[47] = byte(suite)
		flights = append(flights, b)
	}
	b := hello()
	b[43] = 1
	b = append(b[:44], append([]byte{1}, b[44:]...)...)
	b[4]++
	b[8]++
	flights = append(flights, b)
	for _, ext := range []uint16{35, 41, 42, 0xfe0d} {
		b = append(hello(), 0, 4, byte(ext>>8), byte(ext), 0, 0)
		b[4] += 6
		b[8] += 6
		flights = append(flights, b)
	}
	flights = append(flights, hello()[:len(hello())-1])
	return flights
}

func TestIneligibleOrPartialHelloIsTransparentAndNeverReplayed(t *testing.T) {
	for i, flight := range ineligibleFlights() {
		c, s, calls := setup(t)
		if ValidFlight(flight) {
			t.Fatal("ineligible flight admitted", i)
		}
		got := make(chan []byte, 1)
		go func() { defer s.Close(); b, _ := io.ReadAll(s); got <- b }()
		if _, e := c.Write(flight); e != nil {
			t.Fatal(e)
		}
		// Close the peer after exactly this write; preserve every original byte.
		s.Close()
		_, _ = c.Read(make([]byte, 1))
		if !bytes.Equal(<-got, flight) {
			t.Fatal("transparent data changed", i)
		}
		if calls.Load() != 0 {
			t.Fatal("ineligible data replayed", i)
		}
	}
}

type rejectDeadlineConn struct {
	net.Conn
	failure                             error
	rejectRead                          bool
	rejectWriteCall, writeDeadlineCalls int
	closed                              atomic.Bool
}

func (c *rejectDeadlineConn) Write(p []byte) (int, error) { return len(p), nil }
func (c *rejectDeadlineConn) Read([]byte) (int, error)    { return 0, io.EOF }
func (c *rejectDeadlineConn) SetReadDeadline(time.Time) error {
	if c.rejectRead {
		return c.failure
	}
	return nil
}
func (c *rejectDeadlineConn) SetWriteDeadline(time.Time) error {
	c.writeDeadlineCalls++
	if c.writeDeadlineCalls == c.rejectWriteCall {
		return c.failure
	}
	return nil
}
func (c *rejectDeadlineConn) Close() error { c.closed.Store(true); return c.Conn.Close() }

func TestRecoveryDeadlineFailureClosesUnpublishedFreshSocket(t *testing.T) {
	for _, tc := range []struct {
		name      string
		read      bool
		writeCall int
	}{
		{"budget-write", false, 1}, {"restore-read", true, 0}, {"restore-write", false, 2},
	} {
		t.Run(tc.name, func(t *testing.T) {
			c, s, _ := setup(t)
			a, b := net.Pipe()
			defer b.Close()
			failure := errors.New("controlled deadline failure")
			fresh := &rejectDeadlineConn{Conn: a, failure: failure, rejectRead: tc.read, rejectWriteCall: tc.writeCall}
			c.dial = func(context.Context) (net.Conn, error) { return fresh, nil }
			go func() { defer s.Close(); _, _ = io.CopyN(io.Discard, s, int64(len(hello()))) }()
			_, _ = c.Write(hello())
			if _, err := c.Read(make([]byte, 1)); !errors.Is(err, failure) {
				t.Fatal(err)
			}
			if !fresh.closed.Load() {
				t.Fatal("fresh socket leaked")
			}
			if c.conn != nil {
				t.Fatal("failed fresh socket was published")
			}
		})
	}
}

type blockingWriteConn struct {
	net.Conn
	entered chan struct{}
}

func (c *blockingWriteConn) Write(p []byte) (int, error) { close(c.entered); return c.Conn.Write(p) }

func TestCloseCancelsBlockedRecoveryWrite(t *testing.T) {
	c, s, _ := setup(t)
	a, b := net.Pipe()
	defer b.Close()
	entered := make(chan struct{})
	c.dial = func(context.Context) (net.Conn, error) { return &blockingWriteConn{a, entered}, nil }
	go func() { defer s.Close(); _, _ = io.CopyN(io.Discard, s, int64(len(hello()))) }()
	_, _ = c.Write(hello())
	done := make(chan error, 1)
	go func() { _, e := c.Read(make([]byte, 1)); done <- e }()
	<-entered
	_ = c.Close()
	select {
	case err := <-done:
		if !errors.Is(err, net.ErrClosed) {
			t.Fatal(err)
		}
	case <-time.After(time.Second):
		t.Fatal("Close did not release the fresh write")
	}
}

func TestCanceledContextDoesNotDialAfterEOF(t *testing.T) {
	a, b := net.Pipe()
	defer b.Close()
	ctx, cancel := context.WithCancel(context.Background())
	var calls atomic.Int32
	c := New(ctx, &readErrorConn{a, io.EOF}, func(context.Context) (net.Conn, error) {
		calls.Add(1)
		return nil, errors.New("unexpected dial after cancel")
	}, nil)
	defer c.Close()
	_, _ = c.Write(hello())
	cancel()
	if _, e := c.Read(make([]byte, 1)); !errors.Is(e, context.Canceled) {
		t.Fatal(e)
	}
	if calls.Load() != 0 {
		t.Fatal("canceled context redialed")
	}
}

type helloPrefixWriteConn struct{ net.Conn }

func (c *helloPrefixWriteConn) Write([]byte) (int, error) { return len(hello()), io.ErrShortWrite }
func (c *helloPrefixWriteConn) Read([]byte) (int, error)  { return 0, io.EOF }
func TestCoalescedExtraDataDisablesRecoveryBeforePartialWrite(t *testing.T) {
	a, b := net.Pipe()
	defer b.Close()
	var calls atomic.Int32
	c := New(context.Background(), &helloPrefixWriteConn{a}, func(context.Context) (net.Conn, error) {
		calls.Add(1)
		return nil, errors.New("unexpected redial")
	}, nil)
	defer c.Close()
	wire := append(hello(), 23, 3, 3, 0, 1, 0)
	if n, e := c.Write(wire); n != len(hello()) || e != io.ErrShortWrite {
		t.Fatal(n, e)
	}
	if !c.disabled || len(c.flight) != 0 {
		t.Fatal("coalesced extra data remained eligible")
	}
	_, _ = c.Read(make([]byte, 1))
	if calls.Load() != 0 {
		t.Fatal("coalesced business write allowed recovery")
	}
}
