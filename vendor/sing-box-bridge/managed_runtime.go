package main

import (
	"context"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"runtime"
	"runtime/debug"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	D "github.com/miekg/dns"
	box "github.com/sagernet/sing-box"
	"github.com/sagernet/sing-box/adapter"
	O "github.com/sagernet/sing-box/adapter/outbound"
	"github.com/sagernet/sing-box/experimental/clashapi/trafficontrol"
	"github.com/sagernet/sing-box/include"
	"github.com/sagernet/sing-box/log"
	"github.com/sagernet/sing-box/option"
	"github.com/sagernet/sing-box/route/rule"
	M "github.com/sagernet/sing/common/metadata"
	"github.com/sagernet/sing/service"
)

type pawsDelay struct {
	delay uint32
	alive bool
}
type pawsSingMetadata struct {
	GeoDataPaths pawsGeoDataPaths `json:"geodataPaths"`
	pawsNetworkServices
	ProxyProviders map[string]json.RawMessage `json:"proxyProviders"`
	RuleProviders  map[string]json.RawMessage `json:"ruleProviders"`
	RuntimeRules   []string                   `json:"runtimeRules"`
	DNSConfig      pawsDnsConfig              `json:"dnsConfig"`
}
type pawsProviderHealthTask struct {
	generation *pawsOutboundGeneration // guarded by providerHealthMu
}
type pawsSingRuntime struct {
	providerHealthMu                     sync.Mutex
	providerHealthTasks                  map[string]*pawsProviderHealthTask
	providerProbeLocks                   sync.Map
	providerProbeSlots                   chan struct{}
	providerHealthEpoch                  time.Time
	dnsTransports                        []*pawsDNSTransport
	dnsServers                           []*D.Server
	dnsSockets                           []io.Closer
	benchmarkAuthenticated               bool
	geoReferences                        map[string]pawsGeoReference
	geoFiles                             map[uint32][]byte
	geoCompiled                          map[string]pawsCompiledProvider
	geoFileBytes, geoOutputBytes         int
	ctx                                  context.Context
	cancel                               context.CancelFunc
	lease                                sync.RWMutex
	observations                         *pawsObservations
	providers                            *pawsProviderRegistry
	groups                               map[string]*pawsManagedGroup
	groupOrder                           []string
	manager                              adapter.OutboundManager
	registry                             *O.Registry
	router                               adapter.Router
	logger                               log.ContextLogger
	staticTags                           map[string]struct{}
	staticProxyTags                      []string
	directory, secret, controllerAddress string
	metadata                             pawsSingMetadata
	providerTransaction                  sync.Mutex
	ruleProviders                        map[string]*pawsProviderRecord
	proxyDefinitions                     map[string]pawsProviderDefinition
	ruleDefinitions                      map[string]pawsProviderDefinition
	delays                               map[string]pawsDelay // guarded by providers.mu
	workerMu                             sync.Mutex
	workers                              sync.WaitGroup
	probePending                         sync.Map
	traffic                              *trafficontrol.Manager
	public                               *pawsPublicServer
	connectionMu                         sync.Mutex
	controllerConnections                map[net.Conn]struct{}
}

// Metadata is consumed before sing-box's strict decoder. It never appears in
// /configs; the independent public endpoint applies its own credential/redactor.
func newPawsSingRuntime(parent context.Context, raw []byte, directory string) (*pawsSingRuntime, []byte, error) {
	var document map[string]json.RawMessage
	if err := json.Unmarshal(raw, &document); err != nil {
		return nil, nil, errors.New("invalid native config")
	}
	var metadata pawsSingMetadata
	if err := json.Unmarshal(document["paws_runtime"], &metadata); err != nil {
		return nil, nil, errors.New("missing or invalid runtime metadata")
	}
	delete(document, "paws_runtime")
	var experimental struct {
		ClashAPI struct {
			Address string `json:"external_controller"`
			Secret  string `json:"secret"`
		} `json:"clash_api"`
	}
	if err := json.Unmarshal(document["experimental"], &experimental); err != nil {
		return nil, nil, errors.New("invalid private controller config")
	}
	address := experimental.ClashAPI.Address
	host, _, err := net.SplitHostPort(address)
	if err != nil || net.ParseIP(host) == nil || !net.ParseIP(host).IsLoopback() || experimental.ClashAPI.Secret == "" {
		return nil, nil, errors.New("private controller must be authenticated loopback")
	}
	ctx, cancel := context.WithCancel(parent)
	p := &pawsSingRuntime{ctx: ctx, cancel: cancel, observations: newPawsObservations(), groups: make(map[string]*pawsManagedGroup), directory: directory, metadata: metadata, secret: experimental.ClashAPI.Secret, controllerAddress: address, staticTags: make(map[string]struct{}), ruleProviders: make(map[string]*pawsProviderRecord), proxyDefinitions: make(map[string]pawsProviderDefinition), ruleDefinitions: make(map[string]pawsProviderDefinition), delays: make(map[string]pawsDelay), controllerConnections: make(map[net.Conn]struct{})}
	p.providerHealthTasks = make(map[string]*pawsProviderHealthTask)
	p.providerProbeSlots = make(chan struct{}, 8)
	p.providerHealthEpoch = time.Now()
	p.geoReferences = make(map[string]pawsGeoReference)
	p.geoFiles = make(map[uint32][]byte)
	p.geoCompiled = make(map[string]pawsCompiledProvider)
	p.providers = &pawsProviderRegistry{owner: p, retired: make(map[*pawsOutboundGeneration]bool)}
	p.providers.current.Store(&pawsOutboundGeneration{outbounds: make(map[string]adapter.Outbound), providers: make(map[string]*pawsProviderRecord)})
	p.observations.groups.Store(1)
	p.observations.providers.Store(1)
	p.observations.rules.Store(1)
	for name, raw := range metadata.ProxyProviders {
		definition, err := parsePawsProviderDefinition(raw, "proxies")
		if err != nil {
			cancel()
			return nil, nil, err
		}
		p.proxyDefinitions[name] = definition
	}
	for name, raw := range metadata.RuleProviders {
		definition, err := parsePawsProviderDefinition(raw, "rules")
		if err != nil {
			cancel()
			return nil, nil, err
		}
		p.ruleDefinitions[name] = definition
	}
	var inbounds []struct {
		Tag   string `json:"tag"`
		Users []struct {
			Username string `json:"username"`
			Password string `json:"password"`
		} `json:"users"`
		Skip []string `json:"paws_skip_auth_prefixes"`
	}
	if err := json.Unmarshal(document["inbounds"], &inbounds); err != nil {
		cancel()
		return nil, nil, errors.New("invalid inbound roster")
	}
	for _, inbound := range inbounds {
		if inbound.Tag == "sing-box-benchmark" && len(inbound.Users) == 1 && inbound.Users[0].Username == "paws-app" && inbound.Users[0].Password == p.secret && len(inbound.Skip) == 0 {
			p.benchmarkAuthenticated = true
		}
	}
	var outbounds []struct {
		Tag  string `json:"tag"`
		Type string `json:"type"`
	}
	if err := json.Unmarshal(document["outbounds"], &outbounds); err != nil {
		cancel()
		return nil, nil, errors.New("invalid outbound roster")
	}
	for _, outbound := range outbounds {
		p.staticTags[outbound.Tag] = struct{}{}
		if outbound.Type == "paws-group" {
			p.groupOrder = append(p.groupOrder, outbound.Tag)
		} else if outbound.Tag != "direct" && outbound.Tag != "block" {
			p.staticProxyTags = append(p.staticProxyTags, outbound.Tag)
		}
	}
	if err := p.prepareNativeDocument(document); err != nil {
		cancel()
		return nil, nil, err
	}
	native, err := json.Marshal(document)
	if err != nil {
		cancel()
		return nil, nil, err
	}
	return p, native, nil
}
func (p *pawsSingRuntime) nativeContext() context.Context {
	registry := include.OutboundRegistry()
	dnsRegistry := include.DNSTransportRegistry()
	registerPawsDNS(dnsRegistry, p)
	ctx := box.Context(p.ctx, include.InboundRegistry(), registry, include.EndpointRegistry(), dnsRegistry, include.ServiceRegistry())
	return p.bind(ctx, registry)
}
func (p *pawsSingRuntime) bind(ctx context.Context, registry *O.Registry) context.Context {
	p.ctx = ctx
	p.registry = registry
	O.Register[pawsGroupOptions](registry, "paws-group", newPawsManagedGroup(p))
	service.MustRegister[adapter.RuntimeManagement](ctx, p)
	service.MustRegister[adapter.RuntimeDNSObserver](ctx, p)
	return ctx
}
func (p *pawsSingRuntime) initialize(instance *box.Box) error {
	p.manager = instance.Outbound()
	p.router = instance.Router()
	p.logger = instance.Logger()
	source, ok := p.manager.(interface{ SetDynamicSource(O.DynamicOutboundSource) })
	if !ok {
		return errors.New("native dynamic provider registry is unavailable")
	}
	source.SetDynamicSource(p.providers)
	if server := service.FromContext[adapter.ClashServer](p.ctx); server != nil {
		if source, ok := server.(interface{ TrafficManager() *trafficontrol.Manager }); ok {
			p.traffic = source.TrafficManager()
			p.traffic.SetLifecycleObserver(p.connectionEvent)
		}
	}
	if err := p.initializeGeo(); err != nil {
		return err
	}
	for _, transport := range p.dnsTransports {
		if err := transport.prepare(); err != nil {
			return err
		}
	}
	records := make(map[string]*pawsProviderRecord, len(p.proxyDefinitions))
	initialBodies := make(map[string][]byte, len(p.proxyDefinitions))
	names := make([]string, 0, len(p.proxyDefinitions))
	for name := range p.proxyDefinitions {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		definition := p.proxyDefinitions[name]
		body, err := p.providerBody(p.ctx, definition, true)
		if err != nil {
			return err
		}
		record, err := p.compileProvider(p.ctx, "proxies", name, definition, body)
		if err != nil {
			return err
		}
		records[name] = record
		initialBodies[name] = body
	}
	generation, err := p.stageProviders(records)
	if err != nil {
		return err
	}
	for _, name := range names {
		if err = p.writeProviderCache(p.proxyDefinitions[name], initialBodies[name]); err != nil {
			closePawsGeneration(generation)
			return err
		}
	}
	if err = p.providers.publish(generation); err != nil {
		closePawsGeneration(generation)
		return err
	}
	for name, definition := range p.ruleDefinitions {
		body, err := p.providerBody(p.ctx, definition, true)
		if err != nil {
			return err
		}
		record, err := p.compileProvider(p.ctx, "rules", name, definition, body)
		if err != nil {
			return err
		}
		set, found := p.router.RuleSet("paws-provider:" + name)
		if !found {
			return errors.New("initial rule provider has no native rule-set")
		}
		updater, ok := set.(interface {
			Update([]option.HeadlessRule) error
		})
		if !ok {
			return errors.New("native rule provider update unavailable")
		}
		if err = updater.Update(record.rules); err != nil {
			return err
		}
		if err = p.writeProviderCache(definition, body); err != nil {
			return err
		}
		p.ruleProviders[name] = record
	}
	return nil
}
func (p *pawsSingRuntime) launch(work func()) bool {
	p.workerMu.Lock()
	defer p.workerMu.Unlock()
	if !p.observations.active.Load() {
		return false
	}
	p.workers.Add(1)
	go func() { defer p.workers.Done(); work() }()
	return true
}
func (p *pawsSingRuntime) started() error {
	if err := p.startDNSListener(); err != nil {
		return err
	}
	public, err := startPawsPublicServer(p.metadata.pawsNetworkServices, p.controllerAddress, p.secret)
	if err != nil {
		return err
	}
	p.public = public
	for _, group := range p.groups {
		if group.options.Kind != "url-test" && group.options.Kind != "fallback" {
			continue
		}
		if !group.options.Lazy {
			p.scheduleProbe(group)
		}
		if group.options.Interval == 0 {
			continue
		}
		group := group
		p.launch(func() {
			ticker := time.NewTicker(time.Duration(group.options.Interval) * time.Second)
			defer ticker.Stop()
			for {
				select {
				case <-p.ctx.Done():
					return
				case <-ticker.C:
					if !group.options.Lazy || group.touched.Load() {
						p.scheduleProbe(group)
					}
				}
			}
		})
	}
	for name, definition := range p.proxyDefinitions {
		p.startProviderTimer("proxies", name, definition)
		p.startProviderHealth(name, definition)
	}
	for name, definition := range p.ruleDefinitions {
		p.startProviderTimer("rules", name, definition)
	}
	return nil
}
func (p *pawsSingRuntime) startProviderTimer(kind, name string, definition pawsProviderDefinition) {
	if definition.Type != "http" || definition.Interval == 0 {
		return
	}
	p.launch(func() {
		ticker := time.NewTicker(time.Duration(definition.Interval) * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-p.ctx.Done():
				return
			case <-ticker.C:
				p.lease.RLock()
				if p.observations.active.Load() {
					if err := p.refreshProvider(p.ctx, kind, name); err != nil {
						_ = p.observations.log("warn", "provider refresh failed", time.Now().UTC().Format(time.RFC3339Nano))
					}
				}
				p.lease.RUnlock()
			}
		}
	})
}
func (p *pawsSingRuntime) scheduleProbe(group *pawsManagedGroup) {
	if _, loaded := p.probePending.LoadOrStore(group.Tag(), true); loaded {
		return
	}
	if !p.launch(func() {
		defer p.probePending.Delete(group.Tag())
		p.lease.RLock()
		defer p.lease.RUnlock()
		if p.observations.active.Load() {
			_ = p.probeGroup(p.ctx, group)
		}
	}) {
		p.probePending.Delete(group.Tag())
	}
}
func (p *pawsSingRuntime) close() error {
	p.workerMu.Lock()
	if !p.observations.active.Swap(false) {
		p.workerMu.Unlock()
		return nil
	}
	p.cancel()
	p.workerMu.Unlock()
	for _, socket := range p.dnsSockets {
		_ = socket.Close()
	}
	for _, server := range p.dnsServers {
		ctx, cancel := context.WithTimeout(context.Background(), 6*time.Second)
		_ = server.ShutdownContext(ctx)
		cancel()
	}
	p.connectionMu.Lock()
	connections := make([]net.Conn, 0, len(p.controllerConnections))
	for connection := range p.controllerConnections {
		connections = append(connections, connection)
	}
	p.connectionMu.Unlock()
	for _, connection := range connections {
		_ = connection.Close()
	}
	if p.public != nil {
		p.public.close()
	}
	p.lease.Lock()
	p.lease.Unlock()
	p.workers.Wait()
	for _, group := range p.groups {
		_ = group.Close()
	}
	p.providers.close()
	return nil
}
func (p *pawsSingRuntime) ConnectionState(connection net.Conn, state http.ConnState) {
	p.connectionMu.Lock()
	defer p.connectionMu.Unlock()
	if state == http.StateClosed {
		delete(p.controllerConnections, connection)
	} else if state == http.StateNew {
		p.controllerConnections[connection] = struct{}{}
	}
}
func (p *pawsSingRuntime) DNSQuery(name string, recordType uint16, cached bool) {
	p.observations.dnsQuery(strings.TrimSuffix(name, "."), D.TypeToString[recordType], cached)
}
func (p *pawsSingRuntime) WrapHandler(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		credential := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")
		if r.URL.Query().Get("token") != "" && credential == "" {
			credential = r.URL.Query().Get("token")
		}
		if subtle.ConstantTimeCompare([]byte(credential), []byte(p.secret)) != 1 {
			http.Error(w, "Unauthorized", http.StatusUnauthorized)
			return
		}
		if !p.observations.active.Load() {
			http.Error(w, "engine session stopped", http.StatusServiceUnavailable)
			return
		}
		p.lease.RLock()
		defer p.lease.RUnlock()
		if !p.observations.active.Load() {
			http.Error(w, "engine session stopped", http.StatusServiceUnavailable)
			return
		}
		ctx, cancel := context.WithCancel(r.Context())
		stop := context.AfterFunc(p.ctx, cancel)
		defer stop()
		defer cancel()
		r = r.WithContext(ctx)
		if r.Method == http.MethodPost && r.URL.Path == "/cache/fakeip/flush" {
			store := p.fakeIPStore()
			if store == nil {
				http.Error(w, "fake-IP is not configured", http.StatusConflict)
				return
			}
			if err := store.Reset(); err != nil {
				http.Error(w, "native fake-IP reset failed", http.StatusInternalServerError)
				return
			}
			service.FromContext[adapter.DNSRouter](p.ctx).ClearCache()
			w.WriteHeader(http.StatusNoContent)
			return
		}
		if strings.HasPrefix(r.URL.Path, "/paws/") || strings.HasPrefix(r.URL.Path, "/providers/") {
			p.handle(w, r)
			return
		}
		if r.Method == http.MethodDelete && strings.HasPrefix(r.URL.Path, "/proxies/") {
			group := p.groups[strings.TrimPrefix(r.URL.Path, "/proxies/")]
			if group == nil {
				http.NotFound(w, r)
				return
			}
			err := group.Unfix()
			if err != nil {
				http.Error(w, err.Error(), http.StatusBadRequest)
			} else {
				w.WriteHeader(http.StatusNoContent)
			}
			return
		}
		if strings.HasPrefix(r.URL.Path, "/proxies/") && strings.HasSuffix(r.URL.Path, "/delay") {
			tag := strings.TrimSuffix(strings.TrimPrefix(r.URL.Path, "/proxies/"), "/delay")
			timeout, err := strconv.ParseUint(r.URL.Query().Get("timeout"), 10, 32)
			if err != nil || timeout == 0 || timeout > 120000 {
				http.Error(w, "invalid timeout", 400)
				return
			}
			link, expected := r.URL.Query().Get("url"), r.URL.Query().Get("expected")
			delay, err := p.probeTag(ctx, tag, &link, &expected, time.Duration(timeout)*time.Millisecond)
			p.respond(w, map[string]uint32{"delay": delay}, err)
			return
		}
		next.ServeHTTP(w, r)
	})
}
func (p *pawsSingRuntime) respond(w http.ResponseWriter, value any, err error) {
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadGateway)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(value)
}
func (p *pawsSingRuntime) handle(w http.ResponseWriter, r *http.Request) {
	path := strings.TrimPrefix(r.URL.Path, "/paws/observations/")
	if strings.HasPrefix(r.URL.Path, "/providers/") {
		p.handleProvider(w, r)
		return
	}
	if strings.HasPrefix(r.URL.Path, "/paws/proxies/") && strings.HasSuffix(r.URL.Path, "/echo") && r.Method == http.MethodPost {
		value, err := p.echo(r, strings.TrimSuffix(strings.TrimPrefix(r.URL.Path, "/paws/proxies/"), "/echo"))
		p.respond(w, value, err)
		return
	}
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", 405)
		return
	}
	var value any
	var err error
	switch {
	case strings.HasPrefix(path, "fakeip/"):
		store := p.fakeIPStore()
		if store == nil {
			err = errors.New("fake-IP is not configured")
			break
		}
		address, parseErr := netip.ParseAddr(strings.TrimPrefix(path, "fakeip/"))
		if parseErr != nil {
			err = errors.New("invalid fake-IP lookup address")
			break
		}
		var domain *string
		if actual, found := store.Lookup(address); found {
			domain = &actual
		}
		value = struct {
			Domain *string `json:"domain"`
		}{domain}
	case path == "revisions":
		value = p.observations.revisions()
	case path == "groups":
		value = p.groupSummaries()
	case strings.HasPrefix(path, "proxy-groups/"):
		value, err = p.group(strings.TrimPrefix(path, "proxy-groups/"))
	case strings.HasPrefix(path, "providers/"):
		parts := strings.SplitN(strings.TrimPrefix(path, "providers/"), "/", 2)
		if len(parts) != 2 {
			err = errors.New("missing provider identity")
		} else {
			value, err = p.providerState(parts[0], parts[1])
		}
	case path == "rules":
		rules := make([]pawsRuntimeRule, 0)
		for _, native := range p.router.Rules() {
			target := native.Action().String()
			if action, ok := native.Action().(*rule.RuleActionRoute); ok {
				target = action.Outbound
			}
			rules = append(rules, pawsRuntimeRule{native.Type(), native.String(), target})
		}
		value = rules
	case path == "mode":
		if server := service.FromContext[adapter.ClashServer](p.ctx); server != nil {
			value = server.Mode()
		} else {
			err = errors.New("native mode service unavailable")
		}
	case path == "dns":
		value = p.observations.dnsSnapshot()
	case path == "dns-config":
		value = p.metadata.DNSConfig
	case path == "applied-services":
		value, err = p.appliedServices()
	case path == "memory":
		var memory runtime.MemStats
		runtime.ReadMemStats(&memory)
		value = pawsMemory{Scope: "engineHeap", InUse: memory.HeapInuse}
	case path == "connection-summary":
		value, err = p.connectionSummary()
	case path == "logs" || path == "connection-events":
		after, parseErr := strconv.ParseUint(r.URL.Query().Get("after"), 10, 64)
		limit, limitErr := strconv.ParseUint(r.URL.Query().Get("limit"), 10, 16)
		if parseErr != nil || limitErr != nil {
			err = errors.New("invalid journal cursor")
			break
		}
		journal := p.observations.logs
		if path == "connection-events" {
			journal = p.observations.connections
		}
		value, err = journal.page(after, uint16(limit))
	default:
		http.NotFound(w, r)
		return
	}
	p.respond(w, value, err)
}
func (p *pawsSingRuntime) delay(tag string) *uint32 {
	p.providers.mu.Lock()
	defer p.providers.mu.Unlock()
	record, ok := p.delays[tag]
	if !ok || !record.alive {
		return nil
	}
	value := record.delay
	return &value
}
func (p *pawsSingRuntime) groupSummaries() []pawsGroupSummary {
	result := make([]pawsGroupSummary, 0, len(p.groupOrder))
	for _, name := range p.groupOrder {
		group := p.groups[name]
		selected := group.Now()
		result = append(result, pawsGroupSummary{name, group.options.Kind, &selected, group.Fixed(), len(group.members(p.providers.current.Load())), p.delay(selected)})
	}
	return result
}
func (p *pawsSingRuntime) group(name string) (pawsProxyGroup, error) {
	group := p.groups[name]
	if group == nil {
		return pawsProxyGroup{}, errors.New("group not found")
	}
	selected := group.Now()
	result := pawsProxyGroup{Name: name, GroupType: group.options.Kind, Selected: &selected, Fixed: group.Fixed(), Proxies: []pawsProxyItem{}}
	for _, tag := range group.All() {
		proxy, ok := p.manager.Outbound(tag)
		if !ok {
			return pawsProxyGroup{}, errors.New("group member unavailable")
		}
		result.Proxies = append(result.Proxies, pawsProxyItem{tag, proxy.Type(), p.delay(tag), tag == selected})
	}
	return result, nil
}
func (p *pawsSingRuntime) providerState(kind, name string) (pawsProviderState, error) {
	p.providerTransaction.Lock()
	defer p.providerTransaction.Unlock()
	record := p.ruleProviders[name]
	if kind == "proxies" {
		generation := p.providers.current.Load()
		if generation != nil {
			record = generation.providers[name]
		}
	} else if kind != "rules" {
		return pawsProviderState{}, errors.New("invalid provider kind")
	}
	if record == nil {
		return pawsProviderState{}, errors.New("provider not found")
	}
	vehicle := record.definition.Type
	updated := record.updatedAt
	result := pawsProviderState{Kind: map[string]string{"proxies": "proxy", "rules": "rule"}[kind], Name: name, UpdatedAt: &updated, Vehicle: &vehicle}
	if kind == "proxies" {
		members := make([]pawsProviderProxy, 0, len(record.tags))
		for _, tag := range record.tags {
			proxy, ok := p.manager.Outbound(tag)
			if !ok {
				return result, errors.New("provider member unavailable")
			}
			delay := p.delay(tag)
			var alive *bool
			p.providers.mu.Lock()
			if measured, ok := p.delays[tag]; ok {
				value := measured.alive
				alive = &value
			}
			p.providers.mu.Unlock()
			members = append(members, pawsProviderProxy{tag, proxy.Type(), delay, alive})
		}
		result.Members = &members
	} else {
		count := record.ruleCount
		result.RuleCount = &count
	}
	return result, nil
}

// Standard Clash provider routes are distinct from the neutral observation DTO.
func (p *pawsSingRuntime) handleProvider(w http.ResponseWriter, r *http.Request) {
	parts := strings.Split(strings.Trim(strings.TrimPrefix(r.URL.EscapedPath(), "/providers/"), "/"), "/")
	for index, part := range parts {
		decoded, err := url.PathUnescape(part)
		if err != nil {
			http.Error(w, "invalid provider path", 400)
			return
		}
		parts[index] = decoded
	}
	if len(parts) == 0 || (parts[0] != "proxies" && parts[0] != "rules") {
		http.NotFound(w, r)
		return
	}
	kind := parts[0]
	if r.Method == http.MethodGet && len(parts) == 1 {
		names := make([]string, 0)
		definitions := p.proxyDefinitions
		if kind == "rules" {
			definitions = p.ruleDefinitions
		}
		for name := range definitions {
			names = append(names, name)
		}
		sort.Strings(names)
		providers := make(map[string]any, len(names))
		for _, name := range names {
			value, err := p.clashProvider(kind, name)
			if err != nil {
				p.respond(w, nil, err)
				return
			}
			providers[name] = value
		}
		p.respond(w, map[string]any{"providers": providers}, nil)
		return
	}
	if len(parts) < 2 {
		http.NotFound(w, r)
		return
	}
	name := parts[1]
	if r.Method == http.MethodPut && len(parts) == 2 {
		if err := p.refreshProvider(r.Context(), kind, name); err != nil {
			p.respond(w, nil, err)
		} else {
			w.WriteHeader(http.StatusNoContent)
		}
		return
	}
	if r.Method == http.MethodGet && ((len(parts) == 3 && parts[2] == "healthcheck") || (len(parts) == 4 && parts[3] == "healthcheck")) {
		if kind != "proxies" {
			http.Error(w, "not a proxy provider", 400)
			return
		}
		single := ""
		if len(parts) == 4 {
			single = parts[2]
		}
		timeout := 5 * time.Second
		if value := r.URL.Query().Get("timeout"); value != "" {
			milliseconds, err := strconv.ParseUint(value, 10, 32)
			if err != nil || milliseconds == 0 || milliseconds > 120000 {
				http.Error(w, "invalid timeout", 400)
				return
			}
			timeout = time.Duration(milliseconds) * time.Millisecond
		}
		link, expected := r.URL.Query().Get("url"), r.URL.Query().Get("expected")
		delay, err := p.probeProvider(r.Context(), name, single, &link, &expected, timeout)
		if err != nil {
			p.respond(w, nil, err)
		} else if single != "" {
			p.respond(w, map[string]uint32{"delay": delay}, nil)
		} else {
			w.WriteHeader(http.StatusNoContent)
		}
		return
	}
	if r.Method == http.MethodGet && len(parts) == 2 {
		value, err := p.clashProvider(kind, name)
		p.respond(w, value, err)
		return
	}
	http.Error(w, "method not allowed", 405)
}
func (p *pawsSingRuntime) clashProvider(kind, name string) (any, error) {
	state, err := p.providerState(kind, name)
	if err != nil {
		return nil, err
	}
	value := map[string]any{"name": name, "vehicleType": state.Vehicle, "updatedAt": state.UpdatedAt}
	if state.Members != nil {
		proxies := make([]any, 0, len(*state.Members))
		for _, member := range *state.Members {
			history := make([]any, 0, 1)
			if member.Delay != nil {
				history = append(history, map[string]any{"delay": *member.Delay})
			}
			proxies = append(proxies, map[string]any{"name": member.Name, "type": member.ProxyType, "alive": member.Alive, "history": history})
		}
		value["proxies"] = proxies
	} else {
		value["ruleCount"] = state.RuleCount
	}
	return value, nil
}

// A retained generation protects the actual outbound object throughout await.
// Publication checks the identical generation, not a newly looked-up tag.
func (p *pawsSingRuntime) probeTag(ctx context.Context, tag string, link, expected *string, timeout time.Duration) (uint32, error) {
	generation := p.providers.acquire()
	defer p.providers.release(generation)
	if generation == nil {
		return 0, errors.New("provider generation unavailable")
	}
	proxy := generation.outbounds[tag]
	if proxy == nil {
		proxy, _ = p.manager.Outbound(tag)
	}
	delay, err := probePawsOutbound(ctx, proxy, link, expected, timeout)
	p.providers.mu.Lock()
	defer p.providers.mu.Unlock()
	if !p.observations.active.Load() || p.providers.current.Load() != generation {
		return 0, errors.New("runtime generation changed during probe")
	}
	p.delays[tag] = pawsDelay{delay: delay, alive: err == nil}
	p.observations.groups.Add(1)
	return delay, err
}
func (p *pawsSingRuntime) probeProvider(ctx context.Context, name, single string, link, expected *string, timeout time.Duration) (uint32, error) {
	return p.probeProviderGeneration(ctx, name, single, link, expected, timeout, nil)
}
func (p *pawsSingRuntime) probeProviderGeneration(ctx context.Context, name, single string, link, expected *string, timeout time.Duration, expectedGeneration *pawsOutboundGeneration) (uint32, error) {
	generation := p.providers.acquire()
	defer p.providers.release(generation)
	if generation == nil || generation.providers[name] == nil {
		return 0, errors.New("provider not found")
	}
	if expectedGeneration != nil && generation != expectedGeneration {
		return 0, errors.New("provider generation changed before scheduled health check")
	}
	record := generation.providers[name]
	if single != "" && !containsPawsTag(record.tags, single) {
		return 0, errors.New("provider member not found")
	}
	leave, err := p.enterProviderProbe(ctx, name)
	if err != nil {
		return 0, err
	}
	defer leave()
	if !p.observations.active.Load() || p.providers.current.Load() != generation {
		return 0, errors.New("provider generation changed before health check")
	}
	if *link == "" && record.definition.Health.URL != "" {
		configured := record.definition.Health.URL
		link = &configured
	}
	if *expected == "" && record.definition.Health.ExpectedStatus != "" {
		configured := record.definition.Health.ExpectedStatus
		expected = &configured
	}
	type result struct {
		tag   string
		delay pawsDelay
	}
	results := make(map[string]pawsDelay, len(record.tags))
	jobs := make(chan string)
	measured := make(chan result, 8)
	var workers sync.WaitGroup
	workerCount := min(8, len(record.tags))
	for index := 0; index < workerCount; index++ {
		workers.Add(1)
		go func() {
			defer workers.Done()
			for tag := range jobs {
				select {
				case p.providerProbeSlots <- struct{}{}:
				case <-ctx.Done():
					return
				case <-p.ctx.Done():
					return
				}
				delay, err := probePawsOutbound(ctx, generation.outbounds[tag], link, expected, timeout)
				<-p.providerProbeSlots
				select {
				case measured <- result{tag, pawsDelay{delay: delay, alive: err == nil}}:
				case <-ctx.Done():
					return
				}
			}
		}()
	}
	workers.Add(1)
	go func() {
		defer workers.Done()
		defer close(jobs)
		for _, tag := range record.tags {
			if single != "" && single != tag {
				continue
			}
			select {
			case jobs <- tag:
			case <-ctx.Done():
				return
			case <-p.ctx.Done():
				return
			}
		}
	}()
	go func() { workers.Wait(); close(measured) }()
	for result := range measured {
		results[result.tag] = result.delay
	}
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	if err := p.ctx.Err(); err != nil {
		return 0, err
	}
	// Keep a deterministic response for explicit single/all checks, independent
	// of the bounded workers' completion order.
	var last uint32
	var failed error
	for _, tag := range record.tags {
		if result, ok := results[tag]; ok {
			last = result.delay
			if !result.alive {
				failed = errors.New("provider member health check failed")
			}
		}
	}
	p.providers.mu.Lock()
	defer p.providers.mu.Unlock()
	if !p.observations.active.Load() || p.providers.current.Load() != generation {
		return 0, errors.New("provider generation changed during health check")
	}
	for tag, delay := range results {
		p.delays[tag] = delay
	}
	if len(results) > 0 {
		p.observations.groups.Add(1)
		p.observations.providers.Add(1)
	}
	return last, failed
}
func (p *pawsSingRuntime) connectionEvent(event trafficontrol.ConnectionEvent) {
	if event.Metadata == nil || event.Type == trafficontrol.ConnectionEventUpdate {
		return
	}
	_ = p.observations.connection(pawsSingConnection(event.Metadata), event.Type == trafficontrol.ConnectionEventClosed)
}
func pawsSingConnection(info *trafficontrol.TrackerMetadata) pawsConnection {
	domain := info.Metadata.Domain
	if domain == "" {
		domain = info.Metadata.Destination.Fqdn
	}
	source := ""
	if domain != "" {
		source = "native"
	}
	route := "final"
	if info.Rule != nil {
		route = info.Rule.String()
	}
	return pawsConnection{DomainSource: source, ID: info.ID.String(), Host: domain, Domain: domain, DestinationIP: info.Metadata.Destination.Addr.String(), DestinationPort: info.Metadata.Destination.Port, Network: info.Metadata.Network, Rule: route, RulePayload: "", Proxy: info.Outbound, Chains: append([]string{}, info.Chain...), StartedAt: info.CreatedAt.UTC().Format(time.RFC3339Nano), Upload: uint64(max(0, info.Upload.Load())), Download: uint64(max(0, info.Download.Load()))}
}
func (p *pawsSingRuntime) connectionSummary() (pawsConnectionSummary, error) {
	if p.traffic == nil {
		return pawsConnectionSummary{}, errors.New("native connection source unavailable")
	}
	count := 0
	upload, download := p.traffic.Total()
	recent := make([]*trafficontrol.TrackerMetadata, 0, 5)
	p.traffic.RangeConnections(func(info *trafficontrol.TrackerMetadata) {
		count++
		at := sort.Search(len(recent), func(index int) bool { return recent[index].CreatedAt.Before(info.CreatedAt) })
		if at < 5 {
			recent = append(recent, nil)
			copy(recent[at+1:], recent[at:])
			recent[at] = info
			if len(recent) > 5 {
				recent = recent[:5]
			}
		}
	})
	result := pawsConnectionSummary{Count: count, Upload: uint64(max(0, upload)), Download: uint64(max(0, download)), Recent: []pawsConnection{}}
	for _, info := range recent {
		result.Recent = append(result.Recent, pawsSingConnection(info))
	}
	p.observations.mu.Lock()
	if p.observations.latestRuleNode != nil {
		value := *p.observations.latestRuleNode
		result.LatestRuleNode = &value
	}
	p.observations.mu.Unlock()
	return result, nil
}
func (p *pawsSingRuntime) echo(r *http.Request, tag string) (any, error) {
	var request struct {
		URL     string `json:"url"`
		Payload string `json:"payload"`
		Timeout uint32 `json:"timeout"`
	}
	if err := json.NewDecoder(io.LimitReader(r.Body, 64*1024)).Decode(&request); err != nil {
		return nil, errors.New("invalid echo request")
	}
	proxy, ok := p.manager.Outbound(tag)
	if !ok {
		return nil, errors.New("proxy not found")
	}
	target, err := url.Parse(request.URL)
	if err != nil || target.Hostname() == "" {
		return nil, errors.New("invalid echo target")
	}
	port := target.Port()
	if port == "" {
		port = "443"
		if target.Scheme == "http" {
			port = "80"
		}
	}
	ctx, cancel := context.WithTimeout(r.Context(), time.Duration(min(max(request.Timeout, 1), 30000))*time.Millisecond)
	defer cancel()
	connection, err := proxy.DialContext(ctx, "tcp", M.ParseSocksaddr(net.JoinHostPort(target.Hostname(), port)))
	if err != nil {
		return nil, err
	}
	defer connection.Close()
	if deadline, ok := ctx.Deadline(); ok {
		_ = connection.SetDeadline(deadline)
	}
	if _, err = io.Copy(connection, strings.NewReader(request.Payload)); err != nil {
		return nil, err
	}
	reply := make([]byte, len(request.Payload))
	if _, err = io.ReadFull(connection, reply); err != nil {
		return nil, err
	}
	if string(reply) != request.Payload {
		return nil, errors.New("echo payload mismatch")
	}
	return map[string]string{"payload": string(reply)}, nil
}

func (p *pawsSingRuntime) appliedServices() (pawsAppliedServices, error) {
	if !p.observations.active.Load() || !p.benchmarkAuthenticated {
		return pawsAppliedServices{}, errors.New("private proxy ownership is not established")
	}
	manager := service.FromContext[adapter.InboundManager](p.ctx)
	if manager == nil {
		return pawsAppliedServices{}, errors.New("native inbound registry unavailable")
	}
	address := func(tag string) (*net.TCPAddr, error) {
		inbound, ok := manager.Get(tag)
		if !ok {
			return nil, errors.New("required native listener is missing")
		}
		listener, ok := inbound.(interface{ RuntimeListenerAddress() net.Addr })
		if !ok {
			return nil, errors.New("required listener has no ownership receipt")
		}
		actual, ok := listener.RuntimeListenerAddress().(*net.TCPAddr)
		if !ok || actual.Port <= 0 {
			return nil, errors.New("required native listener is not bound")
		}
		return actual, nil
	}
	benchmark, err := address("sing-box-benchmark")
	if err != nil {
		return pawsAppliedServices{}, err
	}
	if !benchmark.IP.IsLoopback() {
		return pawsAppliedServices{}, errors.New("private proxy is not loopback")
	}
	result := pawsAppliedServices{BenchmarkPort: uint16(benchmark.Port)}
	if p.metadata.Ports.MixedEnabled {
		mixed, err := address("mixed-in")
		if err != nil {
			return result, err
		}
		if mixed.Port != int(p.metadata.Ports.MixedPort) {
			return result, errors.New("public mixed listener port mismatch")
		}
		result.MixedReady = true
	}
	if p.metadata.Ports.ControllerEnabled {
		if p.public == nil || !p.public.active.Load() {
			return result, errors.New("public controller not bound")
		}
		actual := p.public.address
		result.PublicControllerAddress = &actual
	}
	return result, nil
}

func trimPawsSingRuntimeMemory(owner *pawsSingRuntime) error {
	if owner == nil || !owner.observations.active.Load() {
		return errors.New("sing-box is not running")
	}
	runtime.GC()
	debug.FreeOSMemory()
	return nil
}

func (p *pawsSingRuntime) fakeIPStore() adapter.FakeIPStore {
	manager := service.FromContext[adapter.DNSTransportManager](p.ctx)
	if manager == nil {
		return nil
	}
	transport := manager.FakeIP()
	if transport == nil {
		return nil
	}
	return transport.Store()
}
