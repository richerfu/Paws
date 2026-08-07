use super::*;

pub(super) fn about_snapshot() -> AboutSnapshot {
    AboutSnapshot {
        app_version: APP_VERSION.to_owned(),
        core_version: env!("CARGO_PKG_VERSION").to_owned(),
        meow_rs_version: MEOW_RS_VERSION.to_owned(),
        arkit_rev: ARKIT_REV.to_owned(),
        rust_version: RUST_VERSION.to_owned(),
        privacy_summary: vec![
            "数据控制者：Paws 不要求注册账号，不接入广告、行为分析或远程遥测服务，也没有用于收集用户数据的 Paws 后端。".to_owned(),
            "配置数据：订阅地址与内容、YAML 和备份、规则、节点选择及资源缓存保存在应用私有目录，用于代理运行；Paws 不会主动把这些数据上传到 Paws 或其他分析服务。".to_owned(),
            "运行诊断：DNS 查询记录、连接目标与代理链、流量统计和当前运行日志只在本机内存或应用进程间共享，用于页面展示和排错，不会发送到远程遥测服务。".to_owned(),
            "订阅与规则提供方：只有在导入、刷新或运行用户配置的远程资源时，Paws 才会访问对应 URL。服务方可能获得当前出口 IP、标准 HTTP 请求信息，以及 URL 中由用户自行配置的鉴权参数；Paws 不会把订阅内容转交给其他服务。".to_owned(),
            "出口 IP 查询：仅在 VPN 已连接且页面需要刷新时，经当前混合代理并发请求本页列出的 HTTPS 服务；混合代理不可用时会尝试系统 VPN 路径。服务方会获得当前公网出口 IP 和标准请求头，但请求不包含订阅、节点、规则、DNS 记录或连接记录。".to_owned(),
            "出口 IP 结果：首个有效服务返回的公网 IP、国家或地区、国家代码和服务名称仅保存在运行内存中用于首页展示；断开 VPN 会清除，Paws 不会将结果上传到其他服务。".to_owned(),
            "日志与导出：持久化日志记录需要用户主动开启，开启后日志写入应用私有目录；用户可以在应用内清理或导出。导出文件离开私有目录后由用户自行保管和删除，分享前应检查其中的域名、IP 或节点信息。".to_owned(),
            "局域网控制器：默认只监听 127.0.0.1。启用局域网访问后，控制器会监听 0.0.0.0 并使用随机密钥鉴权；持有密钥的局域网设备可能读取运行状态或控制代理，请勿在不可信网络启用或泄露密钥。".to_owned(),
            "删除与保留：可在应用内删除配置、规则和日志；卸载应用时，应用私有目录按系统规则移除。此前主动导出的文件不会随卸载自动删除。".to_owned(),
            "外部链接：Paws 只会在用户点击后打开项目或第三方服务文档；离开应用后适用对应网站的隐私政策。隐私说明最后更新于 2026-08-07。".to_owned(),
        ],
        exit_ip_services: exit_ip_service_summaries(),
    }
}

pub(super) fn active_connections_from_tunnel(tunnel: &Tunnel) -> Vec<ConnectionSummary> {
    let mut connections: Vec<_> = tunnel
        .statistics()
        .active_connections()
        .into_iter()
        .map(|connection| {
            let chains: Vec<String> = connection
                .chains
                .into_iter()
                .map(|chain| chain.to_string())
                .collect();
            let proxy = if chains.is_empty() {
                "DIRECT".to_owned()
            } else {
                chains.join(" > ")
            };
            let rule_payload = connection.rule_payload.to_string();
            let rule = if rule_payload.is_empty() {
                connection.rule.to_string()
            } else {
                format!("{}({})", connection.rule, rule_payload)
            };
            ConnectionSummary {
                id: connection.id.to_string(),
                host: connection.metadata.remote_address().to_string(),
                domain: connection.metadata.rule_host().to_owned(),
                destination_ip: connection
                    .metadata
                    .dst_ip
                    .map(|ip| ip.to_string())
                    .unwrap_or_default(),
                destination_port: connection.metadata.dst_port,
                network: connection.metadata.network.to_string(),
                rule,
                rule_payload,
                proxy,
                chains,
                started_at: connection.start.to_string(),
                upload_bytes: non_negative_i64_to_u64(connection.counters.upload_bytes()),
                download_bytes: non_negative_i64_to_u64(connection.counters.download_bytes()),
            }
        })
        .collect();
    connections.sort_by(|a, b| a.host.cmp(&b.host).then_with(|| a.id.cmp(&b.id)));
    connections
}

pub(super) fn record_request_history(state: &mut CoreState, connections: &[ConnectionSummary]) {
    let now = system_time_secs(SystemTime::now()).unwrap_or_else(|| "now".to_owned());
    for request in &mut state.request_history {
        request.active = false;
    }

    for connection in connections {
        if let Some(request) = state
            .request_history
            .iter_mut()
            .find(|request| request.id == connection.id)
        {
            request.host = connection.host.clone();
            request.domain = connection.domain.clone();
            request.destination_ip = connection.destination_ip.clone();
            request.destination_port = connection.destination_port;
            request.network = connection.network.clone();
            request.rule = connection.rule.clone();
            request.proxy = connection.proxy.clone();
            request.upload_bytes = connection.upload_bytes;
            request.download_bytes = connection.download_bytes;
            request.active = true;
            request.updated_at = now.clone();
            continue;
        }

        state.request_history.push_back(RequestSummary {
            id: connection.id.clone(),
            host: connection.host.clone(),
            domain: connection.domain.clone(),
            destination_ip: connection.destination_ip.clone(),
            destination_port: connection.destination_port,
            network: connection.network.clone(),
            rule: connection.rule.clone(),
            proxy: connection.proxy.clone(),
            upload_bytes: connection.upload_bytes,
            download_bytes: connection.download_bytes,
            active: true,
            updated_at: now.clone(),
        });
        while state.request_history.len() > MAX_REQUEST_HISTORY {
            state.request_history.pop_front();
        }
    }
}

pub(super) fn non_negative_i64_to_u64(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(0)
}

pub(super) fn controller_url(
    addr: SocketAddr,
    segments: &[&str],
) -> Result<reqwest::Url, HMetaError> {
    let mut url = reqwest::Url::parse(&format!("http://{addr}/"))
        .map_err(|err| HMetaError::Core(format!("invalid controller URL: {err}")))?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| HMetaError::Core("controller URL cannot be a base".to_owned()))?;
        path.clear();
        for segment in segments {
            path.push(segment);
        }
    }
    Ok(url)
}

pub(super) fn controller_credentials(state: &CoreState) -> Option<(SocketAddr, Option<String>)> {
    let controller = state.api_controller.as_ref()?;
    let secret = controller
        .raw_config
        .read()
        .secret
        .clone()
        .filter(|secret| !secret.is_empty());
    Some((controller.client_addr, secret))
}

pub(super) fn proxy_groups_from_tunnel(tunnel: &Tunnel) -> Vec<ProxyGroup> {
    let route = tunnel.route_snapshot();
    let proxies = &route.proxies;
    let mut groups: Vec<_> = proxies
        .values()
        .filter_map(|proxy| {
            let members = proxy.members()?;
            let selected = proxy.current();
            Some(ProxyGroup {
                name: proxy.name().to_owned(),
                group_type: proxy.adapter_type().to_string(),
                selected: selected.clone(),
                fixed: proxy.selection().and_then(|selection| selection.fixed()),
                proxies: members
                    .into_iter()
                    .map(|name| {
                        proxy_item(
                            proxies.get(name.as_str()),
                            &name,
                            selected.as_deref() == Some(name.as_str()),
                        )
                    })
                    .collect(),
            })
        })
        .collect();
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    groups
}

pub(super) fn refresh_proxy_groups_preserving_order(state: &mut CoreState, tunnel: &Tunnel) {
    let mut refreshed = proxy_groups_from_tunnel(tunnel);
    preserve_proxy_group_member_order(&state.proxy_groups, &mut refreshed);
    state.proxy_groups = refreshed;
    persist_runtime_ui_cache_best_effort(state);
}

pub(super) fn preserve_proxy_group_member_order(
    previous: &[ProxyGroup],
    refreshed: &mut [ProxyGroup],
) {
    for group in refreshed {
        let Some(previous_group) = previous
            .iter()
            .find(|candidate| candidate.name == group.name)
        else {
            continue;
        };
        let positions = previous_group
            .proxies
            .iter()
            .enumerate()
            .map(|(index, proxy)| (proxy.name.as_str(), index))
            .collect::<std::collections::HashMap<_, _>>();
        // Rust's stable sort also preserves the tunnel order of newly added
        // provider nodes after every member already present in the snapshot.
        group.proxies.sort_by_key(|proxy| {
            positions
                .get(proxy.name.as_str())
                .copied()
                .map_or((1, usize::MAX), |index| (0, index))
        });
    }
}

pub(super) fn proxy_item(
    proxy: Option<&Arc<dyn meow_common::Proxy>>,
    name: &str,
    selected: bool,
) -> ProxyItem {
    ProxyItem {
        name: name.to_owned(),
        proxy_type: proxy
            .map(|proxy| proxy.adapter_type().to_string())
            .unwrap_or_else(|| "Unknown".to_owned()),
        delay_ms: proxy.and_then(|proxy| {
            let delay = proxy.last_delay();
            (delay > 0).then_some(u32::from(delay))
        }),
        selected,
    }
}
