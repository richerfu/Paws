package main

import (
	"context"
	"time"
)

// Provider health is independent of a group's URL-test policy. It is owned by
// this runtime, and Stop cancels and joins both the scheduler and real probes.
func (p *pawsSingRuntime) startProviderHealth(name string, definition pawsProviderDefinition) {
	if !definition.Health.Enable {
		return
	}
	interval := definition.Health.Interval
	if interval == 0 {
		interval = 300 // Same default as the pinned Clash provider schema.
	}
	p.scheduleProviderHealth(name)
	p.launch(func() {
		ticker := time.NewTicker(time.Duration(interval) * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-p.ctx.Done():
				return
			case <-ticker.C:
				generation := p.providers.current.Load()
				if generation == nil {
					continue
				}
				record := generation.providers[name]
				if record == nil {
					continue
				}
				lastTouch := record.lastTouch.Load()
				if !definition.Health.Lazy || (lastTouch != 0 && time.Since(p.providerHealthEpoch)-time.Duration(lastTouch-1) < time.Duration(interval)*time.Second) {
					p.scheduleProviderHealth(name)
				}
			}
		}
	})
}

// Coalesce timer ticks and refresh requests by provider and generation. An old
// generation finishing after refresh cannot suppress the new generation's
// initial health check. Native objects are still captured by probeProvider.
func (p *pawsSingRuntime) scheduleProviderHealth(name string) {
	if !p.observations.active.Load() {
		return
	}
	p.providerHealthMu.Lock()
	generation := p.providers.current.Load()
	if generation == nil || generation.providers[name] == nil || !generation.providers[name].definition.Health.Enable {
		p.providerHealthMu.Unlock()
		return
	}
	if task := p.providerHealthTasks[name]; task != nil {
		task.generation = generation
		p.providerHealthMu.Unlock()
		return
	}
	task := &pawsProviderHealthTask{generation: generation}
	p.providerHealthTasks[name] = task
	p.providerHealthMu.Unlock()
	if p.launch(func() {
		p.lease.RLock()
		defer p.lease.RUnlock()
		for {
			p.providerHealthMu.Lock()
			target := task.generation
			p.providerHealthMu.Unlock()
			if p.observations.active.Load() {
				if record := target.providers[name]; record != nil {
					timeout := record.definition.Health.Timeout
					if timeout == 0 {
						timeout = 5000
					}
					link, expected := record.definition.Health.URL, record.definition.Health.ExpectedStatus
					_, _ = p.probeProviderGeneration(p.ctx, name, "", &link, &expected, time.Duration(timeout)*time.Millisecond, target)
				}
			}
			p.providerHealthMu.Lock()
			if p.observations.active.Load() && task.generation != target {
				p.providerHealthMu.Unlock()
				continue
			}
			delete(p.providerHealthTasks, name)
			p.providerHealthMu.Unlock()
			return
		}
	}) {
		return
	}
	p.providerHealthMu.Lock()
	delete(p.providerHealthTasks, name)
	p.providerHealthMu.Unlock()
}

// Explicit and periodic checks share one provider gate. Queued HTTP callers
// remain cancellable, and each real outbound probe also consumes one of eight
// session-wide slots. A timer cannot multiply per-provider work while slow.
func (p *pawsSingRuntime) enterProviderProbe(ctx context.Context, name string) (func(), error) {
	value, _ := p.providerProbeLocks.LoadOrStore(name, make(chan struct{}, 1))
	gate := value.(chan struct{})
	select {
	case gate <- struct{}{}:
		return func() { <-gate }, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-p.ctx.Done():
		return nil, p.ctx.Err()
	}
}
