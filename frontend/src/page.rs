//! The "pages" abstraction: the organizer is a page switcher, and everything in
//! the rest of the app renders the currently selected page. A query page is the
//! first (and currently only) page type; settings/playlist/artist pages will
//! follow the same shape.

use std::sync::{Arc, Mutex};

use crate::QueryState;
use crate::rpc::Query;

/// A single query page: an editable `live` query plus the `saved` snapshot it was
/// last persisted from, and a cache of its results.
pub(crate) struct QueryPage {
    /// The editable working copy. Edits to the definition update this.
    pub(crate) live: Query,
    /// The last-saved version, or `None` if the query has never been persisted.
    pub(crate) saved: Option<Query>,
    /// Cached query results, shared with the async fetch task.
    pub(crate) results: Arc<Mutex<QueryState>>,
    /// Whether a result fetch has been kicked off for the current results cache.
    pub(crate) results_fetched: bool,
}

impl QueryPage {
    /// Builds a page from a persisted query (its live and saved versions match).
    pub(crate) fn persisted(query: Query) -> Self {
        Self {
            live: query.clone(),
            saved: Some(query),
            results: Arc::new(Mutex::new(QueryState::default())),
            results_fetched: false,
        }
    }

    /// Builds a brand-new ephemeral page that has never been saved.
    pub(crate) fn ephemeral(query: Query) -> Self {
        Self {
            live: query,
            saved: None,
            results: Arc::new(Mutex::new(QueryState::default())),
            results_fetched: false,
        }
    }

    /// Derived: a page is unsaved whenever its live version differs from the
    /// last-saved version (a never-saved page is always unsaved).
    pub(crate) fn unsaved(&self) -> bool {
        self.saved.as_ref() != Some(&self.live)
    }

    /// Whether this query exists in the backend database.
    pub(crate) fn is_persisted(&self) -> bool {
        self.saved.is_some()
    }
}
