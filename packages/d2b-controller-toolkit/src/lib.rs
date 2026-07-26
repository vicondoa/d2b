//! Shared controller contract seams.

pub mod owner_hints;

pub use owner_hints::{
    MAX_OWNER_HINT_DEPTH, MAX_OWNER_HINT_WORK_ITEMS, OwnedResourceChangedHint, OwnerChangeEvent,
    OwnerHintCoalesceError, OwnerHintDispatch, OwnerHintDispatcher,
};
