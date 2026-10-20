package main

import (
	"context"
	"io"
	"net"
	"testing"
	"time"

	"github.com/sagernet/sing-box/adapter"
	"github.com/sagernet/sing/common/buf"
	"github.com/sagernet/sing/common/bufio"
)

func TestTrafficTCPDirectionAndSampling(t *testing.T) {
	start := time.Now()
	tracker := newTrafficTracker(start)
	device, inbound := net.Pipe()
	defer device.Close()
	defer inbound.Close()
	_ = device.SetDeadline(start.Add(5 * time.Second))
	_ = inbound.SetDeadline(start.Add(5 * time.Second))
	conn := tracker.RoutedConnection(context.Background(), inbound, adapter.InboundContext{}, nil, nil)
	done := make(chan error, 1)
	go func() {
		_, err := device.Write([]byte("upload"))
		if err == nil {
			_, err = io.ReadFull(device, make([]byte, len("download")))
		}
		done <- err
	}()
	if _, err := io.ReadFull(conn, make([]byte, len("upload"))); err != nil {
		t.Fatal(err)
	}
	if _, err := conn.Write([]byte("download")); err != nil {
		t.Fatal(err)
	}
	if err := <-done; err != nil {
		t.Fatal(err)
	}
	sample := tracker.sample(start.Add(2 * time.Second))
	if sample != (trafficSnapshot{uploadSpeed: 3, downloadSpeed: 4, uploadTotal: 6, downloadTotal: 8}) {
		t.Fatalf("wrong TCP totals or elapsed-time rate: %+v", sample)
	}
	idle := tracker.sample(start.Add(3 * time.Second))
	if idle != (trafficSnapshot{uploadTotal: 6, downloadTotal: 8}) {
		t.Fatalf("idle sampling must keep totals and zero rates: %+v", idle)
	}
	// A new engine session owns fresh counters and a fresh sampling baseline.
	restarted := newTrafficTracker(start.Add(4 * time.Second))
	if got := restarted.sample(start.Add(5 * time.Second)); got != (trafficSnapshot{}) {
		t.Fatalf("new session inherited counters: %+v", got)
	}
}

func TestTrafficUDPDirectionAndClosedConnections(t *testing.T) {
	start := time.Now()
	tracker := newTrafficTracker(start)
	inbound, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer inbound.Close()
	device, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer device.Close()
	_ = inbound.SetDeadline(start.Add(5 * time.Second))
	_ = device.SetDeadline(start.Add(5 * time.Second))
	conn := tracker.RoutedPacketConnection(context.Background(), bufio.NewPacketConn(inbound), adapter.InboundContext{}, nil, nil)
	if _, err := device.WriteTo([]byte("upload"), inbound.LocalAddr()); err != nil {
		t.Fatal(err)
	}
	packet := buf.NewPacket()
	defer packet.Release()
	destination, err := conn.ReadPacket(packet)
	if err != nil {
		t.Fatal(err)
	}
	if string(packet.Bytes()) != "upload" {
		t.Fatal("UDP payload changed")
	}
	packet.Reset()
	_, _ = packet.Write([]byte("download"))
	if err := conn.WritePacket(packet, destination); err != nil {
		t.Fatal(err)
	}
	response := make([]byte, 64)
	n, _, err := device.ReadFrom(response)
	if err != nil || string(response[:n]) != "download" {
		t.Fatalf("UDP response: %q, %v", response[:n], err)
	}
	_ = conn.Close()
	sample := tracker.sample(start.Add(500 * time.Millisecond))
	if sample != (trafficSnapshot{uploadSpeed: 12, downloadSpeed: 16, uploadTotal: 6, downloadTotal: 8}) {
		t.Fatalf("wrong UDP direction or lost closed connection totals: %+v", sample)
	}
}

func TestTrafficOptimizedCopyRetainsCounters(t *testing.T) {
	start := time.Now()
	tracker := newTrafficTracker(start)
	device, inbound := net.Pipe()
	defer device.Close()
	defer inbound.Close()
	_ = device.SetDeadline(start.Add(5 * time.Second))
	_ = inbound.SetDeadline(start.Add(5 * time.Second))
	conn := tracker.RoutedConnection(context.Background(), inbound, adapter.InboundContext{}, nil, nil)
	done := make(chan error, 1)
	go func() {
		_, err := device.Write([]byte("optimized-copy"))
		device.Close()
		done <- err
	}()
	if _, err := bufio.Copy(io.Discard, conn); err != nil {
		t.Fatal(err)
	}
	if err := <-done; err != nil {
		t.Fatal(err)
	}
	if sample := tracker.sample(start.Add(time.Second)); sample.uploadTotal != 14 || sample.downloadTotal != 0 {
		t.Fatalf("optimized copy dropped counters: %+v", sample)
	}
}
