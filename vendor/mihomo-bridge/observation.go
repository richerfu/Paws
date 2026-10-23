package main

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/url"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/metacubex/chi"
	H "github.com/metacubex/http"
	"github.com/metacubex/mihomo/common/yaml"
	"github.com/metacubex/mihomo/config"
	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/dns"
	"github.com/metacubex/mihomo/hub/route"
	"github.com/metacubex/mihomo/listener"
	"github.com/metacubex/mihomo/tunnel"
	"github.com/metacubex/mihomo/tunnel/statistic"
	D "github.com/miekg/dns"
)

// Each authenticated router retains this owner. Looking up a process-global
// "current" owner from an old keepalive request would bypass session fencing.
type pawsMihomoRuntime struct {
	lease        sync.RWMutex
	ctx          context.Context
	cancel       context.CancelFunc
	observations *pawsObservations
	config       *config.Config
	public       *pawsPublicServer
	network      pawsNetworkServices
}

var pawsMihomoCurrent atomic.Pointer[pawsMihomoRuntime]

// Diagnostic retention owns only the bounded journal, never config/providers.
var pawsMihomoLastLogs atomic.Pointer[pawsJournal]
var pawsMihomoRegistry = struct {
	sync.RWMutex
	sessions map[string]*pawsMihomoRuntime
}{sessions: make(map[string]*pawsMihomoRuntime)}

func preparePawsMihomo(cfg *config.Config, raw []byte) (*pawsMihomoRuntime, error) {
	var metadata struct {
		Services pawsNetworkServices `yaml:"paws-runtime"`
	}
	if err := yaml.Unmarshal(raw, &metadata); err != nil {
		return nil, err
	}
	if cfg.Controller.Secret == "" {
		return nil, errors.New("private controller requires a session credential")
	}
	ctx, cancel := context.WithCancel(context.Background())
	owner := &pawsMihomoRuntime{observations: newPawsObservations(), config: cfg, ctx: ctx, cancel: cancel, network: metadata.Services}
	public, err := startPawsPublicServer(metadata.Services, cfg.Controller.ExternalController, cfg.Controller.Secret)
	if err != nil {
		cancel()
		return nil, err
	}
	owner.public = public
	owner.observations.groups.Store(1)
	owner.observations.providers.Store(1)
	owner.observations.rules.Store(1)
	pawsMihomoRegistry.Lock()
	pawsMihomoRegistry.sessions[cfg.Controller.Secret] = owner
	pawsMihomoRegistry.Unlock()
	pawsMihomoCurrent.Store(owner)
	pawsMihomoLastLogs.Store(owner.observations.logs)
	owner.bindRuntimeObservers()
	return owner, nil
}
func (p *pawsMihomoRuntime) bindProxy(proxy C.Proxy) {
	if bound, ok := proxy.(interface{ SetRuntimeObserver(func()) }); ok {
		bound.SetRuntimeObserver(func() {
			if p.observations.active.Load() {
				p.observations.groups.Add(1)
				p.observations.providers.Add(1)
			}
		})
	}
}
func (p *pawsMihomoRuntime) bindRuntimeObservers() {
	for _, proxy := range p.config.Proxies {
		p.bindProxy(proxy)
	}
	for _, provider := range p.config.Providers {
		if bound, ok := provider.(interface{ SetRuntimeObserver(func([]C.Proxy)) }); ok {
			bound.SetRuntimeObserver(func(proxies []C.Proxy) {
				if !p.observations.active.Load() {
					return
				}
				for _, proxy := range proxies {
					p.bindProxy(proxy)
				}
				p.observations.providers.Add(1)
				p.observations.groups.Add(1)
			})
		}
	}
	for _, provider := range p.config.RuleProviders {
		if bound, ok := provider.(interface{ SetUpdateObserver(func()) }); ok {
			bound.SetUpdateObserver(func() {
				if p.observations.active.Load() {
					p.observations.providers.Add(1)
					p.observations.rules.Add(1)
				}
			})
		}
	}
}

func (p *pawsMihomoRuntime) close() error {
	if p == nil {
		return nil
	}
	p.observations.active.Store(false)
	p.cancel()
	for _, provider := range p.config.Providers {
		if cancel, ok := provider.(interface{ CancelPending() }); ok {
			cancel.CancelPending()
		}
	}
	for _, provider := range p.config.RuleProviders {
		if cancel, ok := provider.(interface{ CancelPending() }); ok {
			cancel.CancelPending()
		}
	}
	p.lease.Lock()
	defer p.lease.Unlock()
	p.observations.active.Store(false)
	if err := p.public.close(); err != nil {
		return err
	}
	pawsMihomoRegistry.Lock()
	if pawsMihomoRegistry.sessions[p.config.Controller.Secret] == p {
		delete(pawsMihomoRegistry.sessions, p.config.Controller.Secret)
	}
	pawsMihomoRegistry.Unlock()
	pawsMihomoCurrent.CompareAndSwap(p, nil)
	return nil
}
func init() {
	route.RegisterSessionScope(func(secret string) func(H.Handler) H.Handler {
		pawsMihomoRegistry.RLock()
		owner := pawsMihomoRegistry.sessions[secret]
		pawsMihomoRegistry.RUnlock()
		return func(next H.Handler) H.Handler {
			return H.HandlerFunc(func(w H.ResponseWriter, r *H.Request) {
				if owner == nil {
					H.Error(w, "engine session unavailable", H.StatusServiceUnavailable)
					return
				}
				ctx, cancel := context.WithCancel(r.Context())
				stop := context.AfterFunc(owner.ctx, cancel)
				defer stop()
				defer cancel()
				owner.lease.RLock()
				defer owner.lease.RUnlock()
				if !owner.observations.active.Load() {
					H.Error(w, "engine session stopped", H.StatusServiceUnavailable)
					return
				}
				interrupt := context.AfterFunc(ctx, func() { _ = H.NewResponseController(w).SetWriteDeadline(time.Now()) })
				defer interrupt()
				next.ServeHTTP(w, r.WithContext(ctx))
			})
		}
	})
	dns.DefaultQueryObserver = func() dns.QueryNotify {
		owner := pawsMihomoCurrent.Load()
		return func(record dns.QueryRecord) {
			if owner != nil {
				owner.observations.dnsQuery(strings.TrimSuffix(record.Question.Name, "."), D.Type(record.Question.Qtype).String(), record.Cached)
			}
			if dns.DefaultQueryNotify != nil {
				dns.DefaultQueryNotify(record)
			}
		}
	}
	route.RegisterWithSecret(func(router chi.Router, secret string) {
		pawsMihomoRegistry.RLock()
		owner := pawsMihomoRegistry.sessions[secret]
		pawsMihomoRegistry.RUnlock()
		router.HandleFunc("/paws/*", func(w H.ResponseWriter, r *H.Request) {
			if owner == nil {
				H.Error(w, "engine session unavailable", H.StatusServiceUnavailable)
				return
			}
			owner.handle(w, r)
		})
	})
	statistic.DefaultConnectionObserver = func(connection statistic.Tracker) func(statistic.Tracker) {
		owner := pawsMihomoCurrent.Load()
		if owner == nil {
			return nil
		}
		if err := owner.observations.connection(pawsMihomoConnection(connection.Info()), false); err != nil {
			logError("connection journal: %v", err)
		}
		return func(closed statistic.Tracker) {
			if err := owner.observations.connection(pawsMihomoConnection(closed.Info()), true); err != nil {
				logError("connection journal: %v", err)
			}
		}
	}
}
func (p *pawsMihomoRuntime) handle(w H.ResponseWriter, r *H.Request) {
	if !p.observations.active.Load() {
		H.Error(w, "engine session stopped", H.StatusServiceUnavailable)
		return
	}
	name := strings.TrimPrefix(r.URL.Path, "/paws/observations/")
	if strings.HasPrefix(r.URL.Path, "/paws/proxies/") && strings.HasSuffix(r.URL.Path, "/echo") && r.Method == H.MethodPost {
		proxy := strings.TrimSuffix(strings.TrimPrefix(r.URL.Path, "/paws/proxies/"), "/echo")
		value, err := p.echo(r, proxy)
		p.respond(w, value, err)
		return
	}
	if r.Method != H.MethodGet {
		H.Error(w, "method not allowed", H.StatusMethodNotAllowed)
		return
	}
	var value any
	var err error
	switch {
	case name == "revisions":
		value = p.observations.revisions()
	case name == "groups":
		value = p.groups()
	case strings.HasPrefix(name, "proxy-groups/"):
		value, err = p.group(strings.TrimPrefix(name, "proxy-groups/"))
	case strings.HasPrefix(name, "providers/"):
		parts := strings.SplitN(strings.TrimPrefix(name, "providers/"), "/", 2)
		if len(parts) != 2 {
			err = errors.New("missing provider identity")
		} else {
			value, err = p.provider(parts[0], parts[1])
		}
	case name == "rules":
		rules := make([]pawsRuntimeRule, 0, len(p.config.Rules))
		for _, rule := range p.config.Rules {
			rules = append(rules, pawsRuntimeRule{rule.RuleType().String(), rule.Payload(), rule.Adapter()})
		}
		value = rules
	case name == "mode":
		value = tunnel.Mode().String()
	case name == "dns":
		value = p.observations.dnsSnapshot()
	case name == "dns-config":
		value = p.dnsConfig()
	case name == "applied-services":
		value, err = p.appliedServices()
	case name == "memory":
		var memory runtime.MemStats
		runtime.ReadMemStats(&memory)
		value = pawsMemory{Scope: "engineHeap", InUse: memory.HeapInuse}
	case name == "connection-summary":
		value = p.connectionSummary()
	case name == "connection-events" || name == "logs":
		after, parseErr := strconv.ParseUint(r.URL.Query().Get("after"), 10, 64)
		limit, limitErr := strconv.ParseUint(r.URL.Query().Get("limit"), 10, 16)
		if parseErr != nil || limitErr != nil {
			err = errors.New("invalid journal page")
			break
		}
		journal := p.observations.connections
		if name == "logs" {
			journal = p.observations.logs
		}
		value, err = journal.page(after, uint16(limit))
	default:
		H.NotFound(w, r)
		return
	}
	p.respond(w, value, err)
}
func (p *pawsMihomoRuntime) respond(w H.ResponseWriter, value any, err error) {
	if err != nil {
		H.Error(w, err.Error(), H.StatusBadGateway)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(value); err != nil {
		logError("runtime observation encode: %v", err)
	}
}

type pawsMihomoGroup interface {
	Now() string
	GetProxies(bool) []C.Proxy
}

func pawsMihomoFixed(proxy C.Proxy) *string {
	if fixed, ok := proxy.Adapter().(interface{ Fixed() string }); ok {
		value := fixed.Fixed()
		return &value
	}
	return nil
}
func pawsMihomoDelay(proxy C.Proxy) *uint32 {
	history := proxy.DelayHistory()
	if len(history) == 0 || history[len(history)-1].Delay == 0 {
		return nil
	}
	value := uint32(history[len(history)-1].Delay)
	return &value
}
func (p *pawsMihomoRuntime) groups() []pawsGroupSummary {
	groups := make([]pawsGroupSummary, 0)
	for _, proxy := range tunnel.Proxies() {
		group, ok := proxy.Adapter().(pawsMihomoGroup)
		if !ok {
			continue
		}
		selected := group.Now()
		members := group.GetProxies(false)
		var delay *uint32
		for _, member := range members {
			if member.Name() == selected {
				delay = pawsMihomoDelay(member)
				break
			}
		}
		groups = append(groups, pawsGroupSummary{proxy.Name(), proxy.Type().String(), &selected, pawsMihomoFixed(proxy), len(members), delay})
	}
	sort.Slice(groups, func(i, j int) bool { return groups[i].Name < groups[j].Name })
	return groups
}
func (p *pawsMihomoRuntime) group(name string) (pawsProxyGroup, error) {
	proxy, ok := tunnel.Proxies()[name]
	if !ok {
		return pawsProxyGroup{}, errors.New("group not found")
	}
	group, ok := proxy.Adapter().(pawsMihomoGroup)
	if !ok {
		return pawsProxyGroup{}, errors.New("proxy is not a group")
	}
	selected := group.Now()
	result := pawsProxyGroup{Name: name, GroupType: proxy.Type().String(), Selected: &selected, Fixed: pawsMihomoFixed(proxy), Proxies: []pawsProxyItem{}}
	for _, member := range group.GetProxies(false) {
		result.Proxies = append(result.Proxies, pawsProxyItem{member.Name(), member.Type().String(), pawsMihomoDelay(member), member.Name() == selected})
	}
	return result, nil
}
func (p *pawsMihomoRuntime) provider(kind, name string) (pawsProviderState, error) {
	state := pawsProviderState{Name: name}
	switch kind {
	case "proxies":
		provider, ok := tunnel.ProvidersSnapshot()[name]
		if !ok {
			return state, errors.New("proxy provider not found")
		}
		state.Kind = "proxy"
		vehicle := provider.VehicleType().String()
		state.Vehicle = &vehicle
		members := make([]pawsProviderProxy, 0, provider.Count())
		for _, proxy := range provider.Proxies() {
			var alive *bool
			var delay *uint32
			var history []C.DelayHistory
			if source, ok := proxy.(interface{ DelayHistoryForTestUrl(string) []C.DelayHistory }); ok {
				history = source.DelayHistoryForTestUrl(provider.HealthCheckURL())
			} else {
				history = proxy.ExtraDelayHistories()[provider.HealthCheckURL()].History
			}
			if len(history) > 0 {
				measured := proxy.AliveForTestUrl(provider.HealthCheckURL())
				alive = &measured
				if last := history[len(history)-1].Delay; last > 0 {
					value := uint32(last)
					delay = &value
				}
			}
			members = append(members, pawsProviderProxy{proxy.Name(), proxy.Type().String(), delay, alive})
		}
		state.Members = &members
		if updated, ok := provider.(interface{ UpdatedAt() time.Time }); ok {
			stamp := updated.UpdatedAt().Format(time.RFC3339Nano)
			state.UpdatedAt = &stamp
		}
	case "rules":
		provider, ok := tunnel.RuleProvidersSnapshot()[name]
		if !ok {
			return state, errors.New("rule provider not found")
		}
		state.Kind = "rule"
		count := provider.Count()
		state.RuleCount = &count
		vehicle := provider.VehicleType().String()
		state.Vehicle = &vehicle
	default:
		return state, errors.New("unknown provider kind")
	}
	return state, nil
}
func (p *pawsMihomoRuntime) dnsConfig() pawsDnsConfig {
	dns := p.config.DNS
	view := pawsDnsConfig{Model: "mihomo-" + dns.EnhancedMode.String(), Upstreams: []string{}, Fallbacks: []string{}, Policy: make(map[string][]string)}
	if dns.Enable && dns.Listen != "" {
		listen := dns.Listen
		view.Listen = &listen
	}
	server := func(net, addr, detour string) string {
		if net != "" {
			addr = net + "://" + addr
		}
		if detour != "" {
			addr += "#" + detour
		}
		return addr
	}
	for _, upstream := range dns.NameServer {
		view.Upstreams = append(view.Upstreams, server(upstream.Net, upstream.Addr, upstream.ProxyName))
	}
	for _, upstream := range dns.Fallback {
		view.Fallbacks = append(view.Fallbacks, server(upstream.Net, upstream.Addr, upstream.ProxyName))
	}
	for _, policy := range dns.NameServerPolicy {
		for _, upstream := range policy.NameServers {
			view.Policy[policy.Domain] = append(view.Policy[policy.Domain], server(upstream.Net, upstream.Addr, upstream.ProxyName))
		}
	}
	return view
}
func pawsMihomoConnection(info *statistic.TrackerInfo) pawsConnection {
	metadata := info.Metadata
	domain := metadata.Host
	source := "engine DNS/configuration"
	if metadata.SniffHost != "" {
		domain = metadata.SniffHost
		source = "TLS SNI / HTTP Host"
	} else if domain == "" {
		source = "unavailable"
	}
	ip := ""
	if metadata.DstIP.IsValid() {
		ip = metadata.DstIP.String()
	}
	host := domain
	if host == "" {
		host = ip
	}
	chains := append([]string{}, info.Chain...)
	proxy := "DIRECT"
	if len(chains) > 0 {
		proxy = chains[0]
	}
	return pawsConnection{DomainSource: source, ID: info.UUID.String(), Host: host, Domain: domain, DestinationIP: ip, DestinationPort: metadata.DstPort, Network: metadata.NetWork.String(), Rule: info.Rule, RulePayload: info.RulePayload, Proxy: proxy, Chains: chains, StartedAt: info.Start.Format(time.RFC3339Nano), Upload: uint64(max(0, info.UploadTotal.Load())), Download: uint64(max(0, info.DownloadTotal.Load()))}
}
func (p *pawsMihomoRuntime) connectionSummary() pawsConnectionSummary {
	summary := pawsConnectionSummary{Recent: []pawsConnection{}}
	p.observations.mu.Lock()
	summary.LatestRuleNode = p.observations.latestRuleNode
	p.observations.mu.Unlock()
	statistic.DefaultManager.Range(func(connection statistic.Tracker) bool {
		info := connection.Info()
		summary.Count++
		summary.Upload += uint64(max(0, info.UploadTotal.Load()))
		summary.Download += uint64(max(0, info.DownloadTotal.Load()))
		if len(summary.Recent) < 5 || info.Start.Format(time.RFC3339Nano) > summary.Recent[len(summary.Recent)-1].StartedAt {
			summary.Recent = append(summary.Recent, pawsMihomoConnection(info))
			sort.Slice(summary.Recent, func(i, j int) bool { return summary.Recent[i].StartedAt > summary.Recent[j].StartedAt })
			if len(summary.Recent) > 5 {
				summary.Recent = summary.Recent[:5]
			}
		}
		return true
	})
	return summary
}
func (p *pawsMihomoRuntime) echo(r *H.Request, name string) (any, error) {
	var request struct {
		URL     string `json:"url"`
		Payload string `json:"payload"`
		Timeout uint32 `json:"timeout"`
	}
	if err := json.NewDecoder(io.LimitReader(r.Body, 64*1024)).Decode(&request); err != nil {
		return nil, err
	}
	proxy, ok := tunnel.Proxies()[name]
	if !ok {
		return nil, errors.New("proxy not found")
	}
	target, err := url.Parse(request.URL)
	if err != nil || target.Hostname() == "" {
		return nil, errors.New("invalid echo target")
	}
	port := uint16(443)
	if target.Scheme == "http" {
		port = 80
	}
	if target.Port() != "" {
		value, err := strconv.ParseUint(target.Port(), 10, 16)
		if err != nil {
			return nil, err
		}
		port = uint16(value)
	}
	ctx, cancel := context.WithTimeout(r.Context(), time.Duration(min(max(request.Timeout, 1), 30000))*time.Millisecond)
	defer cancel()
	metadata := &C.Metadata{NetWork: C.TCP, Type: C.INNER, Host: target.Hostname(), DstPort: port}
	connection, err := proxy.DialContext(ctx, metadata)
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
	response := make([]byte, len(request.Payload))
	if _, err = io.ReadFull(connection, response); err != nil {
		return nil, err
	}
	if string(response) != request.Payload {
		return nil, errors.New("echo payload mismatch")
	}
	return map[string]string{"payload": string(response)}, nil
}

func validatePawsRequiredListeners(cfg *config.Config) error {
	if cfg == nil {
		return errors.New("native config unavailable")
	}
	for _, expected := range cfg.Listeners {
		if _, ok := listener.PawsAppliedInboundAddress(expected); !ok {
			return errors.New("required native inbound listener failed to bind")
		}
	}
	if !listener.PawsMixedApplied(cfg.General.MixedPort, cfg.General.BindAddress, cfg.General.AllowLan) {
		return errors.New("native public mixed listener failed to bind")
	}
	return nil
}
func (p *pawsMihomoRuntime) appliedServices() (pawsAppliedServices, error) {
	if !p.observations.active.Load() {
		return pawsAppliedServices{}, errors.New("runtime retired")
	}
	if err := validatePawsRequiredListeners(p.config); err != nil {
		return pawsAppliedServices{}, err
	}
	expected := p.config.Listeners["tinypaws-mihomo-benchmark"]
	address, ok := listener.PawsAppliedInboundAddress(expected)
	if !ok {
		return pawsAppliedServices{}, errors.New("private proxy listener is missing")
	}
	actual, err := net.ResolveTCPAddr("tcp", address)
	if err != nil || !actual.IP.IsLoopback() || actual.Port <= 0 {
		return pawsAppliedServices{}, errors.New("private proxy ownership is invalid")
	}
	result := pawsAppliedServices{BenchmarkPort: uint16(actual.Port), MixedReady: p.network.Ports.MixedEnabled}
	if p.network.Ports.ControllerEnabled {
		if p.public == nil || !p.public.active.Load() {
			return result, errors.New("public controller not bound")
		}
		address := p.public.address
		result.PublicControllerAddress = &address
	}
	return result, nil
}
