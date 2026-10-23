package main

import (
	"context"
	"errors"
	"hash/fnv"
	"net"
	"net/http"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/sagernet/sing-box/adapter"
	O "github.com/sagernet/sing-box/adapter/outbound"
	"github.com/sagernet/sing-box/common/interrupt"
	"github.com/sagernet/sing-box/log"
	M "github.com/sagernet/sing/common/metadata"
)

type pawsGroupOptions struct {
	Kind                string   `json:"kind"`
	Outbounds           []string `json:"outbounds"`
	Use                 []string `json:"use"`
	Default             *string  `json:"default"`
	URL                 *string  `json:"url"`
	Interval            uint64   `json:"interval"`
	Timeout             uint64   `json:"timeout"`
	Tolerance           uint32   `json:"tolerance"`
	Lazy                bool     `json:"lazy"`
	Strategy            *string  `json:"strategy"`
	IncludeAllProxies   bool     `json:"include_all_proxies"`
	IncludeAllProviders bool     `json:"include_all_providers"`
	Filter              *string  `json:"filter"`
	ExcludeFilter       *string  `json:"exclude_filter"`
	ExcludeType         *string  `json:"exclude_type"`
	ExpectedStatus      *string  `json:"expected_status"`
	DisableUDP          bool     `json:"disable_udp"`
	Interrupt           bool     `json:"interrupt_exist_connections"`
}

// The group retains tag identities, never a stale map of provider objects.
// Every dial/query resolves members through the same immutable native generation.
type pawsManagedGroup struct {
	interrupt *interrupt.Group
	O.Adapter
	owner            *pawsSingRuntime
	options          pawsGroupOptions
	filter, exclude  *regexp.Regexp
	mu               sync.Mutex
	fixed, automatic string
	round            atomic.Uint64
	touched          atomic.Bool
}

func newPawsManagedGroup(owner *pawsSingRuntime) O.ConstructorFunc[pawsGroupOptions] {
	return func(ctx context.Context, router adapter.Router, logger log.ContextLogger, tag string, options pawsGroupOptions) (adapter.Outbound, error) {
		switch options.Kind {
		case "select", "url-test", "fallback", "load-balance":
		default:
			return nil, errors.New("unsupported managed group strategy")
		}
		if options.Timeout == 0 || options.Timeout > 120000 || options.Interval > 86400*365 {
			return nil, errors.New("invalid managed group timing")
		}
		networks := []string{"tcp", "udp"}
		if options.DisableUDP {
			networks = []string{"tcp"}
		}
		group := &pawsManagedGroup{Adapter: O.NewAdapter("paws-group", tag, networks, options.Outbounds), owner: owner, options: options, interrupt: interrupt.NewGroup()}
		var err error
		if options.Filter != nil && *options.Filter != "" {
			group.filter, err = regexp.Compile(*options.Filter)
			if err != nil {
				return nil, errors.New("unsupported group filter expression")
			}
		}
		if options.ExcludeFilter != nil && *options.ExcludeFilter != "" {
			group.exclude, err = regexp.Compile(*options.ExcludeFilter)
			if err != nil {
				return nil, errors.New("unsupported group exclude-filter expression")
			}
		}
		if options.Kind == "load-balance" && options.Strategy != nil && *options.Strategy != "consistent-hashing" && *options.Strategy != "round-robin" {
			return nil, errors.New("unsupported load-balance strategy")
		}
		if options.Default != nil {
			group.fixed = *options.Default
		}

		owner.groups[tag] = group
		return group, nil
	}
}
func (g *pawsManagedGroup) Start() error { return nil } // Initial roster is validated in Box.StartWithPreparation.
func (g *pawsManagedGroup) All() []string {
	return append([]string(nil), g.members(g.owner.providers.current.Load())...)
}
func (g *pawsManagedGroup) members(generation *pawsOutboundGeneration) []string {
	if generation != nil && generation.groups != nil {
		if roster := generation.groups[g.Tag()]; roster != nil {
			return roster.tags
		}
	}
	return g.buildMembers(generation)
}
func (g *pawsManagedGroup) contains(generation *pawsOutboundGeneration, tag string) bool {
	if generation != nil && generation.groups != nil {
		if roster := generation.groups[g.Tag()]; roster != nil {
			_, ok := roster.index[tag]
			return ok
		}
	}
	return containsPawsTag(g.members(generation), tag)
}
func (g *pawsManagedGroup) buildMembers(generation *pawsOutboundGeneration) []string {
	result := make([]string, 0)
	seen := make(map[string]bool)
	appendTag := func(tag string) {
		if g.options.ExcludeType != nil {
			proxyType := ""
			if group := g.owner.groups[tag]; group != nil {
				proxyType = group.options.Kind
			} else {
				var proxy adapter.Outbound
				if generation != nil {
					proxy = generation.outbounds[tag]
				}
				if proxy == nil {
					proxy, _ = g.owner.manager.Outbound(tag)
				}
				if proxy != nil {
					proxyType = proxy.Type()
				}
			}
			for _, kind := range strings.Split(*g.options.ExcludeType, "|") {
				kind = strings.ToLower(strings.TrimSpace(kind))
				if kind == "ss" {
					kind = "shadowsocks"
				}
				if kind != "" && kind == proxyType {
					return
				}
			}
		}
		if !seen[tag] && (g.filter == nil || g.filter.MatchString(tag)) && (g.exclude == nil || !g.exclude.MatchString(tag)) {
			seen[tag] = true
			result = append(result, tag)
		}
	}
	for _, tag := range g.options.Outbounds {
		appendTag(tag)
	}
	if g.options.IncludeAllProxies {
		for _, tag := range g.owner.staticProxyTags {
			appendTag(tag)
		}
	}
	if generation != nil {
		providers := append([]string(nil), g.options.Use...)
		if g.options.IncludeAllProviders {
			extra := make([]string, 0, len(generation.providers))
			for name := range generation.providers {
				if !containsPawsTag(providers, name) {
					extra = append(extra, name)
				}
			}
			sort.Strings(extra)
			providers = append(providers, extra...)
		}
		for _, name := range providers {
			if provider := generation.providers[name]; provider != nil {
				for _, tag := range provider.tags {
					appendTag(tag)
				}
			}
		}
	}
	return result
}
func (g *pawsManagedGroup) choose(generation *pawsOutboundGeneration, destination M.Socksaddr, advance bool) string {
	members := g.members(generation)
	if len(members) == 0 {
		return ""
	}
	g.mu.Lock()
	defer g.mu.Unlock()
	contains := func(value string) bool { return g.contains(generation, value) }
	if contains(g.fixed) {
		return g.fixed
	}
	if g.options.Kind == "select" {
		return members[0]
	}
	if g.options.Kind == "load-balance" {
		if !advance {
			if contains(g.automatic) {
				return g.automatic
			}
			return members[0]
		}
		var chosen string
		if g.options.Strategy != nil && *g.options.Strategy == "round-robin" {
			chosen = members[(g.round.Add(1)-1)%uint64(len(members))]
		} else {
			hash := fnv.New64a()
			_, _ = hash.Write([]byte(destination.String()))
			chosen = members[hash.Sum64()%uint64(len(members))]
		}
		if g.automatic != chosen {
			g.automatic = chosen
			g.owner.observations.groups.Add(1)
		}
		return chosen
	}
	if contains(g.automatic) {
		return g.automatic
	}
	return members[0]
}
func (g *pawsManagedGroup) Now() string {
	return g.choose(g.owner.providers.current.Load(), M.Socksaddr{}, false)
}
func (g *pawsManagedGroup) Fixed() *string {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.fixed == "" {
		return nil
	}
	value := g.fixed
	return &value
}
func (g *pawsManagedGroup) SelectOutbound(tag string) bool {
	g.owner.providers.mu.Lock()
	if !g.owner.observations.active.Load() || !g.contains(g.owner.providers.current.Load(), tag) {
		g.owner.providers.mu.Unlock()
		return false
	}
	g.mu.Lock()
	changed := g.fixed != tag
	g.fixed = tag
	g.mu.Unlock()
	if changed {
		g.owner.observations.groups.Add(1)
	}
	g.owner.providers.mu.Unlock()
	// Interrupt may close a provider connection and re-enter registry.release.
	if changed {
		g.interrupt.Interrupt(g.options.Interrupt)
	}
	return true
}
func (g *pawsManagedGroup) Unfix() error {
	if !g.owner.observations.active.Load() {
		return errors.New("runtime retired")
	}
	if g.options.Kind == "select" {
		return errors.New("manual selector has no automatic selection")
	}
	g.mu.Lock()
	changed := g.fixed != ""
	g.fixed = ""
	g.mu.Unlock()
	if changed {
		g.owner.observations.groups.Add(1)
	}
	return nil
}
func (g *pawsManagedGroup) dialTarget(destination M.Socksaddr) (adapter.Outbound, *pawsOutboundGeneration, error) {
	if !g.owner.observations.active.Load() {
		return nil, nil, errors.New("runtime retired")
	}
	generation := g.owner.providers.acquire()
	tag := g.choose(generation, destination, true)
	var target adapter.Outbound
	if generation != nil {
		target = generation.outbounds[tag]
	}
	if target == nil {
		target, _ = g.owner.manager.Outbound(tag)
	}
	if target == nil {
		g.owner.providers.release(generation)
		return nil, nil, errors.New("managed group has no live member")
	}
	// Same-generation membership is indexed once at publication. No roster
	// scan/allocation belongs in the per-flow path, including lazy health touch.
	if generation != nil {
		if provider := generation.providerByTag[tag]; provider != nil {
			provider.lastTouch.Store(time.Since(g.owner.providerHealthEpoch).Nanoseconds() + 1)
		}
	}
	if !g.touched.Swap(true) && g.options.Lazy && (g.options.Kind == "url-test" || g.options.Kind == "fallback") {
		g.owner.scheduleProbe(g)
	}
	return target, generation, nil
}
func (g *pawsManagedGroup) DialContext(ctx context.Context, network string, destination M.Socksaddr) (net.Conn, error) {
	target, generation, err := g.dialTarget(destination)
	if err != nil {
		return nil, err
	}
	conn, err := target.DialContext(ctx, network, destination)
	if err != nil {
		g.owner.providers.release(generation)
		return nil, err
	}
	return g.interrupt.NewConn(&pawsGenerationConn{Conn: conn, release: func() { g.owner.providers.release(generation) }}, interrupt.IsExternalConnectionFromContext(ctx)), nil
}
func (g *pawsManagedGroup) ListenPacket(ctx context.Context, destination M.Socksaddr) (net.PacketConn, error) {
	if g.options.DisableUDP {
		return nil, errors.New("UDP is disabled for this group")
	}
	target, generation, err := g.dialTarget(destination)
	if err != nil {
		return nil, err
	}
	conn, err := target.ListenPacket(ctx, destination)
	if err != nil {
		g.owner.providers.release(generation)
		return nil, err
	}
	return g.interrupt.NewPacketConn(&pawsGenerationPacketConn{PacketConn: conn, release: func() { g.owner.providers.release(generation) }}, interrupt.IsExternalConnectionFromContext(ctx)), nil
}

type pawsGenerationConn struct {
	net.Conn
	once    sync.Once
	release func()
}

func (c *pawsGenerationConn) Close() error { err := c.Conn.Close(); c.once.Do(c.release); return err }

type pawsGenerationPacketConn struct {
	net.PacketConn
	once    sync.Once
	release func()
}

func (c *pawsGenerationPacketConn) Close() error {
	err := c.PacketConn.Close()
	c.once.Do(c.release)
	return err
}

// Probes capture the immutable generation before the await. A late probe can
// neither publish into a replacement provider generation nor a new session.
func (p *pawsSingRuntime) probeGroup(ctx context.Context, g *pawsManagedGroup) error {
	generation := p.providers.acquire()
	defer p.providers.release(generation)
	members := g.members(generation)
	if len(members) == 0 {
		return errors.New("empty managed group")
	}
	type outcome struct {
		tag   string
		delay uint32
		err   error
	}
	results := make(chan outcome, len(members))
	semaphore := make(chan struct{}, 8)
	var workers sync.WaitGroup
	for _, tag := range members {
		tag := tag
		workers.Add(1)
		go func() {
			defer workers.Done()
			select {
			case semaphore <- struct{}{}:
			case <-ctx.Done():
				results <- outcome{tag: tag, err: ctx.Err()}
				return
			}
			defer func() { <-semaphore }()
			proxy := generation.outbounds[tag]
			if proxy == nil {
				proxy, _ = p.manager.Outbound(tag)
			}
			delay, err := probePawsOutbound(ctx, proxy, g.options.URL, g.options.ExpectedStatus, time.Duration(g.options.Timeout)*time.Millisecond)
			results <- outcome{tag, delay, err}
		}()
	}
	workers.Wait()
	close(results)
	measured := make(map[string]outcome, len(members))
	for result := range results {
		measured[result.tag] = result
	}
	p.providers.mu.Lock()
	defer p.providers.mu.Unlock()
	if !p.observations.active.Load() || p.providers.current.Load() != generation {
		return errors.New("provider generation changed during probe")
	}
	for _, result := range measured {
		p.delays[result.tag] = pawsDelay{delay: result.delay, alive: result.err == nil}
		p.observations.groups.Add(1)
	}
	chosen := ""
	var fastest uint32
	for _, tag := range members {
		result := measured[tag]
		if result.err != nil {
			continue
		}
		if chosen == "" || (g.options.Kind != "fallback" && result.delay < fastest) {
			chosen = tag
			fastest = result.delay
		}
		if g.options.Kind == "fallback" {
			break
		}
	}
	g.mu.Lock()
	if chosen != "" {
		if previous, ok := measured[g.automatic]; g.options.Kind == "url-test" && ok && previous.err == nil && previous.delay <= fastest+g.options.Tolerance {
			chosen = g.automatic
		}
		if g.automatic != chosen {
			g.automatic = chosen
			p.observations.groups.Add(1)
		}
	}
	g.mu.Unlock()
	return nil
}
func probePawsOutbound(ctx context.Context, proxy adapter.Outbound, urlValue, statusValue *string, timeout time.Duration) (uint32, error) {
	if proxy == nil {
		return 0, errors.New("probe outbound is unavailable")
	}
	link := "https://www.gstatic.com/generate_204"
	if urlValue != nil && *urlValue != "" {
		link = *urlValue
	}
	ctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	transport := &http.Transport{Proxy: nil, DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
		return proxy.DialContext(ctx, network, M.ParseSocksaddr(address))
	}}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	request, err := http.NewRequestWithContext(ctx, "GET", link, nil)
	if err != nil {
		return 0, errors.New("invalid probe URL")
	}
	started := time.Now()
	response, err := client.Do(request)
	if err != nil {
		return 0, errors.New("probe outbound request failed")
	}
	_ = response.Body.Close()
	if statusValue != nil && *statusValue != "" && !pawsStatusAccepted(response.StatusCode, *statusValue) {
		return 0, errors.New("probe response status was rejected")
	}
	elapsed := time.Since(started).Milliseconds()
	if elapsed < 1 {
		elapsed = 1
	}
	if elapsed > int64(^uint32(0)) {
		return 0, errors.New("probe delay overflow")
	}
	return uint32(elapsed), nil
}
func pawsStatusAccepted(status int, spec string) bool {
	for _, part := range strings.Split(spec, "/") {
		part = strings.TrimSpace(part)
		bounds := strings.Split(part, "-")
		first, err := strconv.Atoi(bounds[0])
		if err != nil {
			continue
		}
		last := first
		if len(bounds) == 2 {
			last, err = strconv.Atoi(bounds[1])
			if err != nil {
				continue
			}
		}
		if len(bounds) <= 2 && status >= first && status <= last {
			return true
		}
	}
	return false
}

func (g *pawsManagedGroup) Close() error { g.interrupt.Interrupt(true); return nil }

func (g *pawsManagedGroup) ClashType() string {
	switch g.options.Kind {
	case "select":
		return "Selector"
	case "url-test":
		return "URLTest"
	case "fallback":
		return "Fallback"
	default:
		return "LoadBalance"
	}
}
