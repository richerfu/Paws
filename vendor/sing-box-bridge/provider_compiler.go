package main

/*
#include "provider_compiler.h"
*/
import "C"

import (
	"encoding/json"
	"errors"
	"unsafe"
)

const pawsGeoInputLimit = 64 * 1024 * 1024
const pawsProviderInputLimit = 8 * 1024 * 1024
const pawsProviderOutputLimit = 16 * 1024 * 1024

type pawsCompiledProvider struct {
	Outbounds []json.RawMessage `json:"outbounds"`
	Rules     []json.RawMessage `json:"rules"`
	RuleCount int               `json:"ruleCount"`
}

//export TinyPawsSingSetProviderCompiler
func TinyPawsSingSetProviderCompiler(callback unsafe.Pointer) C.int {
	return C.paws_set_provider_compiler(callback)
}

// Initial load and refresh share this exact converter. The native owner checks
// its ticket before/after conversion and immediately before atomic publication.
func compilePawsProvider(kind, behavior, format uint32, body []byte) (pawsCompiledProvider, error) {
	return callPawsCompiler(kind, behavior, format, body, pawsProviderInputLimit)
}
func callPawsCompiler(kind, behavior, format uint32, body []byte, inputLimit int) (pawsCompiledProvider, error) {
	if len(body) > inputLimit {
		return pawsCompiledProvider{}, errors.New("provider payload exceeds byte budget")
	}
	var data *C.uint8_t
	if len(body) > 0 {
		data = (*C.uint8_t)(unsafe.Pointer(&body[0]))
	}
	var length C.size_t
	output := C.paws_compile_provider(C.uint32_t(kind), C.uint32_t(behavior), C.uint32_t(format), data, C.size_t(len(body)), &length)
	if output == nil {
		return pawsCompiledProvider{}, errors.New("provider compiler is unavailable or rejected its ABI buffer")
	}
	defer C.free(output)
	if uint64(length) > pawsProviderOutputLimit {
		return pawsCompiledProvider{}, errors.New("provider compiler result exceeds byte budget")
	}
	var envelope struct {
		OK    bool                 `json:"ok"`
		Data  pawsCompiledProvider `json:"data"`
		Error string               `json:"error"`
	}
	if err := json.Unmarshal(C.GoBytes(output, C.int(length)), &envelope); err != nil {
		return pawsCompiledProvider{}, errors.New("invalid provider compiler response")
	}
	if !envelope.OK {
		return pawsCompiledProvider{}, errors.New(envelope.Error)
	}
	if kind == 1 && envelope.Data.Outbounds == nil {
		return pawsCompiledProvider{}, errors.New("provider compiler omitted outbound sequence")
	}
	if kind >= 2 && envelope.Data.Rules == nil {
		return pawsCompiledProvider{}, errors.New("provider compiler omitted rule sequence")
	}
	return envelope.Data, nil
}
