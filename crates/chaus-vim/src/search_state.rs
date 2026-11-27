//! The last `/`/`?` search and the last `f`/`F`/`t`/`T` find.

use crate::motion::FindState;

#[derive(Default)]
pub(crate) struct SearchState {
    last_search: Option<(String, bool)>,
    last_find: Option<FindState>,
}

impl SearchState {
    pub(crate) fn last_search(&self) -> Option<&(String, bool)> {
        self.last_search.as_ref()
    }

    pub(crate) fn set_last_search(&mut self, search: Option<(String, bool)>) {
        self.last_search = search;
    }

    pub(crate) fn last_find(&self) -> Option<FindState> {
        self.last_find
    }

    pub(crate) fn set_last_find(&mut self, state: Option<FindState>) {
        self.last_find = state;
    }
}
