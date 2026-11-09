package main

import (
	"encoding/json"
	"sync"
	"sync/atomic"
)

type pawsGroupSummary struct {
	Name          string  `json:"name"`
	GroupType     string  `json:"groupType"`
	Selected      *string `json:"selected"`
	Fixed         *string `json:"fixed"`
	MemberCount   int     `json:"memberCount"`
	SelectedDelay *uint32 `json:"selectedDelayMs"`
}
type pawsProxyItem struct {
	Name      string  `json:"name"`
	ProxyType string  `json:"proxyType"`
	Delay     *uint32 `json:"delayMs"`
	Selected  bool    `json:"selected"`
}
type pawsProxyGroup struct {
	Name      string          `json:"name"`
	GroupType string          `json:"groupType"`
	Selected  *string         `json:"selected"`
	Fixed     *string         `json:"fixed"`
	Proxies   []pawsProxyItem `json:"proxies"`
}
type pawsProviderProxy struct {
	Name      string  `json:"name"`
	ProxyType string  `json:"proxyType"`
	Delay     *uint32 `json:"delayMs"`
	Alive     *bool   `json:"alive"`
}
type pawsProviderState struct {
	Kind      string               `json:"kind"`
	Name      string               `json:"name"`
	Members   *[]pawsProviderProxy `json:"members"`
	RuleCount *int                 `json:"ruleCount"`
	UpdatedAt *string              `json:"updatedAt"`
	Vehicle   *string              `json:"vehicle"`
}
type pawsRuntimeRule struct {
	RuleType string `json:"ruleType"`
	Payload  string `json:"payload"`
	Target   string `json:"target"`
}
type pawsConnection struct {
	DomainSource    string   `json:"domainSource"`
	ID              string   `json:"id"`
	Host            string   `json:"host"`
	Domain          string   `json:"domain"`
	DestinationIP   string   `json:"destinationIp"`
	DestinationPort uint16   `json:"destinationPort"`
	Network         string   `json:"network"`
	Rule            string   `json:"rule"`
	RulePayload     string   `json:"rulePayload"`
	Proxy           string   `json:"proxy"`
	Chains          []string `json:"chains"`
	StartedAt       string   `json:"startedAt"`
	Upload          uint64   `json:"uploadBytes"`
	Download        uint64   `json:"downloadBytes"`
}
type pawsConnectionSummary struct {
	Count          int              `json:"count"`
	Upload         uint64           `json:"uploadBytes"`
	Download       uint64           `json:"downloadBytes"`
	Recent         []pawsConnection `json:"recent"`
	LatestRuleNode *string          `json:"latestRuleNode"`
}
type pawsConnectionEvent struct {
	Sequence   uint64         `json:"sequence"`
	Connection pawsConnection `json:"connection"`
	Closed     bool           `json:"closed"`
}
type pawsLogRecord struct {
	Sequence  uint64 `json:"sequence"`
	Level     string `json:"level"`
	Message   string `json:"message"`
	Timestamp string `json:"timestamp"`
}
type pawsDnsConfig struct {
	Model     string              `json:"model"`
	Listen    *string             `json:"listen"`
	Upstreams []string            `json:"upstreams"`
	Fallbacks []string            `json:"fallbacks"`
	Policy    map[string][]string `json:"nameserverPolicy"`
}

// A small acknowledgment of successfully owned listener handles. Credentials
// stay in the original authenticated client; they are never sent on this wire.
type pawsAppliedServices struct {
	BenchmarkPort           uint16  `json:"benchmarkPort"`
	PublicControllerAddress *string `json:"publicControllerAddr"`
	MixedReady              bool    `json:"mixedReady"`
}
type pawsMemory struct {
	Scope string  `json:"scope"`
	InUse uint64  `json:"inUseBytes"`
	Limit *uint64 `json:"limitBytes"`
}
type pawsRevisions struct {
	Groups      uint64 `json:"groups"`
	Providers   uint64 `json:"providers"`
	Rules       uint64 `json:"rules"`
	Dns         uint64 `json:"dns"`
	Connections uint64 `json:"connections"`
	Logs        uint64 `json:"logs"`
}
type pawsObservations struct {
	active         atomic.Bool
	groups         atomic.Uint64
	providers      atomic.Uint64
	rules          atomic.Uint64
	logs           *pawsJournal
	connections    *pawsJournal
	mu             sync.Mutex
	latestRuleNode *string
}

func newPawsObservations() *pawsObservations {
	p := &pawsObservations{logs: newPawsJournal(512, 256*1024), connections: newPawsJournal(1024, 1024*1024)}
	p.active.Store(true)
	return p
}
func (p *pawsObservations) revisions() pawsRevisions {
	return pawsRevisions{Groups: p.groups.Load(), Providers: p.providers.Load(), Rules: p.rules.Load(), Connections: p.connections.revision(), Logs: p.logs.revision()}
}
func (p *pawsObservations) connection(connection pawsConnection, closed bool) error {
	if !p.active.Load() {
		return nil
	}
	if !closed {
		p.mu.Lock()
		if connection.Proxy != "" {
			node := connection.Proxy
			p.latestRuleNode = &node
		}
		p.mu.Unlock()
	}
	return p.connections.append(func(sequence uint64) ([]byte, error) {
		return json.Marshal(pawsConnectionEvent{sequence, connection, closed})
	})
}
func (p *pawsObservations) log(level, message, timestamp string) error {
	if !p.active.Load() {
		return nil
	}
	return p.logs.append(func(sequence uint64) ([]byte, error) {
		return json.Marshal(pawsLogRecord{sequence, level, message, timestamp})
	})
}
