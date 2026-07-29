mod ordinary {
    pub(super) struct Request;
}

use ordinary::{self as wire};

impl Default for wire::Request {
    fn default() -> Self {
        ordinary::Request
    }
}
