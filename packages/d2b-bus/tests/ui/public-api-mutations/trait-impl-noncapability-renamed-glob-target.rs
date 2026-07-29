mod deep {
    pub(super) struct Request;
}

use deep as facade;
use facade::*;

impl Default for Request {
    fn default() -> Self {
        deep::Request
    }
}
