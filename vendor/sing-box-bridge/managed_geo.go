package main

import (
	"encoding/binary"
	"encoding/json"
	"errors"
	"io"
	"os"
	"sort"
	"strings"

	"github.com/sagernet/sing-box/option"
	SJ "github.com/sagernet/sing/common/json"
)

type pawsGeoDataPaths struct {
	Country *string `json:"countryMmdb"`
	ASN     *string `json:"asnMmdb"`
	Site    *string `json:"geosite"`
}
type pawsGeoReference struct {
	kind     uint32
	category string
}

func pawsGeoTag(tag string) (pawsGeoReference, bool) {
	for _, source := range []struct {
		prefix string
		kind   uint32
	}{{"paws-geosite:", 3}, {"paws-geoip:", 4}, {"paws-asn:", 5}} {
		if strings.HasPrefix(tag, source.prefix) {
			return pawsGeoReference{source.kind, strings.TrimPrefix(tag, source.prefix)}, true
		}
	}
	return pawsGeoReference{}, false
}
func expandPawsDNSMatchers(value string) []string {
	for _, prefix := range []string{"geosite:", "rule-set:"} {
		if strings.HasPrefix(strings.ToLower(value), prefix) {
			parts := strings.Split(value[len(prefix):], ",")
			for index, part := range parts {
				parts[index] = prefix + strings.TrimSpace(part)
			}
			return parts
		}
	}
	parts := strings.Split(value, ",")
	for index := range parts {
		parts[index] = strings.TrimSpace(parts[index])
	}
	return parts
}

// Only private compiler markers are consumed here. Ordered route rules and
// MATCH positions remain unchanged; Resolve is emitted at the exact lazy IP
// condition boundary declared by the shared Rust rule compiler.
func (p *pawsSingRuntime) prepareNativeDocument(document map[string]json.RawMessage) error {
	var route map[string]any
	if raw := document["route"]; len(raw) > 0 {
		if err := json.Unmarshal(raw, &route); err != nil {
			return errors.New("invalid route options")
		}
	}
	if route == nil {
		route = make(map[string]any)
	}
	var collect func(any)
	collect = func(value any) {
		switch typed := value.(type) {
		case map[string]any:
			if tags, ok := typed["rule_set"].([]any); ok {
				for _, tag := range tags {
					if text, ok := tag.(string); ok {
						if reference, ok := pawsGeoTag(text); ok {
							p.geoReferences[text] = reference
						}
					}
				}
			}
			for _, child := range typed {
				collect(child)
			}
		case []any:
			for _, child := range typed {
				collect(child)
			}
		}
	}
	collect(route)
	if rules, ok := route["rules"].([]any); ok {
		compiled := make([]any, 0, len(rules))
		for _, raw := range rules {
			rule, ok := raw.(map[string]any)
			if !ok {
				return errors.New("invalid compiled route rule")
			}
			if resolve, ok := rule["paws_resolve"].(bool); ok && resolve {
				compiled = append(compiled, map[string]any{"action": "resolve"})
			}
			delete(rule, "paws_resolve")
			compiled = append(compiled, rule)
		}
		route["rules"] = compiled
	}
	var dns struct {
		Servers []struct {
			Policies     []pawsDNSPolicyOptions `json:"policies"`
			FakeIPFilter []string               `json:"fakeIPFilter"`
		} `json:"servers"`
	}
	if raw := document["dns"]; len(raw) > 0 {
		if err := json.Unmarshal(raw, &dns); err != nil {
			return errors.New("invalid DNS options")
		}
	}
	for _, server := range dns.Servers {
		matchers := append([]string(nil), server.FakeIPFilter...)
		for _, policy := range server.Policies {
			matchers = append(matchers, policy.Matcher)
		}
		for _, source := range matchers {
			for _, matcher := range expandPawsDNSMatchers(source) {
				if strings.HasPrefix(strings.ToLower(matcher), "geosite:") {
					category := matcher[len("geosite:"):]
					p.geoReferences["paws-geosite:"+category] = pawsGeoReference{3, category}
				}
			}
		}
	}
	sets, _ := route["rule_set"].([]any)
	known := make(map[string]bool)
	for _, set := range sets {
		if value, ok := set.(map[string]any); ok {
			if tag, ok := value["tag"].(string); ok {
				known[tag] = true
			}
		}
	}
	tags := make([]string, 0, len(p.geoReferences)+len(p.ruleDefinitions))
	for tag := range p.geoReferences {
		tags = append(tags, tag)
	}
	for name := range p.ruleDefinitions {
		tags = append(tags, "paws-provider:"+name)
	}
	sort.Strings(tags)
	for _, tag := range tags {
		if known[tag] {
			continue
		}
		known[tag] = true
		sets = append(sets, map[string]any{"type": "inline", "tag": tag, "rules": []any{}})
	}
	if len(sets) > 0 {
		route["rule_set"] = sets
	}
	raw, err := json.Marshal(route)
	if err != nil {
		return err
	}
	document["route"] = raw
	return nil
}
func (p *pawsSingRuntime) compileGeo(tag string) (pawsCompiledProvider, error) {
	if result, ok := p.geoCompiled[tag]; ok {
		return result, nil
	}
	reference, ok := pawsGeoTag(tag)
	if !ok || reference.category == "" {
		return pawsCompiledProvider{}, errors.New("invalid compiled geodata reference")
	}
	path := p.metadata.GeoDataPaths.Site
	if reference.kind == 4 {
		path = p.metadata.GeoDataPaths.Country
	} else if reference.kind == 5 {
		path = p.metadata.GeoDataPaths.ASN
	}
	if path == nil || *path == "" {
		return pawsCompiledProvider{}, errors.New("prepared geodata path is missing")
	}
	body, ok := p.geoFiles[reference.kind]
	if !ok {
		file, err := os.Open(*path)
		if err != nil {
			return pawsCompiledProvider{}, errors.New("prepared geodata resource is unavailable")
		}
		body, err = io.ReadAll(io.LimitReader(file, pawsGeoInputLimit+1))
		_ = file.Close()
		if err != nil || len(body) > pawsGeoInputLimit {
			return pawsCompiledProvider{}, errors.New("geodata resource exceeds byte budget or cannot be read")
		}
		if p.geoFileBytes+len(body) > 128*1024*1024 {
			return pawsCompiledProvider{}, errors.New("combined geodata input exceeds runtime budget")
		}
		p.geoFileBytes += len(body)
		p.geoFiles[reference.kind] = body
	}
	category := []byte(reference.category)
	if len(category) == 0 || len(category) > 512 {
		return pawsCompiledProvider{}, errors.New("invalid geodata category length")
	}
	input := make([]byte, 4+len(category)+len(body))
	binary.LittleEndian.PutUint32(input, uint32(len(category)))
	copy(input[4:], category)
	copy(input[4+len(category):], body)
	result, err := callPawsCompiler(reference.kind, 0, 3, input, pawsGeoInputLimit)
	if err != nil {
		return result, err
	}
	size := 0
	for _, rule := range result.Rules {
		size += len(rule)
	}
	if len(p.geoCompiled) >= 1024 || p.geoOutputBytes+size > 64*1024*1024 {
		return pawsCompiledProvider{}, errors.New("compiled geodata runtime budget exceeded")
	}
	p.geoOutputBytes += size
	p.geoCompiled[tag] = result
	return result, nil
}
func (p *pawsSingRuntime) initializeGeo() error {
	tags := make([]string, 0, len(p.geoReferences))
	for tag := range p.geoReferences {
		tags = append(tags, tag)
	}
	sort.Strings(tags)
	for _, tag := range tags {
		compiled, err := p.compileGeo(tag)
		if err != nil {
			return err
		}
		rules := make([]option.HeadlessRule, 0, len(compiled.Rules))
		for _, raw := range compiled.Rules {
			value, err := SJ.UnmarshalExtendedContext[option.HeadlessRule](p.ctx, raw)
			if err != nil {
				return errors.New("compiled geodata has an invalid native schema")
			}
			rules = append(rules, value)
		}
		set, ok := p.router.RuleSet(tag)
		if !ok {
			return errors.New("compiled geodata native slot is missing")
		}
		updater, ok := set.(interface {
			Update([]option.HeadlessRule) error
		})
		if !ok {
			return errors.New("compiled geodata slot cannot update")
		}
		if err = updater.Update(rules); err != nil {
			return errors.New("compiled geodata failed native validation")
		}
	}
	return nil
}

// A classical provider can reference a category not mentioned in static rules.
// Expand it to this session's immutable compiled native matcher before the
// provider's entire candidate is validated; no new global route slot is needed.
func (p *pawsSingRuntime) expandProviderGeo(raw json.RawMessage) (json.RawMessage, error) {
	var value any
	if err := json.Unmarshal(raw, &value); err != nil {
		return nil, errors.New("invalid converted provider rule")
	}
	expandedBytes := len(raw)
	var expand func(any) (any, error)
	expand = func(value any) (any, error) {
		switch typed := value.(type) {
		case map[string]any:
			if children, ok := typed["rules"].([]any); ok {
				for index, child := range children {
					next, err := expand(child)
					if err != nil {
						return nil, err
					}
					children[index] = next
				}
			}
			tags, ok := typed["rule_set"].([]any)
			if !ok {
				return typed, nil
			}
			geoRules := make([]any, 0)
			for _, tag := range tags {
				text, ok := tag.(string)
				if !ok {
					return nil, errors.New("invalid provider rule-set reference")
				}
				if _, geo := pawsGeoTag(text); !geo {
					return nil, errors.New("nested provider rule-set references are not supported by native headless rules")
				}
				compiled, err := p.compileGeo(text)
				if err != nil {
					return nil, err
				}
				for _, raw := range compiled.Rules {
					expandedBytes += len(raw)
					if expandedBytes > pawsProviderOutputLimit {
						return nil, errors.New("expanded provider geodata exceeds output budget")
					}
					var rule any
					if err = json.Unmarshal(raw, &rule); err != nil {
						return nil, err
					}
					geoRules = append(geoRules, rule)
				}
			}
			if source, _ := typed["rule_set_ip_cidr_match_source"].(bool); source {
				var sourceProjection func(any)
				sourceProjection = func(value any) {
					switch rule := value.(type) {
					case map[string]any:
						if cidrs, ok := rule["ip_cidr"]; ok {
							rule["source_ip_cidr"] = cidrs
							delete(rule, "ip_cidr")
						}
						for _, child := range rule {
							sourceProjection(child)
						}
					case []any:
						for _, child := range rule {
							sourceProjection(child)
						}
					}
				}
				sourceProjection(geoRules)
			}
			delete(typed, "rule_set")
			delete(typed, "rule_set_ip_cidr_match_source")
			delete(typed, "type")
			combined := map[string]any{"type": "logical", "mode": "or", "rules": geoRules}
			if len(typed) > 0 {
				combined = map[string]any{"type": "logical", "mode": "and", "rules": []any{typed, combined}}
			}
			return combined, nil
		default:
			return value, nil
		}
	}
	expanded, err := expand(value)
	if err != nil {
		return nil, err
	}
	return json.Marshal(expanded)
}
