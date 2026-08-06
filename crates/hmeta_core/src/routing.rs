use super::*;

pub(super) fn target_routes_through_proxy(
    tunnel: &Tunnel,
    target: &str,
    visiting: &mut BTreeSet<String>,
) -> bool {
    if matches!(
        target.to_ascii_uppercase().as_str(),
        "DIRECT" | "REJECT" | "REJECT-DROP"
    ) {
        return false;
    }
    if !visiting.insert(target.to_owned()) {
        return false;
    }

    let routes_through_proxy = match tunnel.proxy(target) {
        // Provider-backed group members are not always registered as
        // top-level route entries. They are still real proxy members unless
        // they use one of the built-in direct/reject names handled above.
        None => true,
        Some(proxy) => match proxy.adapter_type() {
            AdapterType::Direct | AdapterType::Reject | AdapterType::RejectDrop => false,
            AdapterType::Selector
            | AdapterType::Fallback
            | AdapterType::UrlTest
            | AdapterType::LoadBalance
            | AdapterType::Relay => {
                if let Some(current) = proxy.current() {
                    target_routes_through_proxy(tunnel, &current, visiting)
                } else if let Some(members) = proxy.members() {
                    !members.is_empty()
                        && members
                            .iter()
                            .all(|member| target_routes_through_proxy(tunnel, member, visiting))
                } else {
                    false
                }
            }
            _ => true,
        },
    };
    visiting.remove(target);
    routes_through_proxy
}

pub(super) fn ensure_global_proxy_selected(
    tunnel: &Tunnel,
    required_target: Option<&str>,
) -> Result<Option<String>, HMetaError> {
    let global = tunnel
        .proxy("GLOBAL")
        .ok_or_else(|| HMetaError::Core("Global mode has no GLOBAL proxy selector".to_owned()))?;
    let Some(members) = global.members() else {
        return if target_routes_through_proxy(tunnel, "GLOBAL", &mut BTreeSet::new()) {
            Ok(None)
        } else {
            Err(HMetaError::Core(
                "Global mode requires at least one proxy node".to_owned(),
            ))
        };
    };

    let is_concrete_proxy = |target: &str| {
        match tunnel.proxy(target) {
            // Provider-backed leaves may only exist inside their parent group.
            None => true,
            Some(proxy) => !matches!(
                proxy.adapter_type(),
                AdapterType::Direct
                    | AdapterType::Reject
                    | AdapterType::RejectDrop
                    | AdapterType::Selector
                    | AdapterType::Fallback
                    | AdapterType::UrlTest
                    | AdapterType::LoadBalance
                    | AdapterType::Relay
            ),
        }
    };
    let is_valid_target = |target: &str| {
        members.iter().any(|member| member == target)
            && is_concrete_proxy(target)
            && target_routes_through_proxy(tunnel, target, &mut BTreeSet::new())
    };

    let current = global.current();
    let target = if let Some(required_target) = required_target {
        if required_target == "DIRECT" {
            if !members.iter().any(|member| member == "DIRECT") {
                return Err(HMetaError::Core(
                    "GLOBAL has no built-in DIRECT outbound".to_owned(),
                ));
            }
        } else if !is_valid_target(required_target) {
            return Err(HMetaError::Core(format!(
                "Global proxy target is unavailable or not a proxy: {required_target}"
            )));
        }
        required_target.to_owned()
    } else if let Some(current) = current.as_deref().filter(|target| is_valid_target(target)) {
        current.to_owned()
    } else if members.iter().any(|member| member == "DIRECT") {
        // Community (meow/mihomo) semantics: an unselected GLOBAL falls
        // back to its built-in DIRECT outbound when the subscription
        // exposes no concrete proxy nodes.
        "DIRECT".to_owned()
    } else {
        members
            .iter()
            .find(|target| is_valid_target(target))
            .cloned()
            .ok_or_else(|| {
                HMetaError::Core("Global mode requires at least one proxy node".to_owned())
            })?
    };

    if current.as_deref() != Some(target.as_str()) {
        global
            .selection()
            .ok_or_else(|| HMetaError::Core("GLOBAL outbound is not selectable".to_owned()))?
            .force_set(Some(&target));
    }
    if !global
        .current()
        .as_deref()
        .is_some_and(|current| current == target)
    {
        return Err(HMetaError::Core(
            "GLOBAL selector did not resolve to a proxy node".to_owned(),
        ));
    }
    Ok(Some(target))
}

pub(super) fn apply_global_proxy_policy(
    state: &mut CoreState,
    required_target: Option<&str>,
    persist_profile_selection: bool,
) -> Result<Option<String>, HMetaError> {
    let Some(tunnel) = state.tunnel.clone() else {
        if state.profiles.active_profile().is_none() {
            return Err(HMetaError::Core(
                "Global mode requires an active profile with at least one proxy node".to_owned(),
            ));
        }
        return Ok(required_target.map(ToOwned::to_owned));
    };
    let global_proxy = ensure_global_proxy_selected(&tunnel, required_target)?;
    if persist_profile_selection {
        if let (Some(profile_id), Some(global_proxy)) = (
            state.profiles.active_profile().map(ToOwned::to_owned),
            global_proxy.as_ref(),
        ) {
            state.profiles.set_selected_proxy(
                &profile_id,
                "GLOBAL".to_owned(),
                global_proxy.clone(),
            )?;
        }
    }
    refresh_proxy_groups_preserving_order(state, &tunnel);
    Ok(global_proxy)
}

pub(super) fn mode_to_tunnel(value: RuntimeMode) -> TunnelMode {
    match value {
        RuntimeMode::Rule => TunnelMode::Rule,
        RuntimeMode::Global => TunnelMode::Global,
        RuntimeMode::Direct => TunnelMode::Direct,
    }
}

pub(super) fn mode_from_tunnel(value: TunnelMode) -> RuntimeMode {
    match value {
        TunnelMode::Rule => RuntimeMode::Rule,
        TunnelMode::Global => RuntimeMode::Global,
        TunnelMode::Direct => RuntimeMode::Direct,
    }
}
