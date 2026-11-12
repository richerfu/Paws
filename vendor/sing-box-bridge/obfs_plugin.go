//go:build !paws_host_test

package main

import (
	"context"
	"errors"
	"fmt"
	"github.com/sagernet/sing-box/cmd/libtinypaws/tlsretry"
	boxlog "github.com/sagernet/sing-box/log"
	"net"
	"strconv"

	"github.com/sagernet/sing-box/adapter"
	"github.com/sagernet/sing-box/cmd/libtinypaws/httpobfs"
	obfs "github.com/sagernet/sing-box/transport/simple-obfs"
	"github.com/sagernet/sing-box/transport/sip003"
	M "github.com/sagernet/sing/common/metadata"
	N "github.com/sagernet/sing/common/network"
)

// Registration occurs after imported packages' init functions. Override only
// the HTTP envelope; sing-box still owns encryption, DNS, routing and sockets.
func init() { sip003.RegisterPlugin("obfs-local", newCompatibleObfs) }

type compatibleObfs struct {
	dialer     N.Dialer
	server     M.Socksaddr
	host, mode string
}

func newCompatibleObfs(_ context.Context, args sip003.Args, _ adapter.Router, dialer N.Dialer, server M.Socksaddr) (sip003.Plugin, error) {
	mode, _ := args.Get("obfs")
	if mode == "" {
		mode = "http"
	}
	if mode != "http" && mode != "tls" {
		return nil, errors.New("unsupported HTTP-obfs mode")
	}
	host, _ := args.Get("obfs-host")
	if host == "" {
		host = "bing.com"
	}
	return &compatibleObfs{dialer: dialer, server: server, host: host, mode: mode}, nil
}
func (p *compatibleObfs) DialContext(ctx context.Context) (net.Conn, error) {
	conn, err := p.dialer.DialContext(ctx, N.NetworkTCP, p.server)
	if err != nil {
		return nil, err
	}
	conn = newTracedSocket(ctx, conn)
	if p.mode == "tls" {
		return obfs.NewTLSObfs(conn, p.host), nil
	}
	wrapped, err := httpobfs.New(conn, p.host, strconv.Itoa(int(p.server.Port)))
	if err != nil {
		_ = conn.Close()
		return nil, err
	}
	return wrapped, nil
}

// Match the established Mihomo HTTP-obfs path: flush the destination record
// before the application's first payload. TLS-obfs keeps its own timing.
func (p *compatibleObfs) RequiresEagerShadowsocksHandshake() bool { return p.mode == "http" }

func (p *compatibleObfs) WrapFirstFlight(ctx context.Context, conn net.Conn, redial func(context.Context) (net.Conn, error)) net.Conn {
	if p.mode != "http" {
		return conn
	}
	return tlsretry.New(ctx, conn, redial, func(reason string) {
		flow, _ := boxlog.IDFromContext(ctx)
		traceMessage(fmt.Sprintf("tls-first-flight-retry flow=%d reason=%s", flow.ID, reason))
	})
}

// Bind cancellation before the plugin emits the encrypted destination record.
func (p *compatibleObfs) DialHandshakeContext(ctx context.Context, raw net.Conn, handshake func() (net.Conn, error)) (net.Conn, error) {
	return tlsretry.HandshakeContext(ctx, raw, handshake)
}
