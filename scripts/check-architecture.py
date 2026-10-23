#!/usr/bin/env python3
"""Check the three-layer workspace using Cargo's resolved package metadata."""
import json
from pathlib import Path
import subprocess
import tomllib

root = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.check_output(
    ['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=root))
packages = {item['name']: item for item in metadata['packages'] if item['name'].startswith('paws_')}
allowed = {
    'paws_model': set(),
    'paws_engine': {'paws_model'},
    'paws_profile': {'paws_model'},
    'paws_vpn': {'paws_engine', 'paws_model'},
    'paws_engine_meow': {'paws_engine', 'paws_model', 'paws_vpn'},
    'paws_engine_go': {'paws_engine', 'paws_model'},
    'paws_core': {'paws_engine', 'paws_engine_meow', 'paws_engine_go', 'paws_model', 'paws_profile', 'paws_vpn'},
    'paws_app': {'paws_core', 'paws_model', 'paws_profile'},
    'paws_ui': {'paws_app', 'paws_core', 'paws_model'},
}
if set(packages) != set(allowed):
    raise SystemExit(f'Unreviewed workspace package set: {sorted(set(packages) ^ set(allowed))}')
graph = {}
for name, item in sorted(packages.items()):
    if name in {'paws_model', 'paws_engine', 'paws_profile', 'paws_vpn', 'paws_app', 'paws_ui'}:
        backends = {dep['name'] for dep in item['dependencies'] if dep['kind'] != 'dev'
                    and ((dep['name'].startswith('meow-') and dep['name'] != 'meow-lwip')
                         or dep['name'] in {'sing-box', 'mihomo', 'paws_engine_go', 'paws_engine_meow'})}
        if backends:
            raise SystemExit(f'{name} directly depends on concrete proxy backends: {sorted(backends)}')
    dependencies = {dep['name'] for dep in item['dependencies'] if dep['name'] in packages and dep['kind'] != 'dev'}
    if dependencies - allowed[name]:
        raise SystemExit(f'{name} crosses a layer boundary: {sorted(dependencies - allowed[name])}')
    manifest = tomllib.loads(Path(item['manifest_path']).read_text())
    groups = [manifest.get('dependencies', {}), manifest.get('dev-dependencies', {}), manifest.get('build-dependencies', {})]
    groups.extend(group for target in manifest.get('target', {}).values() for group in (
        target.get('dependencies', {}), target.get('dev-dependencies', {}), target.get('build-dependencies', {})))
    for group in groups:
        for dependency, configuration in group.items():
            if not isinstance(configuration, dict) or configuration.get('workspace') is not True:
                raise SystemExit(f'{name}: {dependency} must inherit its dependency from the workspace')
    graph[name] = dependencies

def visit(name, active, visited):
    if name in active:
        raise SystemExit(f'Workspace dependency cycle: {name}')
    if name in visited:
        return
    for child in graph[name]:
        visit(child, active | {name}, visited)
    visited.add(name)

visited = set()
for package in graph:
    visit(package, set(), visited)
print(json.dumps({name: sorted(edges) for name, edges in graph.items()}, indent=2))
print(f'Architecture verified: {len(packages)} Paws crates; acyclic; workspace dependencies centralized.')
