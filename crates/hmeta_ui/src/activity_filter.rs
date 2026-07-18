use hmeta_model::{ConnectionSummary, RequestSummary};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RequestStatusFilter {
    #[default]
    All,
    Active,
    Ended,
}

impl RequestStatusFilter {
    pub(crate) const ALL: [Self; 3] = [Self::All, Self::Active, Self::Ended];
}

pub(crate) fn matches_connection_query(connection: &ConnectionSummary, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    let query = query.to_ascii_lowercase();
    contains(&connection.host, &query)
        || contains(&connection.network, &query)
        || contains(&connection.rule, &query)
        || contains(&connection.rule_payload, &query)
        || contains(&connection.proxy, &query)
        || connection
            .chains
            .iter()
            .any(|chain| contains(chain, &query))
        || contains(&connection.started_at, &query)
}

pub(crate) fn matches_request_filter(
    request: &RequestSummary,
    status_filter: RequestStatusFilter,
    query: &str,
) -> bool {
    match status_filter {
        RequestStatusFilter::All => {}
        RequestStatusFilter::Active if !request.active => return false,
        RequestStatusFilter::Ended if request.active => return false,
        _ => {}
    }

    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    let query = query.to_ascii_lowercase();
    contains(&request.host, &query)
        || contains(&request.network, &query)
        || contains(&request.rule, &query)
        || contains(&request.proxy, &query)
        || contains(&request.updated_at, &query)
}

pub(crate) fn request_connection_query(request: &RequestSummary) -> String {
    request.host.trim().to_owned()
}

fn contains(value: &str, query: &str) -> bool {
    value.to_ascii_lowercase().contains(query)
}
