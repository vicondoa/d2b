mod ordinary {
    pub(super) struct Request;
}

use ordinary as wire;

impl Default for wire::Request {
    fn default() -> Self {
        ordinary::Request
    }
}
