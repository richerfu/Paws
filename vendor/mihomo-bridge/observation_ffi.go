//go:build (android || openharmony) && cgo

package main

/*
#include <stdlib.h>
#include <string.h>
*/
import "C"

import (
	"encoding/json"
	"runtime"
	"runtime/debug"
	"unsafe"
)

//export TinyPawsMihomoCopyLog
func TinyPawsMihomoCopyLog(destination *C.char, capacity C.size_t) C.size_t {
	journal := pawsMihomoLastLogs.Load()
	if owner := pawsMihomoCurrent.Load(); owner != nil {
		journal = owner.observations.logs
	}
	if journal == nil {
		return 0
	}
	page, err := journal.page(0, 512)
	if err != nil {
		return 0
	}
	var data []byte
	for _, entry := range page.Entries {
		var record pawsLogRecord
		if json.Unmarshal(entry, &record) != nil {
			continue
		}
		data = append(data, record.Timestamp+" "+record.Level+" "+record.Message+"\n"...)
	}
	if destination != nil && capacity > 0 {
		limit := min(len(data), int(capacity)-1)
		if limit > 0 {
			C.memcpy(unsafe.Pointer(destination), unsafe.Pointer(&data[0]), C.size_t(limit))
		}
		*(*byte)(unsafe.Add(unsafe.Pointer(destination), limit)) = 0
	}
	return C.size_t(len(data))
}

//export TinyPawsMihomoStop
func TinyPawsMihomoStop() *C.char {
	if owner := pawsMihomoCurrent.Load(); owner != nil {
		if err := owner.close(); err != nil {
			return C.CString(err.Error())
		}
	}
	tunLock.Lock()
	stopTunLocked()
	tunLock.Unlock()
	if !handleStopListener() {
		return C.CString("native inbound listener cleanup failed")
	}
	return nil
}

//export TinyPawsMihomoTrimMemory
func TinyPawsMihomoTrimMemory() {
	// Reclaim garbage without invalidating live outbound transports, DNS
	// fake-IP mappings, group selections, or routing state.
	runtime.GC()
	debug.FreeOSMemory()
}
