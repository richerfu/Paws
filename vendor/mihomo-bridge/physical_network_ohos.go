//go:build openharmony && cgo

package main

/*
#cgo LDFLAGS: -lnet_connection
#include <stdint.h>
#include <stdlib.h>
#include <network/netmanager/net_connection.h>
*/
import "C"

import (
	"net/netip"
	"strings"
	"sync/atomic"
	"unsafe"
)

var activePhysicalNetID atomic.Int32

func physicalDNSServers(netID int) []string {
	if netID <= 0 {
		return nil
	}
	prop := (*C.NetConn_ConnectionProperties)(C.calloc(1, C.sizeof_NetConn_ConnectionProperties))
	if prop == nil {
		return nil
	}
	defer C.free(unsafe.Pointer(prop))
	handle := C.NetConn_NetHandle{netId: C.int32_t(netID)}
	if rc := C.OH_NetConn_GetConnectionProperties(&handle, prop); rc != 0 {
		return nil
	}
	name := C.GoString(&prop.ifaceName[0])
	if name == "" || name == "vpn-tun" || name == "lo" {
		return nil
	}
	var servers []string
	for i := 0; i < int(prop.dnsListSize) && i < int(C.NETCONN_MAX_ADDR_SIZE); i++ {
		raw := C.GoString(&prop.dnsList[i].address[0])
		address, err := netip.ParseAddr(raw)
		if err != nil || address.IsLoopback() || address.IsUnspecified() {
			continue
		}
		servers = append(servers, address.String())
	}
	return servers
}

//export TinyPawsMihomoSetPhysicalNetwork
func TinyPawsMihomoSetPhysicalNetwork(netID C.int32_t) {
	if netID <= 0 {
		activePhysicalNetID.Store(0)
		return
	}
	prev := activePhysicalNetID.Swap(int32(netID))
	servers := physicalDNSServers(int(netID))
	if len(servers) == 0 {
		servers = []string{"223.5.5.5", "119.29.29.29"}
	}
	handleUpdateDnsSync(strings.Join(servers, ","))
	if prev > 0 && prev != int32(netID) {
		handleCloseConnections()
		handleResetConnections()
	}
}
