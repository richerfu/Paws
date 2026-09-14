//! Distinguish duplicate immutable log records without using a hash as identity.
use std::collections::HashMap;
use std::hash::Hash;

pub(crate) fn occurrence_ids<T: Clone + Eq + Hash>(
    items: impl IntoIterator<Item = T>,
) -> Vec<(T, usize)> {
    let mut counts = HashMap::new();
    items
        .into_iter()
        .map(|item| {
            let count = counts.entry(item.clone()).or_insert(0);
            let id = (item, *count);
            *count += 1;
            id
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_log_records_have_unique_ids() {
        assert_eq!(
            occurrence_ids(["same", "other", "same"]),
            vec![("same", 0), ("other", 0), ("same", 1)]
        );
    }

    #[test]
    fn unrelated_insert_does_not_change_existing_record_identity() {
        let before = occurrence_ids(["a", "b"]);
        let after = occurrence_ids(["new", "a", "b"]);
        assert_eq!(before, after[1..]);
    }
}
