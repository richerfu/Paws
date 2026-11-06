package main

import (
	"context"
	"net"
	"sync/atomic"
	"time"

	"github.com/sagernet/sing-box/adapter"
	"github.com/sagernet/sing/common/bufio"
	N "github.com/sagernet/sing/common/network"
)

// Counts routed payload bytes, including direct traffic. sing's counter wrappers
// preserve counters when the copy loop unwraps connections for optimized I/O.
type trafficTracker struct {
	upload   atomic.Int64
	download atomic.Int64
	lastAt   time.Time
	lastUp   int64
	lastDown int64
}

type trafficSnapshot struct {
	uploadSpeed   uint64
	downloadSpeed uint64
	uploadTotal   uint64
	downloadTotal uint64
}

var _ adapter.ConnectionTracker = (*trafficTracker)(nil)

func newTrafficTracker(now time.Time) *trafficTracker {
	return &trafficTracker{lastAt: now}
}

func (t *trafficTracker) RoutedConnection(_ context.Context, conn net.Conn, _ adapter.InboundContext, _ adapter.Rule, _ adapter.Outbound) net.Conn {
	// Reading the inbound is device -> network; writing is network -> device.
	return bufio.NewInt64CounterConn(conn, []*atomic.Int64{&t.upload}, []*atomic.Int64{&t.download})
}

func (t *trafficTracker) RoutedPacketConnection(_ context.Context, conn N.PacketConn, _ adapter.InboundContext, _ adapter.Rule, _ adapter.Outbound) N.PacketConn {
	return bufio.NewInt64CounterPacketConn(conn, []*atomic.Int64{&t.upload}, nil, []*atomic.Int64{&t.download}, nil)
}

// The bridge's runtimeState lock serializes sampling and session replacement.
func (t *trafficTracker) sample(now time.Time) trafficSnapshot {
	up, down := t.upload.Load(), t.download.Load()
	snapshot := trafficSnapshot{uploadTotal: uint64(up), downloadTotal: uint64(down)}
	if elapsed := now.Sub(t.lastAt).Seconds(); elapsed > 0 {
		snapshot.uploadSpeed = uint64(float64(up-t.lastUp) / elapsed)
		snapshot.downloadSpeed = uint64(float64(down-t.lastDown) / elapsed)
		t.lastAt, t.lastUp, t.lastDown = now, up, down
	}
	return snapshot
}
