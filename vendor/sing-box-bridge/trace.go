//go:build !paws_host_test

package main

import (
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	boxlog "github.com/sagernet/sing-box/log"
	"io"
	"net"
	"sync"
	"sync/atomic"
	"syscall"
	"time"
)

var activeTraceLog atomic.Pointer[ringLog]
var nextTraceID atomic.Uint64
var socketTraceEnabled atomic.Bool

func traceMessage(message string) {
	if l := activeTraceLog.Load(); l != nil {
		l.WriteMessage(boxlog.LevelInfo, message)
	}
}

type tracedSocket struct {
	net.Conn
	id             uint64
	peer           string
	flow           uint32
	sent, received atomic.Int64
	closeOnce      sync.Once
	peerEnded      atomic.Bool
	localClosed    atomic.Bool
}

func newTracedSocket(ctx context.Context, c net.Conn) net.Conn {
	hash := sha256.Sum256([]byte(c.RemoteAddr().String()))
	flow, _ := boxlog.IDFromContext(ctx)
	t := &tracedSocket{Conn: c, id: nextTraceID.Add(1), peer: fmt.Sprintf("%x", hash[:8]), flow: flow.ID}
	traceMessage(fmt.Sprintf("obfs-session id=%d flow=%d atMs=%d event=connected peer=%s local=%s", t.id, t.flow, time.Now().UnixMilli(), t.peer, c.LocalAddr()))
	return t
}
func (t *tracedSocket) Read(p []byte) (int, error) {
	n, e := t.Conn.Read(p)
	total := t.received.Add(int64(n))
	if total == int64(n) && n > 0 {
		traceMessage(fmt.Sprintf("obfs-session id=%d flow=%d atMs=%d peer=%s event=first-read readBytes=%d", t.id, t.flow, time.Now().UnixMilli(), t.peer, n) + nativeTCPInfo(t.Conn))
	}
	if e != nil {
		kind := "io-error"
		if t.localClosed.Load() || errors.Is(e, net.ErrClosed) {
			kind = "local-read-closed"
		} else if errors.Is(e, io.EOF) {
			t.peerEnded.Store(true)
			kind = "peer-fin"
		} else if errors.Is(e, syscall.ECONNRESET) {
			t.peerEnded.Store(true)
			kind = "peer-reset"
		}
		traceMessage(fmt.Sprintf("obfs-session id=%d flow=%d atMs=%d peer=%s direction=proxy-to-client event=%s sent=%d received=%d", t.id, t.flow, time.Now().UnixMilli(), t.peer, kind, t.sent.Load(), t.received.Load()) + nativeTCPInfo(t.Conn))
	}
	return n, e
}
func (t *tracedSocket) Write(p []byte) (int, error) {
	n, e := t.Conn.Write(p)
	first := t.sent.Add(int64(n)) == int64(n)
	if first || e != nil {
		kind := "first-write"
		if e != nil {
			kind = "write-error"
		}
		traceMessage(fmt.Sprintf("obfs-session id=%d flow=%d atMs=%d peer=%s direction=client-to-proxy event=%s writeBytes=%d sent=%d received=%d", t.id, t.flow, time.Now().UnixMilli(), t.peer, kind, n, t.sent.Load(), t.received.Load()) + nativeTCPInfo(t.Conn))
	}
	return n, e
}
func (t *tracedSocket) Close() error {
	t.closeOnce.Do(func() {
		t.localClosed.Store(true)
		traceMessage(fmt.Sprintf("obfs-session id=%d flow=%d atMs=%d event=local-close peer=%s peerEnded=%v sent=%d received=%d", t.id, t.flow, time.Now().UnixMilli(), t.peer, t.peerEnded.Load(), t.sent.Load(), t.received.Load()) + nativeTCPInfo(t.Conn))
	})
	return t.Conn.Close()
}
