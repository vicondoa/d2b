mod deep {
    pub(super) mod ordinary {
        pub(crate) struct Request;
    }
}

use deep::ordinary;

impl Default for ordinary::Request {
    fn default() -> Self {
        deep::ordinary::Request
    }
}
