use hmeta_model::ConnectionSummary;

pub(crate) fn latest_active_rule_node(connections: &[ConnectionSummary]) -> Option<String> {
    connections
        .iter()
        .filter_map(|connection| {
            connection
                .chains
                .last()
                .map(|node| (connection.started_at.as_str(), node.as_str()))
        })
        .max_by(|(left_started_at, _), (right_started_at, _)| left_started_at.cmp(right_started_at))
        .map(|(_, node)| node.to_owned())
}
