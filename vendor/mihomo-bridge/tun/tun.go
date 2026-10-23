//go:build (android || openharmony) && cgo

package tun

import "C"
import (
	"syscall"

	"github.com/metacubex/mihomo/listener/sing_tun"
	"github.com/metacubex/mihomo/log"
	"github.com/metacubex/mihomo/tunnel"
)

// Start borrows fd from the platform VPN connection. HarmonyOS keeps ownership
// of the descriptor returned by VpnConnection.create(), and DestroyVpn closes
// it later. Closing that descriptor here makes NetworkVpnClient::DestroyVpn
// close an already-owned fd and is fatal under HarmonyOS FDSAN.
//
// dupFd is handed to sing_tun.New, which only takes ownership of it once
// tunNew succeeds inside New; from that point
// Listener.Close (run by New's own deferred cleanup on error) closes dupFd,
// so this must never close dupFd itself. New's error alone can't say
// whether tunNew was reached; the options built below never enable the
// tun features whose validation runs before tunNew, so New cannot fail
// before taking ownership of dupFd.
func Start(fd int, stack string, address, dns string, mtu uint32) *sing_tun.Listener {
	options, err := interfaceOptions(stack, address, dns, mtu)
	if err != nil {
		log.Errorln("TUN: %v", err)
		return nil
	}
	dupFd, err := syscall.Dup(fd)
	if err != nil {
		log.Errorln("TUN: %v", err)
		return nil
	}
	options.FileDescriptor = dupFd

	listener, err := sing_tun.New(options, tunnel.Tunnel)

	if err != nil {
		log.Errorln("TUN: %v", err)
		return nil
	}

	return listener
}
