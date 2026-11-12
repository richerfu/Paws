//go:build !paws_host_test

package main

/*
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include <stdlib.h>
#include <net/if.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
static int tiny_paws_tcp_info(int fd, struct tcp_info *info) { socklen_t size=sizeof(*info); return getsockopt(fd,IPPROTO_TCP,TCP_INFO,info,&size); }
#include <network/netmanager/net_connection.h>
*/
import "C"

import (
	"errors"
	"fmt"
	"github.com/sagernet/sing-box/adapter"
	constant "github.com/sagernet/sing-box/constant"
	"github.com/sagernet/sing/common/control"
	"net"
	"net/netip"
	"syscall"
	"unsafe"
)

// Use the interface attached to the physical netId. Go's Linux interface
// discovery does not populate sing-box's platform NetworkInterfaces registry.
func physicalInterface(netID int) (adapter.NetworkInterface, error) {
	prop := (*C.NetConn_ConnectionProperties)(C.calloc(1, C.sizeof_NetConn_ConnectionProperties))
	if prop == nil {
		return adapter.NetworkInterface{}, errors.New("physical network allocation failed")
	}
	defer C.free(unsafe.Pointer(prop))
	handle := C.NetConn_NetHandle{netId: C.int32_t(netID)}
	if rc := C.OH_NetConn_GetConnectionProperties(&handle, prop); rc != 0 {
		return adapter.NetworkInterface{}, fmt.Errorf("physical network properties failed: %d", int(rc))
	}
	name := C.GoString(&prop.ifaceName[0])
	if name == "" || name == "vpn-tun" || name == "lo" {
		return adapter.NetworkInterface{}, errors.New("physical interface unavailable")
	}
	index := int(C.if_nametoindex(&prop.ifaceName[0]))
	if index <= 0 {
		return adapter.NetworkInterface{}, errors.New("physical interface index unavailable")
	}
	iif := adapter.NetworkInterface{Interface: control.Interface{Index: index, Name: name, MTU: int(prop.mtu), Flags: net.FlagUp | net.FlagRunning}, Type: constant.InterfaceTypeOther}
	for i := 0; i < int(prop.netAddrListSize) && i < int(C.NETCONN_MAX_ADDR_SIZE); i++ {
		a := prop.netAddrList[i]
		address, err := netip.ParseAddr(C.GoString(&a.address[0]))
		if err != nil {
			continue
		}
		if int(a.prefixlen) <= address.BitLen() {
			iif.Addresses = append(iif.Addresses, netip.PrefixFrom(address, int(a.prefixlen)))
		}
	}
	for i := 0; i < int(prop.dnsListSize) && i < int(C.NETCONN_MAX_ADDR_SIZE); i++ {
		a := prop.dnsList[i]
		address, err := netip.ParseAddr(C.GoString(&a.address[0]))
		if err == nil {
			iif.DNSServers = append(iif.DNSServers, address.String())
		}
	}
	if iif.MTU == 0 {
		iif.MTU = 1500
	}
	return iif, nil
}

func nativeTCPInfo(conn net.Conn) string {
	if !socketTraceEnabled.Load() {
		return ""
	}
	raw, ok := conn.(syscall.Conn)
	if !ok {
		return " tcpInfo=unavailable"
	}
	handle, err := raw.SyscallConn()
	if err != nil {
		return " tcpInfo=unavailable"
	}
	result := " tcpInfo=unavailable"
	_ = handle.Control(func(fd uintptr) {
		var info C.struct_tcp_info
		if C.tiny_paws_tcp_info(C.int(fd), &info) == 0 {
			result = fmt.Sprintf(" tcpState=%d unacked=%d lost=%d retrans=%d rttUs=%d totalRetrans=%d", info.tcpi_state, info.tcpi_unacked, info.tcpi_lost, info.tcpi_retrans, info.tcpi_rtt, info.tcpi_total_retrans)
		}
	})
	return result
}
