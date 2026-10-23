package main

import (
	"context"
	"errors"
	"io"
	"net"
	"net/netip"
	"sort"
	"strings"
	"sync"
	"time"

	D "github.com/miekg/dns"
	"github.com/sagernet/sing-box/adapter"
	B "github.com/sagernet/sing-box/dns"
	"github.com/sagernet/sing-box/log"
	"github.com/sagernet/sing-box/option"
	"github.com/sagernet/sing-box/route/rule"
	M "github.com/sagernet/sing/common/metadata"
	"github.com/sagernet/sing/service"
)

type pawsDNSPolicyOptions struct {
	Matcher string   `json:"matcher"`
	Servers []string `json:"servers"`
}
type pawsDNSOptions struct {
	Servers          []string               `json:"servers"`
	Fallbacks        []string               `json:"fallbacks"`
	Policies         []pawsDNSPolicyOptions `json:"policies"`
	Resolver         string                 `json:"resolver"`
	ProxyResolver    string                 `json:"proxyResolver"`
	Hosts            map[string][]string    `json:"hosts"`
	UseHosts         bool                   `json:"useHosts"`
	FakeIP           string                 `json:"fakeIP"`
	FakeIPFilter     []string               `json:"fakeIPFilter"`
	FakeIPFilterMode string                 `json:"fakeIPFilterMode"`
}
type pawsDNSPolicy struct {
	rules   []adapter.HeadlessRule
	servers []adapter.DNSTransport
}
type pawsDNSTransport struct {
	B.TransportAdapter
	owner             *pawsSingRuntime
	ctx               context.Context
	options           pawsDNSOptions
	manager           adapter.DNSTransportManager
	primary, fallback []adapter.DNSTransport
	fakeIP            adapter.FakeIPTransport
	fakeIPFilter      []adapter.HeadlessRule
	realTransport     *pawsRealDNSTransport
	access            sync.RWMutex
	policies          []pawsDNSPolicy
	prepared          bool
}

func registerPawsDNS(registry *B.TransportRegistry, owner *pawsSingRuntime) {
	B.RegisterTransport[pawsDNSOptions](registry, "paws-dns", func(ctx context.Context, logger log.ContextLogger, tag string, options pawsDNSOptions) (adapter.DNSTransport, error) {
		if len(options.Servers) == 0 {
			return nil, errors.New("DNS dispatcher requires a primary server")
		}
		dependencies := append([]string{}, options.Servers...)
		dependencies = append(dependencies, options.Fallbacks...)
		if options.FakeIP != "" {
			dependencies = append(dependencies, options.FakeIP)
		}
		for _, policy := range options.Policies {
			dependencies = append(dependencies, policy.Servers...)
		}
		seen := make(map[string]bool)
		unique := make([]string, 0, len(dependencies))
		for _, dependency := range dependencies {
			if dependency == tag {
				return nil, errors.New("recursive DNS dispatcher")
			}
			if !seen[dependency] {
				seen[dependency] = true
				unique = append(unique, dependency)
			}
		}
		transport := &pawsDNSTransport{TransportAdapter: B.NewTransportAdapter("paws-dns", tag, unique), owner: owner, ctx: ctx, options: options, manager: service.FromContext[adapter.DNSTransportManager](ctx)}
		transport.realTransport = &pawsRealDNSTransport{dispatcher: transport}
		owner.dnsTransports = append(owner.dnsTransports, transport)
		return transport, nil
	})
}
func (t *pawsDNSTransport) Start(stage adapter.StartStage) error {
	if stage != adapter.StartStateStart {
		return nil
	}
	var err error
	t.primary, err = t.resolve(t.options.Servers)
	if err != nil {
		return err
	}
	t.fallback, err = t.resolve(t.options.Fallbacks)
	if err != nil {
		return err
	}
	return nil
}
func (t *pawsDNSTransport) Close() error { return nil }
func (t *pawsDNSTransport) Reset()       {} // Child transports and the native router own their caches/sockets.
func (t *pawsDNSTransport) resolve(tags []string) ([]adapter.DNSTransport, error) {
	result := make([]adapter.DNSTransport, 0, len(tags))
	for _, tag := range tags {
		transport, ok := t.manager.Transport(tag)
		if !ok {
			return nil, errors.New("DNS dispatcher references an unknown server")
		}
		result = append(result, transport)
	}
	return result, nil
}
func (t *pawsDNSTransport) prepare() error {
	policies := make([]pawsDNSPolicy, 0, len(t.options.Policies))
	for _, options := range t.options.Policies {
		servers, err := t.resolve(options.Servers)
		if err != nil || len(servers) == 0 {
			return errors.New("DNS policy has no native resolver")
		}
		rules, err := t.compileMatchers(expandPawsDNSMatchers(options.Matcher))
		if err != nil {
			return err
		}
		policies = append(policies, pawsDNSPolicy{rules: rules, servers: servers})
	}
	filters := make([]adapter.HeadlessRule, 0, len(t.options.FakeIPFilter))
	if t.options.FakeIP != "" {
		if t.options.FakeIPFilterMode != "" && t.options.FakeIPFilterMode != "blacklist" && t.options.FakeIPFilterMode != "whitelist" {
			return errors.New("invalid fake-IP filter mode")
		}
		transport, ok := t.manager.Transport(t.options.FakeIP)
		if !ok {
			return errors.New("fake-IP transport is missing")
		}
		t.fakeIP, ok = transport.(adapter.FakeIPTransport)
		if !ok {
			return errors.New("fake-IP transport has no native store")
		}
		for _, filter := range t.options.FakeIPFilter {
			compiled, err := t.compileMatchers(expandPawsDNSMatchers(filter))
			if err != nil {
				return err
			}
			filters = append(filters, compiled...)
		}
	}

	t.access.Lock()
	t.policies = policies
	t.fakeIPFilter = filters
	t.prepared = true
	t.access.Unlock()
	return nil
}
func (t *pawsDNSTransport) compileMatchers(matchers []string) ([]adapter.HeadlessRule, error) {
	rules := make([]adapter.HeadlessRule, 0, len(matchers))
	for _, matcher := range matchers {
		matcher = strings.TrimSpace(matcher)
		if matcher == "" {
			return nil, errors.New("empty DNS policy matcher")
		}
		if strings.HasPrefix(strings.ToLower(matcher), "geosite:") {
			set, ok := t.owner.router.RuleSet("paws-geosite:" + matcher[len("geosite:"):])
			if !ok {
				return nil, errors.New("DNS policy geosite has not been compiled")
			}
			rules = append(rules, set)
			continue
		}
		if strings.HasPrefix(strings.ToLower(matcher), "rule-set:") {
			set, ok := t.owner.router.RuleSet("paws-provider:" + matcher[len("rule-set:"):])
			if !ok {
				return nil, errors.New("DNS policy rule provider is unknown")
			}
			rules = append(rules, set)
			continue
		}
		expression := "DOMAIN," + matcher
		if matcher == "*" {
			expression = "DOMAIN-REGEX,.+"
		} else if strings.HasPrefix(matcher, "+.") {
			expression = "DOMAIN-SUFFIX," + strings.TrimPrefix(matcher, "+.")
		} else if strings.HasPrefix(matcher, "*.") {
			expression = "DOMAIN-REGEX,^[^.]+(?:\\.[^.]+)*\\." + escapePawsDomain(strings.TrimPrefix(matcher, "*.")) + "$"
		} else if strings.ContainsAny(matcher, "*+?") {
			return nil, errors.New("DNS policy wildcard is not representable")
		}
		compiled, err := compilePawsProvider(2, 3, 2, []byte(expression))
		if err != nil {
			return nil, err
		}
		for _, raw := range compiled.Rules {
			var options option.HeadlessRule
			if err = options.UnmarshalJSON(raw); err != nil {
				return nil, errors.New("DNS policy native schema is invalid")
			}
			native, err := rule.NewHeadlessRule(t.ctx, options)
			if err != nil {
				return nil, errors.New("DNS policy native matcher failed")
			}
			rules = append(rules, native)
		}
	}
	return rules, nil
}
func escapePawsDomain(value string) string {
	var result strings.Builder
	for _, char := range value {
		if strings.ContainsRune(`\.^$|()[]{}*+?`, char) {
			result.WriteByte('\\')
		}
		result.WriteRune(char)
	}
	return result.String()
}
func (t *pawsDNSTransport) Exchange(ctx context.Context, message *D.Msg) (*D.Msg, error) {
	return t.exchange(ctx, message, 0, true)
}
func (t *pawsDNSTransport) exchange(ctx context.Context, message *D.Msg, depth int, allowFakeIP bool) (*D.Msg, error) {
	if !t.owner.observations.active.Load() {
		return nil, errors.New("DNS runtime retired")
	}
	if len(message.Question) != 1 {
		return nil, errors.New("DNS dispatcher expects one question")
	}
	if depth > 16 {
		return nil, errors.New("DNS hosts alias cycle")
	}
	question := message.Question[0]
	domain := strings.TrimSuffix(strings.ToLower(question.Name), ".")
	if t.options.UseHosts {
		values, found := t.options.Hosts[domain]
		if !found {
			keys := make([]string, 0)
			for key := range t.options.Hosts {
				if strings.HasPrefix(key, "+.") && (domain == key[2:] || strings.HasSuffix(domain, "."+key[2:])) {
					keys = append(keys, key)
				}
			}
			sort.Slice(keys, func(i, j int) bool { return len(keys[i]) > len(keys[j]) })
			if len(keys) > 0 {
				values = t.options.Hosts[keys[0]]
				found = true
			}
		}
		if found {
			response := new(D.Msg)
			response.SetReply(message)
			for _, value := range values {
				address, err := netip.ParseAddr(value)
				if err == nil {
					header := D.RR_Header{Name: question.Name, Class: D.ClassINET, Ttl: 60}
					if question.Qtype == D.TypeA && address.Is4() {
						header.Rrtype = D.TypeA
						response.Answer = append(response.Answer, &D.A{Hdr: header, A: net.IP(address.AsSlice())})
					} else if question.Qtype == D.TypeAAAA && address.Is6() {
						header.Rrtype = D.TypeAAAA
						response.Answer = append(response.Answer, &D.AAAA{Hdr: header, AAAA: net.IP(address.AsSlice())})
					}
					continue
				}
				if _, valid := D.IsDomainName(value); !valid {
					return nil, errors.New("DNS hosts value is neither an address nor valid alias")
				}
				alias := message.Copy()
				alias.Question[0].Name = D.Fqdn(value)
				answer, err := t.exchange(ctx, alias, depth+1, allowFakeIP)
				if err != nil {
					return nil, err
				}
				response.Answer = append(response.Answer, &D.CNAME{Hdr: D.RR_Header{Name: question.Name, Rrtype: D.TypeCNAME, Class: D.ClassINET, Ttl: 60}, Target: D.Fqdn(value)})
				response.Answer = append(response.Answer, answer.Answer...)
			}
			return response, nil
		}
	}
	primary := t.primary
	t.access.RLock()
	prepared := t.prepared
	policies := t.policies
	filters := t.fakeIPFilter
	t.access.RUnlock()
	if !prepared && len(t.options.Policies) > 0 {
		return nil, errors.New("DNS policy is not initialized")
	}
	metadata := &adapter.InboundContext{Domain: domain, QueryType: question.Qtype}
	if allowFakeIP && t.fakeIP != nil && (question.Qtype == D.TypeA || question.Qtype == D.TypeAAAA) {
		matched := false
		for _, filter := range filters {
			if filter.Match(metadata) {
				matched = true
				break
			}
		}
		useFake := !matched
		if t.options.FakeIPFilterMode == "whitelist" {
			useFake = matched
		}
		if useFake {
			return t.fakeIP.Exchange(ctx, message)
		}
	}
	for _, policy := range policies {
		matched := false
		for _, rule := range policy.rules {
			if rule.Match(metadata) {
				matched = true
				break
			}
		}
		if matched {
			primary = policy.servers
			break
		}
	}
	var last error
	for _, server := range append(append([]adapter.DNSTransport(nil), primary...), t.fallback...) {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		response, err := server.Exchange(ctx, message.Copy())
		if err == nil && response != nil && (response.Rcode == D.RcodeSuccess || response.Rcode == D.RcodeNameError) {
			return response, nil
		}
		if err != nil {
			last = err
		} else {
			last = errors.New("DNS server returned unsuccessful status")
		}
	}
	if last == nil {
		last = errors.New("DNS dispatcher has no available server")
	}
	return nil, last
}

// Both sockets are bound before startup succeeds. Requests use the actual
// native DNS router (including cache/strategy/observer), never a second resolver.
func (p *pawsSingRuntime) startDNSListener() error {
	if p.metadata.DNSConfig.Listen == nil || *p.metadata.DNSConfig.Listen == "" {
		return nil
	}
	address := *p.metadata.DNSConfig.Listen
	if _, _, err := net.SplitHostPort(address); err != nil {
		return errors.New("invalid DNS listener address")
	}
	router := service.FromContext[adapter.DNSRouter](p.ctx)
	if router == nil {
		return errors.New("native DNS router unavailable")
	}
	udp, err := net.ListenPacket("udp", address)
	if err != nil {
		return errors.New("DNS UDP listener bind failed")
	}
	tcp, err := net.Listen("tcp", address)
	if err != nil {
		_ = udp.Close()
		return errors.New("DNS TCP listener bind failed")
	}
	capacity := make(chan struct{}, 64)
	handler := D.HandlerFunc(func(w D.ResponseWriter, request *D.Msg) {
		select {
		case capacity <- struct{}{}:
			defer func() { <-capacity }()
		default:
			response := new(D.Msg)
			response.SetRcode(request, D.RcodeServerFailure)
			_ = w.WriteMsg(response)
			return
		}
		p.lease.RLock()
		defer p.lease.RUnlock()
		if !p.observations.active.Load() {
			return
		}
		ctx, cancel := context.WithTimeout(p.ctx, 5*time.Second)
		defer cancel()
		metadata := &adapter.InboundContext{Inbound: "paws-dns-listener", Network: "udp"}
		if _, ok := w.RemoteAddr().(*net.TCPAddr); ok {
			metadata.Network = "tcp"
		}
		metadata.Source = M.ParseSocksaddr(w.RemoteAddr().String())
		response, err := router.Exchange(adapter.WithContext(ctx, metadata), request, adapter.DNSQueryOptions{})
		if err != nil {
			response = new(D.Msg)
			response.SetRcode(request, D.RcodeServerFailure)
		}
		if response != nil {
			_ = w.WriteMsg(response)
		}
	})
	p.dnsSockets = []io.Closer{udp, tcp}
	p.dnsServers = []*D.Server{{PacketConn: udp, Handler: handler, ReadTimeout: 5 * time.Second, WriteTimeout: 5 * time.Second}, {Listener: tcp, Handler: handler, ReadTimeout: 5 * time.Second, WriteTimeout: 5 * time.Second}}
	for _, server := range p.dnsServers {
		server := server
		p.launch(func() {
			if err := server.ActivateAndServe(); err != nil && p.observations.active.Load() {
				_ = p.observations.log("error", "DNS listener stopped unexpectedly", time.Now().UTC().Format(time.RFC3339Nano))
			}
		})
	}
	return nil
}

func (t *pawsDNSTransport) UsesFakeIP() bool                    { return t.options.FakeIP != "" }
func (t *pawsDNSTransport) WithoutFakeIP() adapter.DNSTransport { return t.realTransport }

// This view deliberately does not embed the parent: it must not promote its
// fake-IP policy interface back into the native router. It owns no sockets.
type pawsRealDNSTransport struct{ dispatcher *pawsDNSTransport }

func (t *pawsRealDNSTransport) Type() string                   { return "paws-dns-real" }
func (t *pawsRealDNSTransport) Tag() string                    { return t.dispatcher.Tag() + ":real" }
func (t *pawsRealDNSTransport) Dependencies() []string         { return t.dispatcher.Dependencies() }
func (t *pawsRealDNSTransport) Start(adapter.StartStage) error { return nil }
func (t *pawsRealDNSTransport) Close() error                   { return nil }
func (t *pawsRealDNSTransport) Reset()                         {}
func (t *pawsRealDNSTransport) Exchange(ctx context.Context, message *D.Msg) (*D.Msg, error) {
	return t.dispatcher.exchange(ctx, message, 0, false)
}
