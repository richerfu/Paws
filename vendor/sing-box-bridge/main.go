//go:build !paws_host_test

package main

/*
#cgo LDFLAGS: -lnet_connection
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <network/netmanager/net_connection.h>

static int32_t tiny_paws_bind_socket(int32_t socket_fd, int32_t net_id) {
	NetConn_NetHandle handle = { .netId = net_id };
	return OH_NetConn_BindSocket(socket_fd, &handle);
}
*/
import "C"

import (
	"context"
	"errors"
	"fmt"
	"net/netip"
	"os"
	"path/filepath"
	"runtime"
	"runtime/debug"
	"strings"
	"sync"
	"time"
	"unsafe"

	box "github.com/sagernet/sing-box"
	"github.com/sagernet/sing-box/adapter"
	boxlog "github.com/sagernet/sing-box/log"
	"github.com/sagernet/sing-box/option"
	tun "github.com/sagernet/sing-tun"
	"github.com/sagernet/sing/common/control"
	"github.com/sagernet/sing/common/json"
	"github.com/sagernet/sing/common/logger"
	"github.com/sagernet/sing/common/x/list"
	"github.com/sagernet/sing/service"
	"github.com/sagernet/sing/service/filemanager"
	"golang.org/x/sys/unix"
)

var runtimeState struct {
	sync.Mutex
	instance     *box.Box
	owner        *pawsSingRuntime
	cleanupError error
	cancel       context.CancelFunc
	lastErr      string
	log          *ringLog
	directory    string
	traffic      *trafficTracker
}

type ringLog struct {
	sync.Mutex
	text         string
	observations *pawsObservations
	traceFile    *os.File
	traceBytes   int64
}

func (l *ringLog) WriteMessage(level boxlog.Level, message string) {
	l.Lock()
	defer l.Unlock()
	if l.observations != nil {
		_ = l.observations.log(boxlog.FormatLevel(level), message, time.Now().UTC().Format(time.RFC3339Nano))
	}
	line := boxlog.FormatLevel(level) + " atMs=" + fmt.Sprint(time.Now().UnixMilli()) + " " + message + "\n"
	l.text += line
	if l.traceFile != nil && l.traceBytes+int64(len(line)) <= 8*1024*1024 {
		n, _ := l.traceFile.WriteString(line)
		l.traceBytes += int64(n)
	}
	const maxBytes = 256 * 1024
	if len(l.text) > maxBytes {
		l.text = l.text[len(l.text)-maxBytes:]
		if cut := strings.IndexByte(l.text, '\n'); cut >= 0 {
			l.text = l.text[cut+1:]
		}
	}
}

func (l *ringLog) String() string {
	l.Lock()
	defer l.Unlock()
	return l.text
}

type staticMonitor struct {
	mu        sync.Mutex
	callbacks list.List[tun.DefaultInterfaceUpdateCallback]
	myNames   []string
	physical  *control.Interface
}

func (*staticMonitor) Start() error                           { return nil }
func (*staticMonitor) Close() error                           { return nil }
func (m *staticMonitor) DefaultInterface() *control.Interface { return m.physical }
func (*staticMonitor) OverrideAndroidVPN() bool               { return false }
func (*staticMonitor) AndroidVPNEnabled() bool                { return false }
func (m *staticMonitor) RegisterCallback(callback tun.DefaultInterfaceUpdateCallback) *list.Element[tun.DefaultInterfaceUpdateCallback] {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.callbacks.PushBack(callback)
}
func (m *staticMonitor) UnregisterCallback(element *list.Element[tun.DefaultInterfaceUpdateCallback]) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.callbacks.Remove(element)
}
func (m *staticMonitor) RegisterMyInterface(name string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.myNames = append(m.myNames, name)
}
func (m *staticMonitor) MyInterfaces() []string {
	m.mu.Lock()
	defer m.mu.Unlock()
	return append([]string(nil), m.myNames...)
}

type harmonyPlatform struct {
	fd       int
	netID    int
	monitor  *staticMonitor
	addrs    []netip.Addr
	physical adapter.NetworkInterface
}

func (*harmonyPlatform) Initialize(manager adapter.NetworkManager) error {
	return manager.UpdateInterfaces()
}
func (*harmonyPlatform) UsePlatformAutoDetectInterfaceControl() bool { return true }
func (p *harmonyPlatform) AutoDetectInterfaceControl(fd int) error {
	if p.netID <= 0 {
		return errors.New("TinyPaws underlying network handle is unavailable")
	}
	if result := int32(C.tiny_paws_bind_socket(C.int32_t(fd), C.int32_t(p.netID))); result != 0 {
		return errors.New("TinyPaws failed to bind outbound socket to the underlying network")
	}
	return nil
}
func (*harmonyPlatform) UsePlatformInterface() bool { return true }
func (p *harmonyPlatform) OpenInterface(options *tun.Options, _ option.TunPlatformOptions) (tun.Tun, error) {
	if p.fd < 0 {
		return nil, errors.New("TinyPaws TUN descriptor is unavailable")
	}
	dupFD, err := unix.Dup(p.fd)
	if err != nil {
		return nil, err
	}
	options.FileDescriptor = dupFD
	// HarmonyOS names the VpnConnection-created interface `vpn-tun` even when
	// userspace receives only its descriptor. sing-tun's mixed stack binds a
	// forwarder by this name, so advertising the Android-style `tun0` leaves the
	// TUN readable but prevents replies from being forwarded.
	options.Name = "vpn-tun"
	options.EXP_ExternalConfiguration = true
	options.AutoRoute = false
	options.InterfaceMonitor = p.monitor
	p.addrs = p.addrs[:0]
	for _, prefix := range options.Inet4Address {
		p.addrs = append(p.addrs, prefix.Addr())
	}
	for _, prefix := range options.Inet6Address {
		p.addrs = append(p.addrs, prefix.Addr())
	}
	p.monitor.RegisterMyInterface(options.Name)
	created, createErr := tun.New(*options)
	if createErr != nil {
		_ = unix.Close(dupFD)
		return nil, createErr
	}
	return created, nil
}
func (p *harmonyPlatform) UsePlatformDefaultInterfaceMonitor() bool { return true }
func (p *harmonyPlatform) CreateDefaultInterfaceMonitor(logger.Logger) tun.DefaultInterfaceMonitor {
	return p.monitor
}
func (*harmonyPlatform) UsePlatformNetworkInterfaces() bool { return true }
func (p *harmonyPlatform) NetworkInterfaces() ([]adapter.NetworkInterface, error) {
	return []adapter.NetworkInterface{p.physical}, nil
}
func (*harmonyPlatform) UnderNetworkExtension() bool { return true }

// includeAllNetworks is an Apple NetworkExtension setting. OpenHarmony routes
// the selected application UIDs through the platform-owned TUN before this
// adapter is opened, so reporting it here incorrectly disables sing-tun's
// system/mixed stacks.
func (*harmonyPlatform) NetworkExtensionIncludeAllNetworks() bool { return false }
func (*harmonyPlatform) ClearDNSCache()                           {}
func (*harmonyPlatform) RequestPermissionForWIFIState() error     { return nil }
func (*harmonyPlatform) ReadWIFIState() adapter.WIFIState         { return adapter.WIFIState{} }
func (*harmonyPlatform) SystemCertificates() []string             { return nil }
func (*harmonyPlatform) UsePlatformConnectionOwnerFinder() bool   { return false }
func (*harmonyPlatform) FindConnectionOwner(*adapter.FindConnectionOwnerRequest) (*adapter.ConnectionOwner, error) {
	return nil, os.ErrInvalid
}
func (*harmonyPlatform) UsePlatformWIFIMonitor() bool                 { return false }
func (*harmonyPlatform) UsePlatformNotification() bool                { return false }
func (*harmonyPlatform) SendNotification(*adapter.Notification) error { return nil }
func (p *harmonyPlatform) MyInterfaceAddress() []netip.Addr {
	return append([]netip.Addr(nil), p.addrs...)
}

func setLastError(err error) C.int {
	if err == nil {
		runtimeState.lastErr = ""
		return 0
	}
	runtimeState.lastErr = err.Error()
	return -1
}

func stopLocked() error {
	if runtimeState.cleanupError != nil {
		return runtimeState.cleanupError
	}
	// Revoke credentials, cancel owned requests/timers, join native publications
	// and close keepalive/WS sockets before disposing the native data plane.
	if runtimeState.owner != nil {
		if err := runtimeState.owner.close(); err != nil {
			runtimeState.cleanupError = err
			return err
		}
	}
	if runtimeState.instance != nil {
		if err := runtimeState.instance.Close(); err != nil {
			// Box.Close is terminal even on partial error. Retain the owner and
			// quarantine this process instead of pretending a second Close joined it.
			runtimeState.cleanupError = err
			return err
		}
	}
	runtimeState.instance, runtimeState.owner = nil, nil
	if runtimeState.cancel != nil {
		runtimeState.cancel()
		runtimeState.cancel = nil
	}
	runtimeState.traffic = nil
	if runtimeState.log != nil && runtimeState.directory != "" {
		runtimeState.log.Lock()
		if runtimeState.log.traceFile != nil {
			_ = runtimeState.log.traceFile.Close()
			runtimeState.log.traceFile = nil
		}
		runtimeState.log.Unlock()
		path := filepath.Join(runtimeState.directory, "last-session.log")
		temporary := path + ".tmp"
		if err := os.WriteFile(temporary, []byte(runtimeState.log.String()), 0600); err == nil {
			_ = os.Rename(temporary, path)
		}
	}
	go func() { runtime.GC(); debug.FreeOSMemory() }()
	return nil
}

//export TinyPawsSingVersion
func TinyPawsSingVersion() *C.char {
	return C.CString("sing-box 1.13.21 / TinyPaws OpenHarmony bridge 2")
}

//export TinyPawsSingStart
func TinyPawsSingStart(tunFD C.int, netID C.int, configJSON *C.char, workDir *C.char) C.int {
	runtimeState.Lock()
	defer runtimeState.Unlock()
	if err := stopLocked(); err != nil {
		return setLastError(err)
	}
	if tunFD < 0 || netID <= 0 || configJSON == nil || workDir == nil {
		return setLastError(errors.New("invalid TinyPaws sing-box start arguments"))
	}
	directory := C.GoString(workDir)
	if err := os.MkdirAll(directory, 0o700); err != nil {
		return setLastError(err)
	}
	ctx := filemanager.WithDefault(context.Background(), directory, directory, -1, -1)
	physical, err := physicalInterface(int(netID))
	if err != nil {
		return setLastError(err)
	}
	platform := &harmonyPlatform{fd: int(tunFD), netID: int(netID), physical: physical, monitor: &staticMonitor{physical: &physical.Interface}}
	ctx = service.ContextWith[adapter.PlatformInterface](ctx, platform)
	preparedConfig, err := withPhysicalDNS([]byte(C.GoString(configJSON)), physical.DNSServers)
	if err != nil {
		return setLastError(err)
	}
	owner, native, err := newPawsSingRuntime(ctx, preparedConfig, directory)
	if err != nil {
		return setLastError(err)
	}
	ctx = owner.nativeContext()
	runtimeLog := &ringLog{observations: owner.observations}
	socketTraceEnabled.Store(os.Getenv("PAWS_SOCKET_TRACE") == "1")
	if socketTraceEnabled.Load() {
		runtimeLog.traceFile, _ = os.OpenFile(filepath.Join(directory, "socket-trace.log"), os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0600)
	}
	runtimeState.log, runtimeState.directory = runtimeLog, directory
	activeTraceLog.Store(runtimeLog)
	options, err := json.UnmarshalExtendedContext[option.Options](ctx, native)
	if err != nil {
		_ = owner.close()
		return setLastError(err)
	}
	instance, err := box.New(box.Options{Context: ctx, Options: options, PlatformLogWriter: runtimeLog})
	if err != nil {
		_ = owner.close()
		return setLastError(err)
	}
	// Retain ownership before starting: any partial cleanup error must quarantine
	// the exact instance rather than lose its still-live listener/TUN owners.
	runtimeState.instance, runtimeState.owner = instance, owner
	traffic := newTrafficTracker(time.Now())
	instance.Router().AppendTracker(traffic)
	if err = instance.StartWithPreparation(func() error { return owner.initialize(instance) }); err == nil {
		err = owner.started()
	}
	if err != nil {
		cleanup := stopLocked()
		if cleanup != nil {
			err = errors.Join(err, cleanup)
		}
		return setLastError(err)
	}
	runtimeState.traffic = traffic
	go func() {
		runtime.GC()
		debug.FreeOSMemory()
	}()
	return setLastError(nil)
}

//export TinyPawsSingTraffic
func TinyPawsSingTraffic(uploadSpeed, downloadSpeed, uploadTotal, downloadTotal *C.uint64_t) C.int {
	if uploadSpeed == nil || downloadSpeed == nil || uploadTotal == nil || downloadTotal == nil {
		return -1
	}
	runtimeState.Lock()
	defer runtimeState.Unlock()
	var sample trafficSnapshot
	if runtimeState.traffic != nil {
		sample = runtimeState.traffic.sample(time.Now())
	}
	*uploadSpeed = C.uint64_t(sample.uploadSpeed)
	*downloadSpeed = C.uint64_t(sample.downloadSpeed)
	*uploadTotal = C.uint64_t(sample.uploadTotal)
	*downloadTotal = C.uint64_t(sample.downloadTotal)
	return 0
}

//export TinyPawsSingCopyLog
func TinyPawsSingCopyLog(destination *C.char, capacity C.size_t) C.size_t {
	runtimeState.Lock()
	logBuffer := runtimeState.log
	runtimeState.Unlock()
	if logBuffer == nil {
		return 0
	}
	data := []byte(logBuffer.String())
	if destination == nil || capacity == 0 {
		return C.size_t(len(data))
	}
	limit := int(capacity) - 1
	if len(data) < limit {
		limit = len(data)
	}
	if limit > 0 {
		C.memcpy(unsafe.Pointer(destination), unsafe.Pointer(&data[0]), C.size_t(limit))
	}
	*(*byte)(unsafe.Add(unsafe.Pointer(destination), limit)) = 0
	return C.size_t(len(data))
}

//export TinyPawsSingStop
func TinyPawsSingStop() C.int {
	runtimeState.Lock()
	defer runtimeState.Unlock()
	return setLastError(stopLocked())
}

//export TinyPawsSingTrimMemory
func TinyPawsSingTrimMemory() C.int {
	runtimeState.Lock()
	defer runtimeState.Unlock()
	return setLastError(trimPawsSingRuntimeMemory(runtimeState.owner))
}

//export TinyPawsSingCopyLastError
func TinyPawsSingCopyLastError(destination *C.char, capacity C.size_t) C.size_t {
	runtimeState.Lock()
	defer runtimeState.Unlock()
	if destination == nil || capacity == 0 {
		return C.size_t(len(runtimeState.lastErr))
	}
	data := []byte(runtimeState.lastErr)
	limit := int(capacity) - 1
	if len(data) < limit {
		limit = len(data)
	}
	if limit > 0 {
		C.memcpy(unsafe.Pointer(destination), unsafe.Pointer(&data[0]), C.size_t(limit))
	}
	*(*byte)(unsafe.Add(unsafe.Pointer(destination), limit)) = 0
	return C.size_t(len(data))
}

func main() {}
