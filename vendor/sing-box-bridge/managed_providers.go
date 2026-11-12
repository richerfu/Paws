package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/sagernet/sing-box/adapter"
	"github.com/sagernet/sing-box/option"
	"github.com/sagernet/sing-box/route/rule"
	SJ "github.com/sagernet/sing/common/json"
	M "github.com/sagernet/sing/common/metadata"
	"github.com/sagernet/sing/service"
)

type pawsProviderHealth struct {
	Enable         bool   `json:"enable"`
	URL            string `json:"url"`
	Interval       uint64 `json:"interval"`
	Lazy           bool   `json:"lazy"`
	Timeout        uint64 `json:"timeout"`
	ExpectedStatus string `json:"expected-status"`
}
type pawsProviderDefinition struct {
	Type     string                     `json:"type"`
	URL      string                     `json:"url"`
	Path     string                     `json:"path"`
	Interval uint64                     `json:"interval"`
	Proxy    string                     `json:"proxy"`
	Header   map[string]json.RawMessage `json:"header"`
	Payload  json.RawMessage            `json:"payload"`
	Behavior string                     `json:"behavior"`
	Format   string                     `json:"format"`
	Health   pawsProviderHealth         `json:"health-check"`
	// These are validated explicitly, never silently ignored by the native layer.
	Filter        string          `json:"filter"`
	ExcludeFilter string          `json:"exclude-filter"`
	Override      json.RawMessage `json:"override"`
	SizeLimit     uint64          `json:"size-limit"`
}
type pawsProviderRecord struct {
	// Updated only by actual user dials; querying a provider never wakes lazy health.
	lastTouch  atomic.Int64
	definition pawsProviderDefinition
	options    []option.Outbound
	tags       []string
	updatedAt  string
	rules      []option.HeadlessRule
	ruleCount  int
}
type pawsGroupRoster struct {
	tags  []string
	index map[string]struct{}
}
type pawsOutboundGeneration struct {
	providerByTag map[string]*pawsProviderRecord
	groups        map[string]*pawsGroupRoster
	outbounds     map[string]adapter.Outbound
	order         []string
	providers     map[string]*pawsProviderRecord
	refs          int
	retired       bool
	closing       bool
}
type pawsProviderRegistry struct {
	mu      sync.Mutex
	current atomic.Pointer[pawsOutboundGeneration]
	retired map[*pawsOutboundGeneration]bool
	owner   *pawsSingRuntime
}

func (r *pawsProviderRegistry) Outbound(tag string) (adapter.Outbound, bool) {
	generation := r.current.Load()
	if generation == nil {
		return nil, false
	}
	value, ok := generation.outbounds[tag]
	return value, ok
}
func (r *pawsProviderRegistry) Outbounds() []adapter.Outbound {
	generation := r.current.Load()
	if generation == nil {
		return nil
	}
	result := make([]adapter.Outbound, 0, len(generation.order))
	for _, tag := range generation.order {
		result = append(result, generation.outbounds[tag])
	}
	return result
}
func (r *pawsProviderRegistry) acquire() *pawsOutboundGeneration {
	r.mu.Lock()
	defer r.mu.Unlock()
	generation := r.current.Load()
	if generation != nil {
		generation.refs++
	}
	return generation
}
func (r *pawsProviderRegistry) release(generation *pawsOutboundGeneration) {
	if generation == nil {
		return
	}
	r.mu.Lock()
	generation.refs--
	closeNow := generation.retired && generation.refs == 0 && !generation.closing
	if closeNow {
		generation.closing = true
		delete(r.retired, generation)
	}
	r.mu.Unlock()
	if closeNow {
		closePawsGeneration(generation)
	}
}
func closePawsGeneration(generation *pawsOutboundGeneration) {
	if generation == nil {
		return
	}
	for _, tag := range generation.order {
		if closer, ok := generation.outbounds[tag].(io.Closer); ok {
			_ = closer.Close()
		}
	}
}
func (r *pawsProviderRegistry) publish(generation *pawsOutboundGeneration) error {
	r.mu.Lock()
	if !r.owner.observations.active.Load() {
		r.mu.Unlock()
		return errors.New("runtime retired before provider publication")
	}
	old := r.current.Swap(generation)
	// Staging constructs fresh native objects. Old health belongs to retired
	// objects even when a replacement keeps the same human-facing tag.
	for tag := range generation.outbounds {
		delete(r.owner.delays, tag)
	}
	closeNow := false
	if old != nil {
		old.retired = true
		if old.refs == 0 {
			old.closing = true
			closeNow = true
		} else {
			r.retired[old] = true
		}
	}
	for _, group := range r.owner.groups {
		members := group.members(generation)
		group.mu.Lock()
		if group.fixed != "" && !containsPawsTag(members, group.fixed) {
			group.fixed = ""
		}
		if !containsPawsTag(members, group.automatic) {
			group.automatic = ""
		}
		group.mu.Unlock()
	}
	r.owner.observations.providers.Add(1)
	r.owner.observations.groups.Add(1)
	r.mu.Unlock()
	if closeNow {
		closePawsGeneration(old)
	}
	return nil
}
func (r *pawsProviderRegistry) close() {
	r.mu.Lock()
	all := make([]*pawsOutboundGeneration, 0, len(r.retired)+1)
	if current := r.current.Swap(nil); current != nil {
		all = append(all, current)
	}
	for generation := range r.retired {
		all = append(all, generation)
	}
	r.retired = make(map[*pawsOutboundGeneration]bool)
	for _, generation := range all {
		generation.retired = true
		generation.closing = true
	}
	r.mu.Unlock()
	for _, generation := range all {
		closePawsGeneration(generation)
	}
}
func containsPawsTag(tags []string, tag string) bool {
	for _, value := range tags {
		if value == tag {
			return true
		}
	}
	return false
}

// Staging resolves dependencies against the complete candidate map. It does not
// call Manager.Create/Remove, which mutate and close the live native registry.
type pawsStagingManager struct {
	adapter.OutboundManager
	generation *pawsOutboundGeneration
}

func (s *pawsStagingManager) Outbound(tag string) (adapter.Outbound, bool) {
	if value, ok := s.generation.outbounds[tag]; ok {
		return value, true
	}
	return s.OutboundManager.Outbound(tag)
}
func (s *pawsStagingManager) Outbounds() []adapter.Outbound {
	result := append([]adapter.Outbound(nil), s.OutboundManager.Outbounds()...)
	for _, tag := range s.generation.order {
		result = append(result, s.generation.outbounds[tag])
	}
	return result
}
func (p *pawsSingRuntime) stageProviders(records map[string]*pawsProviderRecord) (generation *pawsOutboundGeneration, err error) {
	generation = &pawsOutboundGeneration{outbounds: make(map[string]adapter.Outbound), providers: records, providerByTag: make(map[string]*pawsProviderRecord)}
	defer func() {
		if err != nil {
			closePawsGeneration(generation)
		}
	}()
	ctx := service.ExtendContext(p.ctx)
	service.MustRegister[adapter.OutboundManager](ctx, &pawsStagingManager{p.manager, generation})
	names := make([]string, 0, len(records))
	for name := range records {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		record := records[name]
		for _, options := range record.options {
			tag := options.Tag
			if tag == "" {
				return generation, errors.New("provider outbound has no tag")
			}
			if _, found := generation.outbounds[tag]; found {
				return generation, errors.New("provider outbound names collide")
			}
			if _, found := p.staticTags[tag]; found {
				return generation, errors.New("provider name collides with a static outbound")
			}
			outbound, err := p.registry.CreateOutbound(ctx, p.router, p.logger, tag, options.Type, options.Options)
			if err != nil {
				return generation, errors.New("native provider outbound construction failed")
			}
			generation.outbounds[tag] = outbound
			generation.providerByTag[tag] = record
			generation.order = append(generation.order, tag)
		}
	}
	started := make(map[string]bool, len(generation.order))
	for len(started) < len(generation.order) {
		progress := false
		for _, tag := range generation.order {
			if started[tag] {
				continue
			}
			outbound := generation.outbounds[tag]
			ready := true
			for _, dependency := range outbound.Dependencies() {
				if _, candidate := generation.outbounds[dependency]; candidate && !started[dependency] {
					ready = false
					break
				}
				if _, candidate := generation.outbounds[dependency]; !candidate {
					if _, found := p.manager.Outbound(dependency); !found {
						return generation, errors.New("provider outbound dependency is unknown")
					}
				}
			}
			if !ready {
				continue
			}
			if err = adapter.LegacyStart(outbound, adapter.StartStateInitialize); err != nil {
				return generation, errors.New("native provider initialization failed")
			}
			if err = adapter.LegacyStart(outbound, adapter.StartStateStart); err != nil {
				return generation, errors.New("native provider start failed")
			}
			started[tag] = true
			progress = true
		}
		if !progress {
			return generation, errors.New("cyclic provider outbound dependencies")
		}
	}
	for _, stage := range []adapter.StartStage{adapter.StartStatePostStart, adapter.StartStateStarted} {
		for _, tag := range generation.order {
			if err = adapter.LegacyStart(generation.outbounds[tag], stage); err != nil {
				return generation, errors.New("native provider finalization failed")
			}
		}
	}
	generation.groups = make(map[string]*pawsGroupRoster, len(p.groups))
	for name, group := range p.groups {
		members := group.buildMembers(generation)
		index := make(map[string]struct{}, len(members))
		for _, tag := range members {
			index[tag] = struct{}{}
		}
		generation.groups[name] = &pawsGroupRoster{tags: members, index: index}
		if len(members) == 0 {
			return generation, errors.New("provider generation leaves a group empty")
		}
		for _, tag := range members {
			if generation.outbounds[tag] == nil {
				if _, found := p.manager.Outbound(tag); !found {
					return generation, errors.New("managed group references an unknown outbound")
				}
			}
		}
	}
	return generation, nil
}

func parsePawsProviderDefinition(raw json.RawMessage, kind string) (pawsProviderDefinition, error) {
	definition := pawsProviderDefinition{Health: pawsProviderHealth{Lazy: true}}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&definition); err != nil {
		return definition, errors.New("provider definition contains unsupported fields")
	}
	key := "proxy-providers"
	if kind == "rules" {
		key = "rule-providers"
	}
	declaration, err := json.Marshal(map[string]any{key: map[string]json.RawMessage{"provider": raw}})
	if err != nil {
		return definition, errors.New("provider definition encoding failed")
	}
	if _, err = compilePawsProvider(6, 0, 1, declaration); err != nil {
		return definition, err
	}
	switch definition.Type {
	case "http":
		if !strings.HasPrefix(definition.URL, "https://") && !strings.HasPrefix(definition.URL, "http://") {
			return definition, errors.New("provider URL must be HTTP or HTTPS")
		}
	case "file":
		if definition.Path == "" {
			return definition, errors.New("file provider has no path")
		}
	case "inline":
		if len(definition.Payload) == 0 {
			return definition, errors.New("inline provider has no payload")
		}
	default:
		return definition, errors.New("unsupported provider source type")
	}
	if definition.Filter != "" || definition.ExcludeFilter != "" || len(definition.Override) > 0 {
		return definition, errors.New("provider filter or override requires converter support")
	}
	if definition.SizeLimit > pawsProviderInputLimit {
		return definition, errors.New("provider size-limit exceeds compiler input budget")
	}
	if definition.Interval > 365*86400 || definition.Health.Interval > 365*86400 || definition.Health.Timeout > 120000 {
		return definition, errors.New("provider timing exceeds bounded runtime limits")
	}
	if kind == "rules" && definition.Behavior != "domain" && definition.Behavior != "ipcidr" && definition.Behavior != "classical" {
		return definition, errors.New("unsupported rule provider behavior")
	}
	return definition, nil
}
func (p *pawsSingRuntime) providerBody(ctx context.Context, definition pawsProviderDefinition, initial bool) ([]byte, error) {
	if definition.Type == "inline" {
		var body any
		if err := json.Unmarshal(definition.Payload, &body); err != nil {
			return nil, errors.New("invalid inline provider payload")
		}
		if _, array := body.([]any); !array {
			return nil, errors.New("inline payload must be a sequence")
		}
		key := "payload"
		if definition.Behavior == "" {
			key = "proxies"
		}
		return json.Marshal(map[string]any{key: body})
	}
	path := definition.Path
	if path != "" && !filepath.IsAbs(path) {
		path = filepath.Join(p.directory, path)
	}
	readCache := func() ([]byte, error) {
		file, err := os.Open(path)
		if err != nil {
			return nil, errors.New("provider cache is unavailable")
		}
		defer file.Close()
		data, err := io.ReadAll(io.LimitReader(file, pawsProviderInputLimit+1))
		if err != nil || len(data) > pawsProviderInputLimit {
			return nil, errors.New("provider file exceeds byte budget or cannot be read")
		}
		return data, nil
	}
	if definition.Type == "file" {
		return readCache()
	}
	transport := &http.Transport{Proxy: nil, DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
		tag := definition.Proxy
		if tag == "" || strings.EqualFold(tag, "DIRECT") {
			tag = "direct"
		}
		proxy, ok := p.manager.Outbound(tag)
		if !ok {
			return nil, errors.New("provider download detour is unavailable")
		}
		return proxy.DialContext(ctx, network, M.ParseSocksaddr(address))
	}}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 30 * time.Second, CheckRedirect: func(request *http.Request, history []*http.Request) error {
		if len(history) >= 5 {
			return errors.New("provider redirect limit")
		}
		if request.URL.Scheme != "http" && request.URL.Scheme != "https" {
			return errors.New("provider redirect scheme")
		}
		return nil
	}}
	request, err := http.NewRequestWithContext(ctx, "GET", definition.URL, nil)
	if err != nil {
		return nil, errors.New("invalid provider request")
	}
	for name, raw := range definition.Header {
		var values []string
		if err := json.Unmarshal(raw, &values); err != nil {
			var value string
			if err := json.Unmarshal(raw, &value); err != nil {
				return nil, errors.New("invalid provider header")
			}
			values = []string{value}
		}
		for _, value := range values {
			request.Header.Add(name, value)
		}
	}
	response, err := client.Do(request)
	if err != nil {
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		if initial && path != "" {
			return readCache()
		}
		return nil, errors.New("provider download failed")
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return nil, errors.New("provider download returned unsuccessful status")
	}
	limit := int64(pawsProviderInputLimit)
	if definition.SizeLimit > 0 && definition.SizeLimit < uint64(limit) {
		limit = int64(definition.SizeLimit)
	}
	body, err := io.ReadAll(io.LimitReader(response.Body, limit+1))
	if err != nil || int64(len(body)) > limit {
		return nil, errors.New("provider download exceeded byte budget or failed")
	}
	return body, nil
}
func (p *pawsSingRuntime) compileProvider(ctx context.Context, kind, name string, definition pawsProviderDefinition, body []byte) (*pawsProviderRecord, error) {
	kindID := uint32(1)
	if kind == "rules" {
		kindID = 2
	}
	behavior := uint32(0)
	switch definition.Behavior {
	case "domain":
		behavior = 1
	case "ipcidr":
		behavior = 2
	case "classical":
		behavior = 3
	case "":
		if kindID == 2 {
			return nil, errors.New("rule provider behavior is required")
		}
	default:
		return nil, errors.New("unknown provider behavior")
	}
	format := uint32(1)
	switch definition.Format {
	case "", "yaml":
	case "text":
		format = 2
	case "mrs":
		format = 3
	default:
		return nil, errors.New("unknown provider format")
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	compiled, err := compilePawsProvider(kindID, behavior, format, body)
	if err != nil {
		return nil, err
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	record := &pawsProviderRecord{definition: definition, updatedAt: time.Now().UTC().Format(time.RFC3339Nano), ruleCount: compiled.RuleCount}
	if kindID == 1 {
		record.options = make([]option.Outbound, 0, len(compiled.Outbounds))
		for _, raw := range compiled.Outbounds {
			outbound, err := SJ.UnmarshalExtendedContext[option.Outbound](p.ctx, raw)
			if err != nil {
				return nil, errors.New("converted provider outbound was rejected by native schema")
			}
			record.options = append(record.options, outbound)
			record.tags = append(record.tags, outbound.Tag)
		}
	} else {
		record.rules = make([]option.HeadlessRule, 0, len(compiled.Rules))
		expandedBytes := 0
		for _, raw := range compiled.Rules {
			raw, err = p.expandProviderGeo(raw)
			expandedBytes += len(raw)
			if expandedBytes > pawsProviderOutputLimit {
				return nil, errors.New("expanded provider rules exceed output budget")
			}
			if err != nil {
				return nil, err
			}
			value, err := SJ.UnmarshalExtendedContext[option.HeadlessRule](p.ctx, raw)
			if err != nil {
				return nil, errors.New("converted provider rule was rejected by native schema")
			}
			if _, err := rule.NewHeadlessRule(p.ctx, value); err != nil {
				return nil, errors.New("converted provider rule failed native compilation")
			}
			record.rules = append(record.rules, value)
		}
	}
	return record, nil
}
func (p *pawsSingRuntime) writeProviderCache(definition pawsProviderDefinition, body []byte) error {
	if definition.Type != "http" || definition.Path == "" {
		return nil
	}
	path := definition.Path
	if !filepath.IsAbs(path) {
		path = filepath.Join(p.directory, path)
	}
	if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		return errors.New("provider cache directory failed")
	}
	file, err := os.CreateTemp(filepath.Dir(path), ".paws-provider-*")
	if err != nil {
		return errors.New("provider cache staging failed")
	}
	temporary := file.Name()
	defer os.Remove(temporary)
	if err = file.Chmod(0600); err == nil {
		_, err = file.Write(body)
	}
	closeErr := file.Close()
	if err != nil || closeErr != nil {
		return errors.New("provider cache write failed")
	}
	if err = os.Rename(temporary, path); err != nil {
		return errors.New("provider cache commit failed")
	}
	return nil
}

// The operation read lease spans all synchronous native and filesystem tails.
// Cancelling its HTTP context cannot make Stop report a false completed drain.
func (p *pawsSingRuntime) refreshProvider(ctx context.Context, kind, name string) error {
	if !p.observations.active.Load() {
		return errors.New("runtime retired")
	}
	p.providerTransaction.Lock()
	defer p.providerTransaction.Unlock()
	if !p.observations.active.Load() {
		return errors.New("runtime retired")
	}
	var previous *pawsProviderRecord
	if kind == "proxies" {
		if generation := p.providers.current.Load(); generation != nil {
			previous = generation.providers[name]
		}
	} else if kind == "rules" {
		previous = p.ruleProviders[name]
	} else {
		return errors.New("invalid provider kind")
	}
	if previous == nil {
		return errors.New("provider not found")
	}
	operation, cancel := context.WithCancel(ctx)
	stop := context.AfterFunc(p.ctx, cancel)
	defer stop()
	defer cancel()
	body, err := p.providerBody(operation, previous.definition, false)
	if err != nil {
		return err
	}
	record, err := p.compileProvider(operation, kind, name, previous.definition, body)
	if err != nil {
		return err
	}
	if kind == "proxies" {
		current := p.providers.current.Load()
		records := make(map[string]*pawsProviderRecord, len(current.providers))
		for key, value := range current.providers {
			records[key] = value
		}
		records[name] = record
		generation, err := p.stageProviders(records)
		if err != nil {
			return err
		}
		committed := false
		defer func() {
			if !committed {
				closePawsGeneration(generation)
			}
		}()
		if err = operation.Err(); err != nil {
			return err
		}
		if err = p.writeProviderCache(previous.definition, body); err != nil {
			return err
		}
		if err = operation.Err(); err != nil {
			return err
		}
		if err = p.providers.publish(generation); err != nil {
			return err
		}
		committed = true
		// All native provider objects were replaced by this generation. Their
		// enabled health checks must observe the new objects, never old results.
		for provider, definition := range p.proxyDefinitions {
			if definition.Health.Enable {
				p.scheduleProviderHealth(provider)
			}
		}
	} else {
		set, found := p.router.RuleSet("paws-provider:" + name)
		if !found {
			return errors.New("native provider rule-set is not registered")
		}
		updater, ok := set.(interface {
			Update([]option.HeadlessRule) error
		})
		if !ok {
			return errors.New("native rule-set update is unavailable")
		}
		if err = operation.Err(); err != nil {
			return err
		}
		if err = p.writeProviderCache(previous.definition, body); err != nil {
			return err
		}
		if err = operation.Err(); err != nil {
			return err
		}
		if err = updater.Update(record.rules); err != nil {
			return err
		}
		p.ruleProviders[name] = record
		p.observations.providers.Add(1)
		p.observations.rules.Add(1)
	}
	return nil
}
