//go:build paws_host_test

package main

/*
#include <stddef.h>
#include <stdint.h>
*/
import "C"

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"strings"
	"sync"
	"unicode/utf8"
	"unsafe"

	box "github.com/sagernet/sing-box"
	"github.com/sagernet/sing-box/option"
	SJ "github.com/sagernet/sing/common/json"
)

// This owner exists only in an explicitly tagged host test library. It starts
// the same managed runtime/registry/provider transaction with no platform TUN.
// The Rust acceptance process installs its real production compiler callback.
var pawsHostProbe struct {
	sync.Mutex
	owner    *pawsSingRuntime
	instance *box.Box
	err      string
}

func main() {}

func pawsHostInput(data *C.uint8_t, length C.size_t, limit uint64) ([]byte, error) {
	if uint64(length) > limit || (length != 0 && data == nil) {
		return nil, errors.New("invalid bounded host probe buffer")
	}
	if length == 0 {
		return nil, nil
	}
	return C.GoBytes(unsafe.Pointer(data), C.int(length)), nil
}

//export TinyPawsSingHostStart
func TinyPawsSingHostStart(data *C.uint8_t, length C.size_t, directory *C.uint8_t, directoryLength C.size_t) C.int {
	pawsHostProbe.Lock()
	defer pawsHostProbe.Unlock()
	fail := func(err error) C.int { pawsHostProbe.err = err.Error(); return -1 }
	if pawsHostProbe.owner != nil || pawsHostProbe.instance != nil {
		return fail(errors.New("host probe owner already running"))
	}
	raw, err := pawsHostInput(data, length, 16*1024*1024)
	if err != nil {
		return fail(err)
	}
	path, err := pawsHostInput(directory, directoryLength, 4096)
	if err != nil || !utf8.Valid(path) || strings.IndexByte(string(path), 0) >= 0 || !filepath.IsAbs(string(path)) {
		return fail(errors.New("host probe needs an absolute UTF-8 directory"))
	}
	var document map[string]json.RawMessage
	if err = json.Unmarshal(raw, &document); err != nil {
		return fail(errors.New("invalid host probe config"))
	}
	var inbounds []json.RawMessage
	if err = json.Unmarshal(document["inbounds"], &inbounds); err != nil {
		return fail(errors.New("invalid host probe inbounds"))
	}
	filtered := make([]json.RawMessage, 0, len(inbounds))
	for _, inbound := range inbounds {
		var kind struct {
			Type string `json:"type"`
		}
		if err = json.Unmarshal(inbound, &kind); err != nil {
			return fail(errors.New("invalid host probe inbound"))
		}
		if kind.Type != "tun" {
			filtered = append(filtered, inbound)
		}
	}
	// The Rust caller uses the actual production renderer. The only host
	// adaptation is removing platform TUN inbounds; DNS, routing, providers,
	// mixed authentication and native metadata are never rewritten here.
	document["inbounds"], err = json.Marshal(filtered)
	if err != nil {
		return fail(err)
	}
	raw, err = json.Marshal(document)
	if err != nil {
		return fail(err)
	}
	owner, native, err := newPawsSingRuntime(context.Background(), raw, string(path))
	if err != nil {
		return fail(err)
	}
	ctx := owner.nativeContext()
	options, err := SJ.UnmarshalExtendedContext[option.Options](ctx, native)
	if err != nil {
		_ = owner.close()
		return fail(err)
	}
	instance, err := box.New(box.Options{Context: ctx, Options: options})
	if err != nil {
		_ = owner.close()
		return fail(err)
	}
	if err = instance.StartWithPreparation(func() error { return owner.initialize(instance) }); err != nil {
		_ = owner.close()
		_ = instance.Close()
		return fail(err)
	}
	if err = owner.started(); err != nil {
		_ = owner.close()
		_ = instance.Close()
		return fail(err)
	}
	pawsHostProbe.owner, pawsHostProbe.instance, pawsHostProbe.err = owner, instance, ""
	return 0
}

//export TinyPawsSingHostStop
func TinyPawsSingHostStop() C.int {
	pawsHostProbe.Lock()
	defer pawsHostProbe.Unlock()
	var err error
	if pawsHostProbe.owner != nil {
		err = pawsHostProbe.owner.close()
	}
	if pawsHostProbe.instance != nil {
		closeErr := pawsHostProbe.instance.Close()
		if err == nil {
			err = closeErr
		}
	}
	pawsHostProbe.owner, pawsHostProbe.instance = nil, nil
	if err != nil {
		pawsHostProbe.err = err.Error()
		return -1
	}
	pawsHostProbe.err = ""
	return 0
}

//export TinyPawsSingHostCopyLastError
func TinyPawsSingHostCopyLastError(destination *C.uint8_t, capacity C.size_t) C.size_t {
	pawsHostProbe.Lock()
	defer pawsHostProbe.Unlock()
	message := []byte(pawsHostProbe.err)
	count := len(message)
	if uint64(capacity) < uint64(count) {
		count = int(capacity)
	}
	if destination != nil && count != 0 {
		copy(unsafe.Slice((*byte)(unsafe.Pointer(destination)), count), message[:count])
	}
	return C.size_t(len(message))
}
