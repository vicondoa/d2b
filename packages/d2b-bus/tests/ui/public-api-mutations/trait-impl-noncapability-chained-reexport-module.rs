mod deep {
    pub(super) mod ordinary {
        pub(crate) struct Request;
    }
}

mod reexports {
    pub(super) use super::deep as first;
}

use reexports::first as second;

impl Default for second::ordinary::Request {
    fn default() -> Self {
        deep::ordinary::Request
    }
}
